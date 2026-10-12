//! The `ai_*` Starlark builtins, the `ai.compute` service
//! (`.context/ai-providers.md` "`ai_*` functions for sources"):
//! `ai_classify`, `ai_score`, `ai_summarize` and `ai_extract`, answered by
//! the [`AiOracle`] of the run's [`AiHost`] (one of its
//! [`crate::runtime::Hosts`]). The oracle is oxplow's recorded
//! computations, so the same question on the same text is one model call,
//! ever. A run not given a model — one that didn't declare `ai.compute`,
//! or whose mode can't call out — is told the need it lacks.
//!
//! The script runs on a worker thread; the oracle is synchronous (the app
//! blocks on its async computation there). The time a script waits on the
//! oracle doesn't count against its sandbox budget's `timeout`, though it
//! does count against its `ceiling`; once the sandbox gives up, the host
//! makes no more calls ([`AiHost::clock`]).

use std::sync::Arc;

use serde_json::Value;

/// What the `ai_*` builtins ask. Each returns JSON or an error message.
pub trait AiOracle: Send + Sync {
    /// Local metadata only: no credentials, completion or usage charge.
    fn capabilities(&self, _role: &str) -> Result<Value, String> {
        Err("this host has no AI capability discovery".into())
    }
    fn models(&self, _role: &str) -> Result<Value, String> {
        Err("this host has no AI model discovery".into())
    }
    fn complete_typed(&self, _role: &str, _request: Value) -> Result<Value, String> {
        Err("this host has no typed AI completion".into())
    }
    /// `{ label, probabilities }`: which of `labels` fits `text`.
    fn classify(&self, text: &str, labels: &[String]) -> Result<Value, String>;
    /// `{ level, score, probabilities }`: where `text` sits on `levels`.
    fn score(&self, text: &str, levels: &[String]) -> Result<Value, String>;
    fn summarize(&self, text: &str, focus: Option<&str>) -> Result<String, String>;
    /// JSON matching `schema`, as `instructions` ask.
    fn extract(&self, instructions: &str, text: &str, schema: &Value) -> Result<Value, String>;
}

/// The model a run asks (`ai.compute`): what the `ai_*` builtins answer
/// from.
pub struct AiHost {
    oracle: Arc<dyn AiOracle>,
}

impl AiHost {
    pub fn new(oracle: Arc<dyn AiOracle>) -> Self {
        Self { oracle }
    }

    fn timed<T>(&self, f: impl FnOnce(&dyn AiOracle) -> Result<T, String>) -> Result<T, String> {
        f(&*self.oracle)
    }
}

/// The builtins that ask a model.
pub const AI_BUILTINS: &[&str] = &[
    "ai_classify",
    "ai_score",
    "ai_summarize",
    "ai_extract",
    "ai_complete",
];

/// Whether Starlark `script` names an `ai_*` builtin anywhere — a call,
/// an alias, inside a def or a lambda — and so can spend a model call
/// each run. Starlark reaches a global only by its name, so this is every
/// way in; a script that doesn't parse runs nothing.
pub fn calls_ai(script: &str) -> bool {
    crate::uses::services_used(script).contains("ai.compute")
}

fn host<'a>(
    eval: &starlark::eval::Evaluator<'_, 'a, '_>,
    builtin: &str,
) -> anyhow::Result<&'a AiHost> {
    crate::runtime::Hosts::get(eval, builtin, |h| h.ai.as_ref())
}

fn strings(v: starlark::values::Value<'_>, what: &str) -> anyhow::Result<Vec<String>> {
    let json = v.to_json_value()?;
    serde_json::from_value(json).map_err(|_| anyhow::anyhow!("{what} must be a list of strings"))
}

#[starlark::starlark_module]
pub(crate) fn ai_builtins(builder: &mut starlark::environment::GlobalsBuilder) {
    /// A V1 request with ordered messages and tools, using a configured role.
    fn ai_complete<'v>(
        request: starlark::values::Value<'v>,
        #[starlark(default = "main")] role: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let request = request.to_json_value()?;
        let out = host(eval, "ai_complete")?
            .timed(|o| o.complete_typed(role, request))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }

    fn ai_capabilities<'v>(
        #[starlark(default = "main")] role: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let out = host(eval, "ai_capabilities")?
            .timed(|o| o.capabilities(role))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }

    fn ai_models<'v>(
        #[starlark(default = "main")] role: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let out = host(eval, "ai_models")?
            .timed(|o| o.models(role))
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }

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
    use crate::runtime::{run_starlark, run_starlark_hosted, Hosts};
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
        let hosts = Hosts {
            ai: Some(AiHost::new(oracle.clone())),
            ..Hosts::default()
        };
        let out = run_starlark_hosted(SCRIPT, &json!({}), &hosts).unwrap();
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

    /// A script that names an `ai_*` builtin anywhere — a call, an alias,
    /// inside a def or a lambda — can spend a model call; a mention in a
    /// string or comment can't, and neither can a script that doesn't parse.
    #[test]
    fn a_script_calls_ai_when_it_names_an_ai_builtin() {
        for script in [
            "def transform(x):\n    return ai_summarize(\"t\")\n",
            "f = ai_classify\ndef transform(x):\n    return f(\"t\", [\"a\"])\n",
            "def transform(x):\n    g = lambda t: ai_score(t, [\"lo\", \"hi\"])\n    return [g(r) for r in x[\"rows\"]]\n",
            "def transform(x):\n    if x:\n        for r in x:\n            return {\"a\": ai_extract(r, {})}\n",
        ] {
            assert!(calls_ai(script), "{script}");
        }
        for script in [
            "def transform(x):\n    return {\"note\": \"ai_summarize\"}  # ai_classify\n",
            "def transform(x):\n    return summarize(x)\n",
            "def transform(x:\n",
        ] {
            assert!(!calls_ai(script), "{script}");
        }
    }

    /// A run not given a model — a gauge, a parser, a script that didn't
    /// declare `ai.compute` — is told the need, whatever else it has.
    #[test]
    fn a_run_not_given_a_model_is_told_its_need() {
        let tree = Hosts {
            tree: Some(TreeHost::default()),
            ..Hosts::default()
        };
        for run in [
            run_starlark(SCRIPT, &json!({})),
            run_starlark_hosted(SCRIPT, &json!({}), &tree),
        ] {
            let err = run.unwrap_err().to_string();
            assert!(
                err.contains("`ai_classify()` needs `ai.compute` in this script's `needs`"),
                "{err}"
            );
        }
    }
}
