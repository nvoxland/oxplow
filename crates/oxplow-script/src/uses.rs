//! What a Starlark script uses of oxplow's host services
//! (`oxplow_domain::service`), read from its syntax: every service builtin
//! it names — a call, an alias, inside a def or a lambda. Starlark reaches
//! a global only by its name, so this is every way in; a script that
//! doesn't parse runs nothing and uses nothing.

use std::collections::BTreeSet;

/// The service builtins `script` names. A name the script binds itself —
/// a parameter, an assignment or loop target, a def, a comprehension
/// variable — is the script's own value, not the service: Starlark makes a
/// name assigned anywhere in a def local to the whole def, and a module-level
/// binding shadows the builtin everywhere.
pub fn builtins_used(script: &str) -> BTreeSet<&'static str> {
    use starlark::syntax::ast::{AstExpr, AstStmt, ClauseP, ExprP, ForClauseP, ParameterP, StmtP};
    use starlark::syntax::{AstModule, Dialect};

    type Names = BTreeSet<String>;

    fn bind_params<'a>(
        params: impl Iterator<Item = &'a starlark::syntax::ast::AstParameter>,
        into: &mut Names,
    ) {
        for p in params {
            if let Some(id) = p.node.ident() {
                into.insert(id.node.ident.clone());
            }
        }
    }

    /// Every name `stmt` binds in its own scope: nested defs bind only their
    /// own name here, their bodies are scopes of their own.
    fn assigned(stmt: &AstStmt, into: &mut Names) {
        match &stmt.node {
            StmtP::Assign(a) => a.lhs.visit_lvalue(|id| {
                into.insert(id.node.ident.clone());
            }),
            StmtP::AssignModify(target, _, _) => target.visit_lvalue(|id| {
                into.insert(id.node.ident.clone());
            }),
            StmtP::For(f) => f.var.visit_lvalue(|id| {
                into.insert(id.node.ident.clone());
            }),
            StmtP::Def(d) => {
                into.insert(d.name.node.ident.clone());
                return;
            }
            StmtP::Load(l) => {
                for arg in &l.args {
                    into.insert(arg.local.node.ident.clone());
                }
            }
            _ => {}
        }
        stmt.node.visit_stmt(|child| assigned(child, into));
    }

    fn for_clause_names(
        clause: &ForClauseP<starlark::syntax::ast::AstNoPayload>,
        into: &mut Names,
    ) {
        clause.var.visit_lvalue(|id| {
            into.insert(id.node.ident.clone());
        });
    }

    fn visit_expr(e: &AstExpr, shadow: &Names, out: &mut BTreeSet<&'static str>) {
        match &e.node {
            ExprP::Identifier(id) => {
                let name = &id.node.ident;
                if !shadow.contains(name) {
                    if let Some(service) = oxplow_domain::service::service_of_builtin(name) {
                        if let Some(b) = service.builtins.iter().find(|b| **b == name) {
                            out.insert(b);
                        }
                    }
                }
            }
            ExprP::Lambda(l) => {
                let mut inner = shadow.clone();
                bind_params(l.params.iter(), &mut inner);
                visit_params(&l.params, shadow, out);
                visit_expr(&l.body, &inner, out);
                return;
            }
            ExprP::ListComprehension(_, first, rest) | ExprP::DictComprehension(_, first, rest) => {
                let mut inner = shadow.clone();
                for_clause_names(first, &mut inner);
                for clause in rest {
                    if let ClauseP::For(f) = clause {
                        for_clause_names(f, &mut inner);
                    }
                }
                e.node.visit_expr(|child| visit_expr(child, &inner, out));
                return;
            }
            _ => {}
        }
        e.node.visit_expr(|child| visit_expr(child, shadow, out));
    }

    fn visit_params(
        params: &[starlark::syntax::ast::AstParameter],
        shadow: &Names,
        out: &mut BTreeSet<&'static str>,
    ) {
        for p in params {
            if let ParameterP::Normal(_, _, Some(default)) = &p.node {
                visit_expr(default, shadow, out);
            }
        }
    }

    fn visit_stmt(stmt: &AstStmt, shadow: &Names, out: &mut BTreeSet<&'static str>) {
        if let StmtP::Def(d) = &stmt.node {
            visit_params(&d.params, shadow, out);
            let mut inner = shadow.clone();
            bind_params(d.params.iter(), &mut inner);
            assigned(&d.body, &mut inner);
            visit_stmt(&d.body, &inner, out);
            return;
        }
        let mut each = |e: &AstExpr| visit_expr(e, shadow, out);
        match &stmt.node {
            StmtP::If(cond, _) => each(cond),
            StmtP::IfElse(cond, _) => each(cond),
            StmtP::For(f) => {
                f.var.visit_expr(&mut each);
                each(&f.over);
            }
            StmtP::Return(Some(e)) | StmtP::Expression(e) => each(e),
            StmtP::Assign(a) => {
                a.lhs.visit_expr(&mut each);
                each(&a.rhs);
            }
            StmtP::AssignModify(target, _, rhs) => {
                target.visit_expr(&mut each);
                each(rhs);
            }
            _ => {}
        }
        stmt.node.visit_stmt(|child| visit_stmt(child, shadow, out));
    }

    let mut out = BTreeSet::new();
    let Ok(ast) = AstModule::parse("script.star", script.to_string(), &Dialect::Standard) else {
        return out;
    };
    let mut module_names = Names::new();
    assigned(ast.statement(), &mut module_names);
    visit_stmt(ast.statement(), &module_names, &mut out);
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

    /// A name the script binds itself is its own value, not the builtin;
    /// Starlark scopes an assignment to the whole enclosing def.
    #[test]
    fn a_locally_bound_name_is_not_a_use_of_the_builtin() {
        let script = r#"
files = 1
def helper(scope, *args, **ai_classify):
    return scope

def transform(input):
    sql_rows = [1]
    for tree_rows in input:
        pass
    (a, scope) = (1, 2)
    ai_summarize += 1
    def ai_decide():
        return 1
    ys = [x for files in input for x in files]
    g = lambda ai_classify: ai_classify
    return capped(files)

def late(input):
    use = scope
    scope = 3
    return use
"#;
        assert!(
            builtins_used(script).is_empty(),
            "{:?}",
            builtins_used(script)
        );
        // Another def's binding doesn't hide the builtin here.
        assert_eq!(
            builtins_used("def a():\n    files = 1\ndef b():\n    return files\n"),
            ["files"].into()
        );
    }
}
