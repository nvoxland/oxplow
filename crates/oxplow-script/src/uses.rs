//! What a Starlark script uses of oxplow's host services
//! (`oxplow_domain::service`), read from its syntax: every service builtin
//! it names — a call, an alias, inside a def or a lambda. Starlark reaches
//! a global only by its name, so this is every way in; a script that
//! doesn't parse runs nothing and uses nothing.

use std::collections::BTreeSet;

/// The service builtins `script` names.
pub fn builtins_used(script: &str) -> BTreeSet<&'static str> {
    use starlark::syntax::ast::{AstExpr, ExprP};
    use starlark::syntax::{AstModule, Dialect};
    fn visit(e: &AstExpr, out: &mut BTreeSet<&'static str>) {
        if let ExprP::Identifier(id) = &e.node {
            if let Some(service) = oxplow_domain::service::service_of_builtin(&id.node.ident) {
                if let Some(b) = service.builtins.iter().find(|b| **b == id.node.ident) {
                    out.insert(b);
                }
            }
        }
        e.node.visit_expr(|child| visit(child, out));
    }
    let mut out = BTreeSet::new();
    let Ok(ast) = AstModule::parse("script.star", script.to_string(), &Dialect::Standard) else {
        return out;
    };
    ast.statement().visit_expr(|e| visit(e, &mut out));
    out
}

/// The services (their scopes) `script`'s builtins need.
pub fn services_used(script: &str) -> BTreeSet<&'static str> {
    builtins_used(script)
        .into_iter()
        .filter_map(oxplow_domain::service::service_of_builtin)
        .map(|s| s.scope)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every way a script reaches a service builtin counts; a name that
    /// only looks like one (a string, an attribute) doesn't.
    #[test]
    fn a_script_uses_the_services_its_builtins_name() {
        let script = r#"
def transform(input):
    f = files
    rows = scope("sql.read", {"sql": "SELECT 1"})
    label = (lambda t: ai_classify(t, ["a"]))("x")
    return {"note": "files ai_summarize", "x": input.files}
"#;
        assert_eq!(
            builtins_used(script),
            ["ai_classify", "files", "scope"].into()
        );
        assert_eq!(
            services_used(script),
            ["ai.compute", "sql.read", "tree.read"].into()
        );
        assert!(services_used("not starlark (").is_empty());
    }
}
