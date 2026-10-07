//! Which implementation of each capability is active
//! (`.context/work-tracking.md` "Swappable pieces").
//!
//! Core declares the capabilities (`oxplow_domain::capability`). The
//! implementations come from three places, all held by one
//! [`CapabilityRegistry`]:
//!
//! - **core**: the fixed ones (the VCS, knowledge) and, for every optional
//!   capability, [`NONE`] — nothing implements it;
//! - **extensions**: `implementations:` naming a built-in in core's
//!   standard library ([`BUILT_INS`], `entry: oxplow:tasks`), the way a
//!   collector names `oxplow:junit`. `oxplow-bundled` declares the
//!   defaults; disabling it takes them away;
//! - **provider instances** while they run.
//!
//! [`CapabilityRegistry::resolve`] picks the active one: a person's own
//! choice (`.oxplow/personal.yaml`), else the project's
//! (`activeProviders`), else the capability's default. A choice that isn't
//! available falls back: an optional capability to [`NONE`], a required
//! one to its default built-in, which core always has. The registry
//! writes what it holds, with the active row and why, to
//! `v_capability_provider`.

use std::sync::RwLock;

use oxplow_config::OxplowConfig;
use oxplow_db::{CapabilityProvider, SqliteCapabilityStore};
use oxplow_domain::capability::{self, NONE};
use oxplow_domain::DomainError;
use serde_json::Value;

/// Core's standard library of implementations, by entry name, with the
/// capability each implements. An extension declares one by naming it;
/// core resolves the name, like a collector's `oxplow:junit`.
pub const BUILT_INS: &[BuiltIn] = &[
    BuiltIn {
        entry: "oxplow:tasks",
        capability: "work_items",
        title: "oxplow's tasks",
        features: &["hierarchy", "comments", "links", "delete"],
    },
    BuiltIn {
        entry: "oxplow:commit-or-switch",
        capability: "effort_policy",
        title: "A commit lands it, or the task switches",
        features: &[],
    },
    BuiltIn {
        entry: "oxplow:snapshots",
        capability: "snapshots",
        title: "Keep every version",
        features: &["contents"],
    },
];

/// One built-in implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltIn {
    pub entry: &'static str,
    pub capability: &'static str,
    pub title: &'static str,
    /// The features it has — core's to say, since it's core's code.
    pub features: &'static [&'static str],
}

/// The built-in `entry` names, if core has it.
pub fn built_in(entry: &str) -> Option<&'static BuiltIn> {
    BUILT_INS.iter().find(|b| b.entry == entry)
}

/// How an implementation is loaded — never how it's called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Core's own (the VCS, knowledge).
    Core,
    /// A built-in in core's standard library, by its entry.
    BuiltIn(&'static str),
    /// A provider instance's process.
    External,
    /// Nothing implements it.
    None,
}

impl Source {
    fn as_str(&self) -> &'static str {
        match self {
            Source::Core => "core",
            Source::BuiltIn(_) => "builtin",
            Source::External => "external",
            Source::None => "none",
        }
    }
}

/// One implementation of a capability.
#[derive(Debug, Clone, PartialEq)]
pub struct Implementation {
    pub capability: String,
    /// Its id: what `activeProviders` names (`oxplow`, `issues`, `none`).
    pub id: String,
    pub title: String,
    /// The extension declaring it; `None` for core's.
    pub extension: Option<String>,
    pub source: Source,
    /// The features it declares.
    pub features: Value,
}

impl Implementation {
    /// The "nothing implements it" of an optional capability.
    pub fn none(capability: &str) -> Self {
        Self {
            capability: capability.into(),
            id: NONE.into(),
            title: "None".into(),
            extension: None,
            source: Source::None,
            features: Value::Object(Default::default()),
        }
    }
}

/// Why the active implementation is the one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChosenBy {
    /// The person's own layer.
    Personal,
    /// The project's `activeProviders`.
    Project,
    /// Nothing chose: the capability's default.
    Default,
    /// The choice isn't available (its extension is disabled, its
    /// instance stopped, its id unknown).
    Fallback,
}

impl ChosenBy {
    fn as_str(self) -> &'static str {
        match self {
            ChosenBy::Personal => "personal",
            ChosenBy::Project => "project",
            ChosenBy::Default => "default",
            ChosenBy::Fallback => "fallback",
        }
    }
}

