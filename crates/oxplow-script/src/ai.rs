//! The `ai_*` Starlark builtins for **collectors** (P5.E2,
//! `.context/ai-providers.md` "`ai_*` functions for sources"):
//! `ai_classify`, `ai_score`, `ai_summarize` and `ai_extract`, answered by
//! the [`AiOracle`] of the run's [`AiHost`] (in `Evaluator::extra`, the
//! `TreeHost` pattern). The oracle is oxplow's recorded computations, so
//! the same question on the same text is one model call, ever. A gauge or
//! a report parser has no host and gets a refusal: a model call in a
//! gauge would make a metric neither cheap nor reproducible.
//!
//! The script runs on a worker thread; the oracle is synchronous (the app
//! blocks on its async computation there). The time a script waits on the
//! oracle doesn't count against its sandbox budget's `timeout`, though it
//! does count against its `ceiling`; once the sandbox gives up, the host
//! makes no more calls ([`AiHost::clock`]).

use std::sync::Arc;

use crate::runtime::RunClock;

use serde_json::Value;

/// What the `ai_*` builtins ask. Each returns JSON or an error message.
pub trait AiOracle: Send + Sync {
    /// `{ label, probabilities }`: which of `labels` fits `text`.
    fn classify(&self, text: &str, labels: &[String]) -> Result<Value, String>;
    /// `{ level, score, probabilities }`: where `text` sits on `levels`.
    fn score(&self, text: &str, levels: &[String]) -> Result<Value, String>;
    fn summarize(&self, text: &str, focus: Option<&str>) -> Result<String, String>;
    /// JSON matching `schema`, as `instructions` ask.
    fn extract(&self, instructions: &str, text: &str, schema: &Value) -> Result<Value, String>;
}

/// Per-run host for a collector's `ai_*` builtins.
#[derive(starlark::any::ProvidesStaticType)]
pub struct AiHost {
    oracle: Arc<dyn AiOracle>,
    /// Paused while the script waits on the oracle; stopped when the
    /// sandbox gives up.
    clock: Arc<RunClock>,
}

impl AiHost {
    pub fn new(oracle: Arc<dyn AiOracle>) -> Self {
        Self {
            oracle,
            clock: Arc::default(),
        }
    }

    /// The run's clock, shared with the sandbox: its budget leaves the
    /// oracle's time out, and it stops the clock when it gives up.
    pub fn clock(&self) -> Arc<RunClock> {
        self.clock.clone()
    }

    fn timed<T>(&self, f: impl FnOnce(&dyn AiOracle) -> Result<T, String>) -> Result<T, String> {
        if self.clock.stopped() {
            return Err("the run ran out of time; no more model calls".into());
        }
        self.clock.paused(|| f(&*self.oracle))
    }
}

fn host<'a>(
    eval: &starlark::eval::Evaluator<'_, 'a, '_>,
    builtin: &str,
) -> anyhow::Result<&'a AiHost> {
    eval.extra
        .and_then(|e| e.downcast_ref::<AiHost>())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{builtin} is available in entity collectors only: a fact collector or a \
                 report parser can't call a model"
            )
        })
}

fn strings(v: starlark::values::Value<'_>, what: &str) -> anyhow::Result<Vec<String>> {
    let json = v.to_json_value()?;
    serde_json::from_value(json).map_err(|_| anyhow::anyhow!("{what} must be a list of strings"))
}

#[starlark::starlark_module]
pub(crate) fn ai_builtins(builder: &mut starlark::environment::GlobalsBuilder) {
    /// `{label, probabilities}`: which of `labels` fits `text`.
    fn ai_classify<'v>(
        text: &str,
        labels: starlark::values::Value<'v>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let labels = strings(labels, "labels")?;
        let out = host(eval, "ai_classify")?
            .timed(|o| o.classify(text, &labels))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }

    /// `{level, score, probabilities}`: where `text` sits on `levels`
    /// (lowest first).
    fn ai_score<'v>(
        text: &str,
        levels: starlark::values::Value<'v>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let levels = strings(levels, "levels")?;
        let out = host(eval, "ai_score")?
            .timed(|o| o.score(text, &levels))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }

    /// A summary of `text`, focused on `focus`.
    fn ai_summarize<'v>(
        text: &str,
        #[starlark(default = starlark::values::none::NoneOr::None)]
        focus: starlark::values::none::NoneOr<&str>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let out = host(eval, "ai_summarize")?
            .timed(|o| o.summarize(text, focus.into_option()))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }

    /// JSON matching `schema` (a dict, JSON Schema) from `text`, as
    /// `instructions` ask.
    fn ai_extract<'v>(
        text: &str,
        schema: starlark::values::Value<'v>,
        #[starlark(default = "")] instructions: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let schema = schema.to_json_value()?;
        let out = host(eval, "ai_extract")?
            .timed(|o| o.extract(instructions, text, &schema))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{
        run_sandboxed_excluding, run_starlark, run_starlark_with_ai, run_starlark_with_host,
        SandboxBudget,
    };
    use crate::TreeHost;
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Duration;

    /// Answers from a script, counting what it was asked; `delay` stands
    /// in for a slow model.
    #[derive(Default)]
    struct Scripted {
        asked: Mutex<Vec<String>>,
        delay: Duration,
    }

    impl AiOracle for Scripted {
        fn classify(&self, text: &str, labels: &[String]) -> Result<Value, String> {
            std::thread::sleep(self.delay);
            self.asked.lock().unwrap().push(format!("classify {text}"));
            Ok(json!({ "label": labels[0], "probabilities": { labels[0].clone(): 1.0 } }))
        }
        fn score(&self, text: &str, levels: &[String]) -> Result<Value, String> {
            self.asked.lock().unwrap().push(format!("score {text}"));
            Ok(json!({ "level": levels[levels.len() - 1], "score": 1.0, "probabilities": {} }))
        }
        fn summarize(&self, text: &str, focus: Option<&str>) -> Result<String, String> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("summarize {text} {focus:?}"));
            Ok("short".into())
        }
        fn extract(&self, instructions: &str, text: &str, schema: &Value) -> Result<Value, String> {
            self.asked
                .lock()
                .unwrap()
                .push(format!("extract {instructions} {text} {}", schema["type"]));
            Ok(json!({ "n": 3 }))
        }
    }

    const SCRIPT: &str = r#"
