//! The `scope(id, args)` Starlark builtin for **command handlers**
//! (`.context/commands.md` "Scopes"): a script asks its host
//! to do something — read the semantic layer (`sql.read`), and in time
//! write oxplow's records, run git, open a page — by the scope's id.
//!
//! The script runs in a script worker ([`crate::workers`]); each call
//! crosses to the host, which answers on the caller's thread (it owns what
//! the answer needs: the command's transaction is not `Send`).

use serde_json::Value;

/// How a script's `scope(id, args)` call is answered.
pub(crate) type ScopeCall = Box<dyn Fn(&str, Value) -> Result<Value, String>>;

/// The script's side: where its `scope` calls go.
#[derive(starlark::any::ProvidesStaticType)]
pub struct ScopeHost {
    call: ScopeCall,
}

impl ScopeHost {
    pub(crate) fn new(call: ScopeCall) -> Self {
        Self { call }
    }

    fn call(&self, id: &str, args: Value) -> Result<Value, String> {
        (self.call)(id, args)
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
    use crate::runtime::run_starlark;
    use serde_json::json;

    /// `scope()` is a command handler's: a script run without a scope
    /// host can't reach one.
    #[test]
    fn only_a_command_handler_has_scopes() {
        let err = run_starlark(
            "def transform(x):\n    return scope('sql.read', {'sql': 'SELECT 1'})\n",
            &json!({}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("scope() is available in command handlers only"),
            "{err}"
        );
    }
}
