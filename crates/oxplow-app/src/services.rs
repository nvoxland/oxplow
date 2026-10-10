//! The services a script's run is given (`.context/extensions.md`
//! "Services"): each service its script uses decided on its run — its mode,
//! what it declares, its authority — and the hosts built for those it may
//! use. Every host that runs an extension's script prepares it here:
//! collectors (entity and fact), commands, effects, effort policies. None
//! builds its own.

use std::collections::HashMap;
use std::sync::Arc;

use oxplow_domain::events::Anchors;
use oxplow_domain::grant::Decision;
use oxplow_domain::run::RunContext;
use oxplow_script::workers::{ScriptRun, Serve};

/// A captured tree: repo-relative path → its text.
pub type Tree = Arc<HashMap<String, String>>;

/// Where a run's tree is read from.
#[async_trait::async_trait]
pub trait TreeSource: Send + Sync {
    /// The tree of the snapshot `anchors` names, else of its stream's
    /// latest snapshot, else of the primary stream's.
    async fn tree(&self, anchors: &Anchors) -> Result<Tree, String>;
}

/// Where a caller's run gets its tree.
#[derive(Clone)]
pub enum Trees {
    /// The run's anchors say which, read when the script uses it.
    From(Arc<dyn TreeSource>),
    /// This one: a fact collector's sweep, or an example's fixture.
    Fixed(Tree),
}

/// What a caller can give its scripts: each service a script may be
/// allowed is built from these.
#[derive(Clone, Default)]
pub struct Sources {
    pub tree: Option<Trees>,
    /// A model, asked as the caller named here (`collector:<owner>/<id>`),
    /// each answer recorded on the run.
    pub ai: Option<(Arc<crate::ai_compute::AiCompute>, String)>,
}

impl Sources {
    pub fn with_tree(mut self, tree: Trees) -> Self {
        self.tree = Some(tree);
        self
    }

    pub fn with_ai(mut self, ai: Arc<crate::ai_compute::AiCompute>, caller: String) -> Self {
        self.ai = Some((ai, caller));
        self
    }
}

/// What a script's run was given.
#[derive(Default)]
pub struct Given {
    pub tree: Option<Tree>,
    pub ai: Option<Arc<dyn oxplow_script::AiOracle>>,
    /// Its `scope(…)` calls are served (`sql.read`), each decided as it
    /// is called against `needs` (`scope_calls::Calls`).
    pub scopes: bool,
    /// What the script declares, and its kind implies.
    pub needs: Vec<String>,
}

impl std::fmt::Debug for Given {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Given")
            .field("tree", &self.tree.as_ref().map(|t| t.len()))
            .field("ai", &self.ai.is_some())
            .field("scopes", &self.scopes)
            .field("needs", &self.needs)
            .finish()
    }
}

impl Given {
    /// `script_run` with these services, its scope calls answered by
    /// `serve` (a caller that serves none has none to give).
    pub fn onto<'a>(
        self,
        mut script_run: ScriptRun<'a>,
        serve: Option<&'a mut Serve<'a>>,
    ) -> ScriptRun<'a> {
        if let Some(tree) = self.tree {
            script_run = script_run.with_tree(tree);
        }
        if let Some(ai) = self.ai {
            script_run = script_run.with_ai(ai);
        }
        if let Some(serve) = serve.filter(|_| self.scopes) {
            script_run = script_run.with_scope(serve);
        }
        script_run
    }
}

/// The services a script may be given, decided ([`decide`]) and not yet
/// built.
#[derive(Debug, Default)]
pub struct Wanted {
    /// The builtin that wants the tree, a model.
    tree: Option<&'static str>,
    ai: Option<&'static str>,
    scopes: bool,
    needs: Vec<String>,
}