/// A capability's active implementation and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    pub chosen_by: ChosenBy,
    /// What was chosen, when it fell back from it.
    pub wanted: Option<String>,
}

/// Every implementation oxplow has now: core's, the ones extensions
/// declare, and running provider instances'.
#[derive(Default)]
pub struct CapabilityRegistry {
    core: RwLock<Vec<Implementation>>,
    declared: RwLock<Vec<Implementation>>,
    external: RwLock<Vec<Implementation>>,
}

impl CapabilityRegistry {
    /// A registry with core's fixed implementations (`vcs`, `knowledge`)
    /// and [`NONE`] for each optional capability.
    pub fn new(fixed: Vec<Implementation>) -> Self {
        let mut core = fixed;
        core.extend(
            capability::CAPABILITIES
                .iter()
                .filter(|c| c.optional)
                .map(|c| Implementation::none(c.id)),
        );
        Self {
            core: RwLock::new(core),
            ..Self::default()
        }
    }

    /// Add one of core's own (built after the registry: knowledge).
    pub fn add_core(&self, implementation: Implementation) {
        self.core
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(implementation);
    }

    /// Replace what extensions declare (on boot, and when the catalog
    /// changes).
    pub fn set_declared(&self, declared: Vec<Implementation>) {
        *self.declared.write().unwrap_or_else(|e| e.into_inner()) = declared;
    }

    /// A provider instance started (`true`) or stopped.
    pub fn set_external(&self, implementation: Implementation, running: bool) {
        let mut external = self.external.write().unwrap_or_else(|e| e.into_inner());
        external
            .retain(|i| !(i.capability == implementation.capability && i.id == implementation.id));
        if running {
            external.push(implementation);
        }
    }

    /// Every implementation, core's first.
    pub fn implementations(&self) -> Vec<Implementation> {
        let mut all = self.core.read().unwrap_or_else(|e| e.into_inner()).clone();
        all.extend(
            self.declared
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        all.extend(
            self.external
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned(),
        );
        all
    }

    /// `capability`'s implementation `id`, if available.
    pub fn get(&self, capability: &str, id: &str) -> Option<Implementation> {
        self.implementations()
            .into_iter()
            .find(|i| i.capability == capability && i.id == id)
    }

    /// `capability`'s active implementation under `config`, and why.
    pub fn resolve(&self, config: &OxplowConfig, capability: &str) -> Resolved {
        let Some(spec) = capability::spec(capability) else {
            return Resolved {
                id: NONE.into(),
                chosen_by: ChosenBy::Fallback,
                wanted: None,
            };
        };
        let (wanted, chosen_by) = if !spec.choosable {
            (spec.default.to_string(), ChosenBy::Default)
        } else if let Some(id) = config.personal_active_providers.get(capability) {
            (id.clone(), ChosenBy::Personal)
        } else if let Some(id) = config.active_providers.get(capability) {
            (id.clone(), ChosenBy::Project)
        } else {
            (spec.default.to_string(), ChosenBy::Default)
        };
        if self.get(capability, &wanted).is_some() {
            return Resolved {
                id: wanted,
                chosen_by,
                wanted: None,
            };
        }
        Resolved {
            // A required capability's default is core's: always there.
            id: if spec.optional {
                NONE.to_string()
            } else {
                spec.default.to_string()
            },
            chosen_by: ChosenBy::Fallback,
            wanted: Some(wanted),
        }
    }

    /// `capability`'s active implementation id under `config`.
    pub fn active(&self, config: &OxplowConfig, capability: &str) -> String {
        self.resolve(config, capability).id
    }

    /// Restate `v_capability_provider` from what the registry holds: every
    /// implementation, the active one marked with why, and a choice that
    /// isn't available as its own row (`available = 0`).
    pub async fn publish(
        &self,
        config: &OxplowConfig,
        db: &oxplow_db::Database,
    ) -> Result<(), DomainError> {
        let all = self.implementations();
        let mut rows: Vec<CapabilityProvider> = Vec::new();
        for spec in capability::CAPABILITIES {
            let resolved = self.resolve(config, spec.id);
            for i in all.iter().filter(|i| i.capability == spec.id) {
                let active = i.id == resolved.id;
                rows.push(CapabilityProvider {
                    capability: i.capability.clone(),
                    provider: i.id.clone(),
                    extension: i.extension.clone(),
                    features: i.features.clone(),
                    active,
                    title: i.title.clone(),
                    source: i.source.as_str().into(),
                    available: true,
                    chosen_by: active.then(|| resolved.chosen_by.as_str().to_string()),
                });
            }
            if let Some(wanted) = &resolved.wanted {
                if !rows
                    .iter()
                    .any(|r| r.capability == spec.id && &r.provider == wanted)
                {
                    rows.push(CapabilityProvider {
                        capability: spec.id.into(),
                        provider: wanted.clone(),
                        extension: None,
                        features: Value::Object(Default::default()),
                        active: false,
                        title: wanted.clone(),
                        source: "unknown".into(),
                        available: false,
                        chosen_by: None,
                    });
                }
            }
        }
        SqliteCapabilityStore::new(db.clone()).reset(rows).await
    }
}

/// The implementations `extensions` declare, as the registry holds them.
pub fn declared_by(extensions: &[crate::extensions::Extension]) -> Vec<Implementation> {
    extensions
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| {
            e.implementations.iter().filter_map(|d| {
                let b = built_in(&d.entry)?;
                Some(Implementation {
                    capability: d.capability.clone(),
                    id: d.id.clone(),
                    title: d.title.clone().unwrap_or_else(|| b.title.to_string()),
                    extension: Some(e.name.clone()),
                    source: Source::BuiltIn(b.entry),
                    features: Value::Object(
                        b.features
                            .iter()
                            .map(|f| (f.to_string(), Value::Bool(true)))
                            .collect(),
                    ),
                })
            })
        })
        .collect()
}

