//! The `scope(id, args)` Starlark builtin for **command handlers**
//! (`.context/commands.md` "Scopes"): a script asks its host
//! to do something — read the semantic layer (`sql.read`), and in time
//! write oxplow's records, run git, open a page — by the scope's id.
//!
//! The script runs on a worker thread (the sandbox); the host answers on
//! the caller's thread, which owns what the answer needs (the command's
//! transaction is not `Send`). [`run_starlark_serving`] runs the script
//! and serves each call while it waits: the call crosses to the caller
//! over a channel, and the answer crosses back. The time spent answering
//! doesn't count against the budget's `timeout` (it does against its
//! `ceiling`); once the sandbox gives up, the detached worker's calls are
//! refused.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::runtime::{run_starlark_inner, Host, RunClock, SandboxBudget};
use crate::CollectError;

/// Answers a script's `scope(id, args)` call: the answer, or why not.
pub type Serve<'a> = dyn FnMut(&str, Value) -> Result<Value, String> + 'a;

enum Msg {
    Call {
        id: String,
        args: Value,
        reply: mpsc::Sender<Result<Value, String>>,
    },
    Done(Result<Value, CollectError>),
}

/// The worker's side: where its `scope` calls go.
#[derive(starlark::any::ProvidesStaticType)]
pub struct ScopeHost {
    calls: mpsc::Sender<Msg>,
}

impl ScopeHost {
    fn call(&self, id: &str, args: Value) -> Result<Value, String> {
        let (reply, answer) = mpsc::channel();
        let gone = || "the run ran out of time; no more scope calls".to_string();
        self.calls
            .send(Msg::Call {
                id: id.to_string(),
                args,
                reply,
            })
            .map_err(|_| gone())?;
        answer.recv().map_err(|_| gone())?
    }
}

/// Run a Starlark command handler (`def transform(x)`) over `input` in
/// the sandbox, answering its `scope(id, args)` calls with `serve`
/// on this thread.
pub fn run_starlark_serving(
    budget: &SandboxBudget,
    script: &str,
    input: &Value,
    serve: &mut Serve<'_>,
) -> Result<Value, CollectError> {
    let (tx, rx) = mpsc::channel();
    let host = ScopeHost { calls: tx.clone() };
    let (script, input) = (script.to_string(), input.clone());
    std::thread::spawn(move || {
        let out = run_starlark_inner(&script, &input, Host::Scope(&host));
        let _ = tx.send(Msg::Done(out));
    });
    let clock = RunClock::default();
    let started = Instant::now();
    loop {
        let deadline = (started + budget.timeout + clock.total()).min(started + budget.ceiling);
        let now = Instant::now();
        if now >= deadline {
            clock.stop();
            return Err(CollectError::Timeout);
        }
        match rx.recv_timeout((deadline - now).min(Duration::from_millis(50))) {
            Ok(Msg::Done(out)) => return out,
            Ok(Msg::Call { id, args, reply }) => {
                let answer = clock.paused(|| serve(&id, args));
                let _ = reply.send(answer);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(CollectError::Runtime("the script's worker died".into()))
            }
        }
    }
}

#[starlark::starlark_module]
pub(crate) fn scope_builtins(builder: &mut starlark::environment::GlobalsBuilder) {
    /// What the scope `id` answers for `args` (a dict): only in a
    /// command's handler, and only a scope the command `needs`.
    fn scope<'v>(
        id: &str,
        #[starlark(default = starlark::values::none::NoneOr::None)]
        args: starlark::values::none::NoneOr<starlark::values::Value<'v>>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let args = match args.into_option() {
            Some(v) => v.to_json_value()?,
            None => Value::Object(Default::default()),
        };
        let host = eval
            .extra
            .and_then(|e| e.downcast_ref::<ScopeHost>())
            .ok_or_else(|| anyhow::anyhow!("scope() is available in command handlers only"))?;
        let out = host.call(id, args).map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::run_starlark;
    use serde_json::json;

    const READS: &str = r#"
def transform(x):
    rows = scope("sql.read", {"sql": "SELECT 1", "params": {"ref": x["ref"]}})
    return {"rows": rows, "again": scope("sql.read", {"sql": "SELECT 2"})}
"#;

    #[test]
    fn a_handler_calls_scopes_its_host_answers() {
        let mut asked = Vec::new();
        let out = run_starlark_serving(
            &SandboxBudget::with_timeout(Duration::from_secs(5)),
            READS,
            &json!({ "ref": "effort:eff1" }),
            &mut |id, args| {
                asked.push(format!("{id} {args}"));
                Ok(json!([{ "n": asked.len() }]))
            },
        )
        .unwrap();
        assert_eq!(out, json!({ "rows": [{ "n": 1 }], "again": [{ "n": 2 }] }));
        assert_eq!(
            asked,
            vec![
                r#"sql.read {"sql":"SELECT 1","params":{"ref":"effort:eff1"}}"#,
                r#"sql.read {"sql":"SELECT 2"}"#,
            ]
        );
    }

    #[test]
    fn a_refused_call_fails_the_script_with_the_reason() {
        let err = run_starlark_serving(
            &SandboxBudget::with_timeout(Duration::from_secs(5)),
            READS,
            &json!({ "ref": "x" }),
            &mut |id, _| Err(format!("`{id}` isn't in the command's needs")),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("`sql.read` isn't in the command's needs"),
            "{err}"
        );
    }

    #[test]
    fn only_a_command_handler_has_scopes() {
        let err = run_starlark(READS, &json!({ "ref": "x" }))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("scope() is available in command handlers only"),
            "{err}"
        );
    }

    /// The host's time answering is left out of the budget, but a script
    /// that runs on past it is given up on.
    #[test]
    fn answering_is_left_out_of_the_budget() {
        let budget = SandboxBudget::with_timeout(Duration::from_millis(150));
        let out = run_starlark_serving(
            &budget,
            "def transform(x):\n    return scope('sql.read')\n",
            &json!({}),
            &mut |_, _| {
                std::thread::sleep(Duration::from_millis(300));
                Ok(json!(7))
            },
        )
        .unwrap();
        assert_eq!(out, json!(7));
        let err = run_starlark_serving(
            &budget,
            "def transform(x):\n    for i in range(100000000):\n        pass\n    return 1\n",
            &json!({}),
            &mut |_, _| Ok(json!(0)),
        )
        .unwrap_err();
        assert!(matches!(err, CollectError::Timeout), "{err:?}");
    }
}