/// Decide each service Starlark `script` uses on `run`: refused when the
/// run's mode can't use it, when `needs` (what it declares, and what its
/// kind implies) doesn't name it, or when its authority denies it. A
/// `sql.read` call is decided as it's made (`scope_calls::Calls`); the
/// others here, recorded on the run either way.
pub fn decide(run: &RunContext, script: &str, needs: &[String]) -> Result<Wanted, String> {
    let mut wanted = Wanted {
        needs: needs.to_vec(),
        ..Wanted::default()
    };
    for builtin in oxplow_script::uses::builtins_used(script) {
        let service =
            oxplow_domain::service::service_of_builtin(builtin).expect("a service builtin");
        let scope = service.scope;
        let declared = needs.iter().any(|n| n == scope);
        if scope == "sql.read" {
            if !declared {
                return Err(oxplow_domain::service::undeclared(builtin));
            }
            wanted.scopes = true;
            continue;
        }
        let slot = match scope {
            "tree.read" => &mut wanted.tree,
            "ai.compute" => &mut wanted.ai,
            other => unreachable!("a service without a host: {other}"),
        };
        if slot.is_some() {
            continue;
        }
        let decision = run.decide_service(scope);
        if !service.modes.contains(&run.mode()) {
            let Decision::Denied(why) = decision else {
                unreachable!("a mode that can't use it is denied")
            };
            return Err(format!("`{builtin}()`: {why}"));
        }
        if !declared {
            return Err(oxplow_domain::service::undeclared(builtin));
        }
        if let Decision::Denied(why) = decision {
            return Err(format!("`{builtin}()` was refused `{scope}`: {why}"));
        }
        *slot = Some(builtin);
    }
    Ok(wanted)
}

impl Wanted {
    /// The hosts for what it may use, built from `sources` for `run`.
    pub async fn build(self, run: &RunContext, sources: &Sources) -> Result<Given, String> {
        let tree = match (self.tree, &sources.tree) {
            (None, _) => None,
            (Some(_), Some(Trees::Fixed(tree))) => Some(tree.clone()),
            (Some(_), Some(Trees::From(source))) => Some(source.tree(run.anchors()).await?),
            (Some(builtin), None) => return Err(unavailable(builtin, "tree.read")),
        };
        let ai: Option<Arc<dyn oxplow_script::AiOracle>> = match (self.ai, &sources.ai) {
            (None, _) => None,
            (Some(_), Some((compute, caller))) => {
                Some(Arc::new(crate::ai_compute::CollectorOracle::new(
                    compute.clone(),
                    caller.clone(),
                    run.clone(),
                )))
            }
            (Some(builtin), None) => return Err(unavailable(builtin, "ai.compute")),
        };
        Ok(Given {
            tree,
            ai,
            scopes: self.scopes,
            needs: self.needs,
        })
    }

    /// The hosts for a caller with no tree or model to give: its scopes.
    pub fn scopes_only(self) -> Result<Given, String> {
        if let Some(builtin) = self.tree {
            return Err(unavailable(builtin, "tree.read"));
        }
        if let Some(builtin) = self.ai {
            return Err(unavailable(builtin, "ai.compute"));
        }
        Ok(Given {
            scopes: self.scopes,
            needs: self.needs,
            ..Given::default()
        })
    }
}

/// [`decide`], then [`Wanted::build`].
pub async fn prepare(
    run: &RunContext,
    script: &str,
    needs: &[String],
    sources: &Sources,
) -> Result<Given, String> {
    decide(run, script, needs)?.build(run, sources).await
}

fn unavailable(builtin: &str, scope: &str) -> String {
    format!("`{builtin}()`: `{scope}` isn't available where this script runs")
}

