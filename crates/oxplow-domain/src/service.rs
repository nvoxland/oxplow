//! The host services a script may use (`.context/extensions.md`
//! "Services"): each one a scope a script declares in its `needs`, the
//! builtins it provides, whether a person approves it, and which runs may
//! use it. One table: the router that builds a run's hosts, `oxplow
//! extension check`, the reference and `list_services` all read it.

use serde::Serialize;

use crate::run::Mode;

/// One service a script may use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ServiceSpec {
    /// The scope it is (`crate::scope::SCOPES`), what `needs` names.
    pub scope: &'static str,
    pub title: &'static str,
    /// What it gives a script, for a person and an author.
    pub summary: &'static str,
    /// The Starlark builtins it provides (`scope` for `sql.read`).
    pub builtins: &'static [&'static str],
    /// A person approves a script that uses it (it spends their key).
    pub approval: bool,
    /// The runs that may use it: a pure run (a command's composer, an
    /// effect's or policy's script) may not call out.
    pub modes: &'static [Mode],
}

const ALL: &[Mode] = &[Mode::Pure, Mode::Transactional, Mode::External];

/// Services multiplexed by the `scope` builtin. The SQL service owns the
/// builtin's metadata; each call checks its actual declared service.
pub const CALLABLE: &[&str] = &[
    "sql.read",
    "state.read",
    "extension_config.read",
    "jobs.read",
    "ai.discover",
];

/// Every service, by scope.
pub const SERVICES: &[ServiceSpec] = &[
    ServiceSpec {
        scope: "jobs.read",
        title: "Read recorded job results",
        summary: "Read this extension's durable job status and result: scope(\"jobs.read\", {id}).",
        builtins: &[],
        approval: false,
        modes: ALL,
    },
    ServiceSpec {
        scope: "extension_config.read",
        title: "Read resolved extension configuration",
        summary: "Read the resolved defaults, project and personal settings with their provenance: \
                  `scope(\"extension_config.read\", {})`.",
        builtins: &[],
        approval: false,
        modes: ALL,
    },
    ServiceSpec {
        scope: "state.read",
        title: "Read private extension state",
        summary: "Read this extension's workflow state or cache: \
                  `scope(\"state.read\", {scope, area})` returns a snapshot or null.",
        builtins: &[],
        approval: false,
        modes: ALL,
    },
    ServiceSpec {
        scope: "sql.read",
        title: "Read the semantic layer",
        summary: "Read-only SQL over the published models (`v_*`): \
                  `scope(\"sql.read\", {sql, params?, page?})` returns every row, or one page.",
        builtins: &["scope"],
        approval: false,
        modes: ALL,
    },
    ServiceSpec {
        scope: "tree.read",
        title: "Read the captured tree",
        summary: "The files of the snapshot the run is anchored to, with their syntax, \
                  metrics and duplicate blocks.",
        builtins: &["files", "source_files", "duplicate_blocks"],
        approval: false,
        modes: ALL,
    },
    ServiceSpec {
        scope: "ai.compute",
        title: "Ask a model",
        summary: "Model calls on your AI provider: classify, score, summarize, extract, typed completion. \
                  Each answer is recorded, so the same question on the same text costs \
                  one call.",
        builtins: &[
            "ai_classify",
            "ai_score",
            "ai_summarize",
            "ai_extract",
            "ai_complete",
        ],
        approval: true,
        modes: &[Mode::External],
    },
    ServiceSpec {
        scope: "ai.discover",
        title: "Discover model capabilities",
        summary: "Declared model identities, features and limits; no credentials or paid probes.",
        builtins: &["ai_capabilities", "ai_models"],
        approval: false,
        modes: ALL,
    },
];

/// The service `scope`, if it is one.
pub fn service(scope: &str) -> Option<&'static ServiceSpec> {
    SERVICES.iter().find(|s| s.scope == need_scope(scope))
}

/// A bounded read declaration, or an ordinary service name. Qualifiers
/// are exact model names or relative file/directory paths, never SQL or globs.
pub fn need_scope(need: &str) -> &str {
    match need.split_once(':') {
        Some(("sql.read", model)) if valid_model(model) => "sql.read",
        Some(("tree.read", path)) if valid_path(path) => "tree.read",
        _ => need,
    }
}

pub fn declares(needs: &[String], scope: &str) -> bool {
    needs.iter().any(|need| need_scope(need) == scope)
}

fn valid_model(model: &str) -> bool {
    model.starts_with("v_")
        && model.len() > 2
        && model
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// Portable repo-relative paths. A trailing slash denotes a directory.
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', '\0', ':'])
        && path
            .trim_end_matches('/')
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

/// The service that provides builtin `name`, if one does.
pub fn service_of_builtin(name: &str) -> Option<&'static ServiceSpec> {
    SERVICES.iter().find(|s| s.builtins.contains(&name))
}

/// A script's need `id` (its manifest's `needs:`): a service, or why not —
/// what a collector's, command's, effect's and policy's `needs` meet at
/// load.
pub fn check_declared(id: &str) -> Result<(), String> {
    if service(id).is_some() {
        return Ok(());
    }
    let what = if crate::scope::scope(id).is_some() {
        "a scope a script can't use"
    } else {
        "not a scope"
    };
    Err(format!(
        "`needs`: `{id}` is {what} — a script uses {}",
        SERVICES
            .iter()
            .map(|s| format!("`{}`", s.scope))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// What a script is told when it calls `builtin` without the service
/// declared: the one wording every host gives.
pub fn undeclared(builtin: &str) -> String {
    match service_of_builtin(builtin) {
        Some(s) => format!("`{builtin}()` needs `{}` in this script's `needs`", s.scope),
        None => format!("`{builtin}()` isn't a service builtin"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every service is a registered scope, and a builtin belongs to one.
    #[test]
    fn every_service_is_a_scope_and_each_builtin_one_services() {
        for s in SERVICES {
            assert!(crate::scope::scope(s.scope).is_some(), "{}", s.scope);
        }
        let mut builtins: Vec<&str> = SERVICES
            .iter()
            .flat_map(|s| s.builtins.iter().copied())
            .collect();
        let n = builtins.len();
        builtins.sort();
        builtins.dedup();
        assert_eq!(builtins.len(), n, "a builtin in two services");
        assert_eq!(service_of_builtin("files").unwrap().scope, "tree.read");
        assert_eq!(
            service_of_builtin("ai_extract").unwrap().scope,
            "ai.compute"
        );
        assert!(
            service_of_builtin("ast_query").is_none(),
            "pure: no service"
        );
    }

    /// Calling a model is the one service a pure run may not use, and
    /// the one a person approves.
    #[test]
    fn only_a_run_that_may_call_out_asks_a_model() {
        let ai = service("ai.compute").unwrap();
        assert!(ai.approval);
        assert_eq!(ai.modes, &[Mode::External]);
        assert!(service("tree.read").unwrap().modes.contains(&Mode::Pure));
        assert_eq!(
            undeclared("files"),
            "`files()` needs `tree.read` in this script's `needs`"
        );
    }
}