/// Restate the registry's declared implementations from the project's
/// extensions and publish the rows.
pub async fn refresh(svc: &crate::Services) -> Result<(), DomainError> {
    let extensions = svc.extension_catalog.get(&svc.layout.project_dir);
    svc.capabilities.set_declared(declared_by(&extensions));
    let config = crate::config_service::read_config(&svc.config);
    svc.capabilities.publish(&config, &svc.db).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin(capability: &str, id: &str, entry: &'static str) -> Implementation {
        Implementation {
            capability: capability.into(),
            id: id.into(),
            title: id.into(),
            extension: Some("oxplow-bundled".into()),
            source: Source::BuiltIn(entry),
            features: Value::Null,
        }
    }

    fn registry(declared: bool) -> CapabilityRegistry {
        let r = CapabilityRegistry::new(Vec::new());
        if declared {
            r.set_declared(vec![
                builtin("work_items", "oxplow", "oxplow:tasks"),
                builtin("effort_policy", "oxplow", "oxplow:commit-or-switch"),
                builtin("snapshots", "oxplow", "oxplow:snapshots"),
            ]);
        }
        r
    }

    fn config(project: &[(&str, &str)], personal: &[(&str, &str)]) -> OxplowConfig {
        let mut c =
            oxplow_config::load_project_config(tempfile::tempdir().unwrap().path()).unwrap();
        let map = |pairs: &[(&str, &str)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        c.active_providers = map(project);
        c.personal_active_providers = map(personal);
        c
    }

    /// A person's choice over the project's over the default.
    #[test]
    fn a_choice_is_the_persons_then_the_projects_then_the_default() {
        let r = registry(true);
        let res = |c: &OxplowConfig| r.resolve(c, "effort_policy");
        assert_eq!(
            res(&config(&[], &[])),
            Resolved {
                id: "oxplow".into(),
                chosen_by: ChosenBy::Default,
                wanted: None
            }
        );
        assert_eq!(
            res(&config(&[("effort_policy", "none")], &[])).chosen_by,
            ChosenBy::Project
        );
        let both = config(&[("effort_policy", "none")], &[("effort_policy", "oxplow")]);
        assert_eq!(
            (res(&both).id.as_str(), res(&both).chosen_by),
            ("oxplow", ChosenBy::Personal)
        );
    }

    /// A choice that isn't available falls back: an optional capability to
    /// none, a required one to its default — which core always has, so
    /// disabling the extension that declares the defaults leaves snapshots
    /// working.
    #[test]
    fn an_unavailable_choice_falls_back() {
        let r = registry(true);
        let gone = r.resolve(&config(&[("work_items", "issues")], &[]), "work_items");
        assert_eq!(
            gone,
            Resolved {
                id: NONE.into(),
                chosen_by: ChosenBy::Fallback,
                wanted: Some("issues".into())
            }
        );
        let disabled = registry(false);
        let none = config(&[], &[]);
        assert_eq!(disabled.active(&none, "work_items"), NONE);
        assert_eq!(disabled.active(&none, "effort_policy"), NONE);
        let snapshots = disabled.resolve(&none, "snapshots");
        assert_eq!(
            (snapshots.id.as_str(), snapshots.chosen_by),
            ("oxplow", ChosenBy::Fallback)
        );
        assert_eq!(disabled.active(&none, "vcs"), "git");
    }

    /// A provider instance is available while it runs.
    #[test]
    fn a_running_instance_is_available() {
        let r = registry(true);
        let c = config(&[("work_items", "issues")], &[]);
        let issues = Implementation {
            capability: "work_items".into(),
            id: "issues".into(),
            title: "Issues".into(),
            extension: Some("tracker".into()),
            source: Source::External,
            features: Value::Null,
        };
        r.set_external(issues.clone(), true);
        assert_eq!(r.active(&c, "work_items"), "issues");
        r.set_external(issues, false);
        assert_eq!(r.active(&c, "work_items"), NONE);
    }

    /// `oxplow:tasks`'s features in core's table are what oxplow's
    /// work-items provider does: the two can't drift.
    #[test]
    fn the_tasks_built_in_declares_what_the_provider_does() {
        let provider = serde_json::to_value(crate::work_items::oxplow_provider().features).unwrap();
        let on: Vec<&str> = provider
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, v)| v.as_bool() == Some(true))
            .map(|(k, _)| k.as_str())
            .collect();
        let mut declared = built_in("oxplow:tasks").unwrap().features.to_vec();
        declared.sort();
        let mut on = on;
        on.sort();
        assert_eq!(declared, on);
    }

    /// With `oxplow-bundled` disabled, nothing declares the defaults: the
    /// work list and the effort policy fall to none, snapshots to core's
    /// default — and filing says no work list is active.
    #[tokio::test]
    async fn disabling_the_bundled_extension_leaves_core_usable() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        svc.config.write().unwrap().extensions_disabled = vec!["oxplow-bundled".into()];
        std::fs::create_dir_all(svc.layout.project_dir.join(".oxplow")).unwrap();
        std::fs::write(
            oxplow_config::config_path(&svc.layout.project_dir),
            "extensions:\n  disabled: [oxplow-bundled]\n",
        )
        .unwrap();
        svc.extension_catalog.changed();
        refresh(svc).await.unwrap();
        let config = crate::config_service::read_config(&svc.config);
        assert_eq!(svc.capabilities.active(&config, "work_items"), NONE);
        assert_eq!(svc.capabilities.active(&config, "effort_policy"), NONE);
        let snapshots = svc.capabilities.resolve(&config, "snapshots");
        assert_eq!(
            (snapshots.id.as_str(), snapshots.chosen_by),
            ("oxplow", ChosenBy::Fallback)
        );
        let err = svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::work_item::CREATE,
                serde_json::json!({ "title": "x" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            format!("{err:?}").contains("no work list is active"),
            "{err:?}"
        );
    }

    /// The rows say which is active and why, and a choice that isn't
    /// available is listed as such.
    #[tokio::test]
    async fn the_rows_say_what_is_active_and_why() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let c = config(&[("work_items", "issues")], &[("effort_policy", "none")]);
        fx.svc.capabilities.publish(&c, &fx.svc.db).await.unwrap();
        let out = fx
            .svc
            .sql
            .query_sql(
                "SELECT capability, provider, active, chosen_by, available FROM v_capability_provider
                  WHERE capability IN ('work_items', 'effort_policy') ORDER BY capability, provider",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            serde_json::json!([
                ["effort_policy", "none", 1, "personal", 1],
                ["effort_policy", "oxplow", 0, null, 1],
                ["work_items", "issues", 0, null, 0],
                ["work_items", "none", 1, "fallback", 1],
                ["work_items", "oxplow", 0, null, 1],
            ])
        );
    }
}
