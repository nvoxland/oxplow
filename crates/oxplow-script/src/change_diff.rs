//! The `change_diff(change_id, paths=None)` Starlark builtin, part of the
//! `tree.read` service: a script asks for the changed lines of an
//! analysed change, `{files: [{path, status, hunks: [{head_start,
//! head_end, base_start, base_end, head_after, added: [{line, text}],
//! removed: [{line, text}]}]}], rest: [path], too_large: [path]}`: the
//! files past a call's limits are `rest`, to ask for again.
//!
//! The change's two sides live with the host (the app reads them through
//! its trees), so the script, which runs in a worker, asks across the
//! line like an `ai_*` call: a [`ChangeDiffSource`] answers on the host's
//! thread, and the time spent waiting there doesn't count against the
//! run's `timeout`.

use std::sync::Arc;

use serde_json::Value;

/// What `change_diff` asks. Each call returns the JSON the builtin
/// returns, or a message naming why it can't (an unknown change, a limit).
pub trait ChangeDiffSource: Send + Sync {
    fn change_diff(&self, change_id: i64, paths: Option<Vec<String>>) -> Result<Value, String>;
}

/// The diffs a run asks (`tree.read`): what `change_diff` answers from.
pub struct ChangeDiffHost {
    source: Arc<dyn ChangeDiffSource>,
}

impl ChangeDiffHost {
    pub fn new(source: Arc<dyn ChangeDiffSource>) -> Self {
        Self { source }
    }
}

#[starlark::starlark_module]
pub(crate) fn change_diff_builtins(builder: &mut starlark::environment::GlobalsBuilder) {
    /// The changed files of change `change_id` (`v_change.id`) with each
    /// one's hunks and their lines (`files`), the paths past this call's
    /// limits (`rest`, to ask again) and those too large for any call
    /// (`too_large`); `paths` limits it to those files.
    fn change_diff<'v>(
        #[starlark(require = pos)] change_id: i32,
        #[starlark(default = starlark::values::none::NoneOr::None)]
        paths: starlark::values::none::NoneOr<
            starlark::values::list_or_tuple::UnpackListOrTuple<String>,
        >,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let host = crate::runtime::Hosts::get(eval, "change_diff", |h| h.change_diff.as_ref())?;
        let paths = paths.into_option().map(|p| p.items);
        let out = host
            .source
            .change_diff(i64::from(change_id), paths)
            .map_err(anyhow::Error::msg)?;
        Ok(eval.heap().alloc(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{run_starlark, run_starlark_hosted, Hosts};
    use serde_json::json;

    struct Fixed;
    impl ChangeDiffSource for Fixed {
        fn change_diff(&self, id: i64, paths: Option<Vec<String>>) -> Result<Value, String> {
            if id == 0 {
                return Err("change 0 has no analysis".into());
            }
            Ok(json!({ "id": id, "paths": paths }))
        }
    }

    const SCRIPT: &str =
        "def transform(x):\n    return [change_diff(7), change_diff(7, paths = [\"a.rs\"])]\n";

    #[test]
    fn a_script_asks_its_source_and_a_refusal_is_its_error() {
        let hosts = Hosts {
            change_diff: Some(ChangeDiffHost::new(Arc::new(Fixed))),
            ..Hosts::default()
        };
        assert_eq!(
            run_starlark_hosted(SCRIPT, &json!({}), &hosts).unwrap(),
            json!([{ "id": 7, "paths": null }, { "id": 7, "paths": ["a.rs"] }])
        );
        let err = run_starlark_hosted(
            "def transform(x):\n    return change_diff(0)\n",
            &json!({}),
            &hosts,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("change 0 has no analysis"), "{err}");
    }

    /// A run not given the service is told the need.
    #[test]
    fn a_run_without_the_service_is_told_its_need() {
        let err = run_starlark(SCRIPT, &json!({})).unwrap_err().to_string();
        assert!(
            err.contains("`change_diff()` needs `tree.read` in this script's `needs`"),
            "{err}"
        );
    }
}