/// A Starlark collector's needs: what it declares, and what its kind
/// implies — a fact collector reads the tree, an entity collector that
/// asks a model uses `ai.compute`.
pub fn collector_needs(
    spec: &oxplow_config::collectors::CollectorSpec,
    script: Option<&str>,
) -> Vec<String> {
    let mut needs = spec.service_needs();
    let entity = spec.facts.is_empty()
        && spec.runtime == oxplow_config::collectors::CollectorRuntime::Starlark;
    if entity
        && script.is_some_and(oxplow_script::ai::calls_ai)
        && !needs.iter().any(|n| n == "ai.compute")
    {
        needs.push("ai.compute".into());
    }
    needs
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::grant::{Decided, Enforcement, GrantRequest, Need};
    use oxplow_domain::run::{Authority, Mode};
    use oxplow_domain::Actor;

    /// A run of a contribution that declared `scopes`.
    fn run(mode: Mode, scopes: &[&str]) -> RunContext {
        RunContext::core(Actor::System, mode).authorized(Authority {
            declared: GrantRequest {
                scopes: scopes.iter().map(|s| s.to_string()).collect(),
                ..GrantRequest::default()
            },
            grant: None,
            enforcement: Enforcement { hosts: true },
        })
    }

    fn needs(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    const FILES: &str = "def transform(x):\n    return files('**')\n";
    const ASKS: &str = "def transform(x):\n    return ai_summarize('t')\n";

    /// A pure run — a composing script — can't ask a model, declared or
    /// not, and the refusal names the mode and lands on the run.
    #[test]
    fn a_pure_run_cant_ask_a_model() {
        let r = run(Mode::Pure, &["ai.compute"]);
        let err = decide(&r, ASKS, &needs(&["ai.compute"])).unwrap_err();
        assert!(
            err.starts_with("`ai_summarize()`: a pure run can't use `ai.compute`"),
            "{err}"
        );
        assert!(matches!(
            r.decisions().as_slice(),
            [Decided { need: Need::Scope(s), decision: Decision::Denied(_) }] if s == "ai.compute"
        ));
    }

    /// A service the script uses but doesn't declare is refused with the
    /// need it lacks, even on a run with no authority to check it.
    #[test]
    fn a_used_service_must_be_declared() {
        for r in [
            run(Mode::External, &[]),
            RunContext::core(Actor::System, Mode::External),
        ] {
            assert_eq!(
                decide(&r, FILES, &[]).unwrap_err(),
                "`files()` needs `tree.read` in this script's `needs`"
            );
        }
        let sql = "def transform(x):\n    return scope('sql.read', {'sql': 'SELECT 1'})\n";
        assert_eq!(
            decide(&run(Mode::Pure, &[]), sql, &[]).unwrap_err(),
            "`scope()` needs `sql.read` in this script's `needs`"
        );
    }

    /// A declared, allowed service is built from what the caller has; a
    /// caller with none to give says so.
    #[tokio::test]
    async fn an_allowed_service_is_built_from_the_callers_sources() {
        let r = run(Mode::External, &["tree.read"]);
        let tree: Tree = Arc::new([("a.rs".to_string(), "fn a() {}".to_string())].into());
        let given = prepare(
            &r,
            FILES,
            &needs(&["tree.read"]),
            &Sources::default().with_tree(Trees::Fixed(tree.clone())),
        )
        .await
        .unwrap();
        assert_eq!(given.tree, Some(tree));
        assert!(given.ai.is_none() && !given.scopes);
        let err = prepare(&r, FILES, &needs(&["tree.read"]), &Sources::default())
            .await
            .unwrap_err();
        assert_eq!(
            err,
            "`files()`: `tree.read` isn't available where this script runs"
        );
        assert_eq!(
            r.decisions()
                .iter()
                .map(|d| d.decision.clone())
                .collect::<Vec<_>>(),
            [Decision::Allowed, Decision::Allowed]
        );
    }

    /// What the script declares but its grant doesn't cover is refused,
    /// with the grant's reason.
    #[test]
    fn a_service_outside_the_grant_is_refused() {
        let err = decide(&run(Mode::External, &[]), FILES, &needs(&["tree.read"])).unwrap_err();
        assert_eq!(err, "`files()` was refused `tree.read`: not declared");
    }
}