def transform(input):
    c = ai_classify("fix the login bug", ["bug", "feature"])
    s = ai_score("big change", ["small", "large"])
    return {
        "label": c["label"],
        "level": s["level"],
        "summary": ai_summarize("a long text", focus = "risk"),
        "plain": ai_summarize("other text"),
        "n": ai_extract("one two three", {"type": "object"}, instructions = "count")["n"],
    }
"#;

    #[test]
    fn a_collector_asks_its_oracle() {
        let oracle = Arc::new(Scripted::default());
        let host = AiHost::new(oracle.clone());
        let out = run_starlark_with_ai(SCRIPT, &json!({}), &host).unwrap();
        assert_eq!(
            out,
            json!({ "label": "bug", "level": "large", "summary": "short", "plain": "short", "n": 3 })
        );
        assert_eq!(
            *oracle.asked.lock().unwrap(),
            vec![
                "classify fix the login bug",
                "score big change",
                "summarize a long text Some(\"risk\")",
                "summarize other text None",
                "extract count one two three \"object\"",
            ]
        );
    }

    /// P5.E2's red (the refusal half): no host — a gauge, a parser — and
    /// the builtin says where it works.
    #[test]
    fn a_fact_collector_or_a_parser_cant_call_a_model() {
        for run in [
            run_starlark(SCRIPT, &json!({})),
            run_starlark_with_host(SCRIPT, &json!({}), &TreeHost::default()),
        ] {
            let err = run.unwrap_err().to_string();
            assert!(
                err.contains("ai_classify is available in entity collectors only"),
                "{err}"
            );
        }
    }

    #[test]
    fn oracle_time_is_left_out_of_the_budget() {
        let oracle = Arc::new(Scripted {
            delay: Duration::from_millis(300),
            ..Scripted::default()
        });
        let host = Arc::new(AiHost::new(oracle));
        let clock = host.clock();
        let budget = SandboxBudget::with_timeout(Duration::from_millis(150));
        let script = "def transform(input):\n    return ai_classify('t', ['a'])\n";
        let h = host.clone();
        let out = run_sandboxed_excluding(&budget, &clock, move || {
            run_starlark_with_ai(script, &json!({}), &h)
        })
        .unwrap();
        assert_eq!(out["label"], "a");
        assert!(clock.total() >= Duration::from_millis(300));
    }

    /// Oracle time is left out of the budget, but not out of the ceiling:
    /// a per-row loop of model calls stops at the ceiling, and once the
    /// sandbox has given up, the detached worker makes no more calls.
    #[test]
    fn a_run_past_its_ceiling_stops_calling_the_oracle() {
        let oracle = Arc::new(Scripted {
            delay: Duration::from_millis(50),
            ..Scripted::default()
        });
        let host = Arc::new(AiHost::new(oracle.clone()));
        let clock = host.clock();
        let budget = SandboxBudget::with_timeout(Duration::from_secs(5))
            .with_ceiling(Duration::from_millis(300));
        let script = "def transform(input):\n    for i in range(100):\n        ai_classify(str(i), ['a'])\n    return {}\n";
        let h = host.clone();
        let started = std::time::Instant::now();
        let err = run_sandboxed_excluding(&budget, &clock, move || {
            run_starlark_with_ai(script, &json!({}), &h)
        })
        .unwrap_err();
        assert!(matches!(err, crate::CollectError::Timeout), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(2));
        let at_timeout = oracle.asked.lock().unwrap().len();
        std::thread::sleep(Duration::from_millis(400));
        let later = oracle.asked.lock().unwrap().len();
        assert!(later <= at_timeout + 1, "{at_timeout} then {later}");
        assert!(later < 100);
    }
}
