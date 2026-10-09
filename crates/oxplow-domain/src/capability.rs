//! Capabilities: the parts of oxplow a project chooses an implementation
//! of (`.context/work-tracking.md` "Capabilities"). Core declares each
//! one here — whether it may be "none", its default implementation and
//! the features an implementation may declare — and nothing else names a
//! capability's rules.
//!
//! An implementation is declared elsewhere (a built-in an extension names,
//! a provider instance); which one is active is resolved from the
//! person's and the project's choices (`oxplow_app::capabilities`).

use schemars::JsonSchema;

use crate::work_items::OXPLOW;
use serde::{Deserialize, Serialize};

/// Why a capability's active implementation is the one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
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
    pub fn as_str(self) -> &'static str {
        match self {
            ChosenBy::Personal => "personal",
            ChosenBy::Project => "project",
            ChosenBy::Default => "default",
            ChosenBy::Fallback => "fallback",
        }
    }
}

/// One capability, as core declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilitySpec {
    /// `work_items`, `effort_policy`, …: the key under `activeProviders`.
    pub id: &'static str,
    /// How a person names it.
    pub title: &'static str,
    /// Whether a project chooses its implementation. One that can't has
    /// a single implementation, always active (`vcs`, `knowledge`).
    pub choosable: bool,
    /// Whether it may be [`NONE`]: nothing implements it, and what
    /// needs it says so.
    pub optional: bool,
    /// Whether many implementations serve it at once (every agent
    /// harness, every AI provider) rather than one chosen: each one
    /// declared is active, nothing is chosen or switched, and a need on
    /// one of its features is met by any of them. Never choosable or
    /// optional.
    pub many: bool,
    /// The implementation used when none is chosen — and, for a required
    /// capability, when the chosen one isn't available. Core always has
    /// it. For a many-capability: the one used when nothing names one.
    pub default: &'static str,
    /// The features an implementation may declare.
    pub features: &'static [&'static str],
    /// What a process implementing it over the provider protocol must
    /// answer, and may emit and own; `None` when no process may.
    pub provider: Option<&'static ProviderContract>,
}

/// What core asks of a provider process implementing a capability
/// (`.context/providers.md` "What a provider may implement"): the host
/// enforces it, whichever capability it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderContract {
    /// The verbs core calls on it. A verb is `confirm: never` and `access:
    /// record`: core's call is what a person confirms and what is gated.
    pub verbs: &'static [Verb],
    /// The command family a verb runs as (`oxplow.work_item` →
    /// `oxplow.work_item.create`), so an inverse naming a verb undoes
    /// through it; `None` when core calls the verbs itself.
    pub dispatch: Option<&'static str>,
    /// The event types it may emit, each `(type, v)`; the schema it
    /// declares must be core's.
    pub events: &'static [(&'static str, u32)],
    /// What its collectors stream; `None` when it reads nothing in.
    pub records: Option<Record>,
    /// The ref kind whose ids are an instance's own
    /// (`work_item:<instance>:…`); `None` when it owns no refs.
    pub ref_kind: Option<&'static str>,
    /// Whether it keeps items: a manifest's `fields` and `id_pattern` apply.
    pub items: bool,
    /// The JSON Schema of what it declares of itself beyond its features
    /// (its capability declaration's `data`); `None` when it declares
    /// nothing more.
    pub data: Option<&'static str>,
}

/// One verb of a [`ProviderContract`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verb {
    pub name: &'static str,
    pub needs: Need,
}

/// When a declaration must carry a verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Always,
    /// When it declares this feature.
    Feature(&'static str),
}

/// What a provider's collectors stream: rows of `entity`, each logged as
/// an `event` whose payload holds the row under `key`, its subject the
/// row's own ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Record {
    pub entity: &'static str,
    pub event: (&'static str, u32),
    pub key: &'static str,
}

impl ProviderContract {
    /// The verb `name`, if it's one of this capability's.
    pub fn verb(&self, name: &str) -> Option<&'static Verb> {
        self.verbs.iter().find(|v| v.name == name)
    }

    /// The verbs a declaration with `features` (`{ "links": true }`) must
    /// carry.
    pub fn required(&self, features: &serde_json::Value) -> Vec<&'static str> {
        self.verbs
            .iter()
            .filter(|v| match v.needs {
                Need::Always => true,
                Need::Feature(f) => features.get(f).and_then(|b| b.as_bool()) == Some(true),
            })
            .map(|v| v.name)
            .collect()
    }
}

/// A work list's: the `work_item.<verb>` commands dispatch to its verbs,
/// it records its items as `work_item.recorded@2`, and owns
/// `work_item:<instance>:…`.
pub const WORK_ITEMS: ProviderContract = ProviderContract {
    verbs: &[
        Verb {
            name: "create",
            needs: Need::Always,
        },
        Verb {
            name: "update",
            needs: Need::Always,
        },
        Verb {
            name: "transition",
            needs: Need::Always,
        },
        Verb {
            name: "link",
            needs: Need::Feature("links"),
        },
        Verb {
            name: "comment",
            needs: Need::Feature("comments"),
        },
        Verb {
            name: "delete",
            needs: Need::Feature("delete"),
        },
        Verb {
            name: "reorder",
            needs: Need::Feature("ordering"),
        },
        Verb {
            name: "move",
            needs: Need::Feature("lists"),
        },
    ],
    dispatch: Some("oxplow.work_item"),
    events: &[("work_item.recorded", 2)],
    records: Some(Record {
        entity: "work_item",
        event: ("work_item.recorded", 2),
        key: "item",
    }),
    ref_kind: Some("work_item"),
    items: true,
    data: None,
};

/// An effort policy's: core offers it each event a policy reacts to and
/// runs the commands it answers with (`.context/work-tracking.md`).
pub const EFFORT_POLICY: ProviderContract = ProviderContract {
    verbs: &[Verb {
        name: "react",
        needs: Need::Always,
    }],
    dispatch: None,
    events: &[],
    records: None,
    ref_kind: None,
    items: false,
    data: None,
};

/// An agent harness's (`.context/agent-model.md`): core launches its
/// sessions through it, has it map its tool hooks and render its answers,
/// and — as its features say — read its transcript and telemetry and
/// rewrite its runtimes' text. What a built-in answers from code
/// (instruction files, environment markers, settings) it declares as its
/// `data` ([`HARNESS_DATA`]); a structured transcript is its feature.
pub const AGENT_HARNESS: ProviderContract = ProviderContract {
    verbs: &[
        Verb {
            name: "launch",
            needs: Need::Always,
        },
        Verb {
            name: "tool_use",
            needs: Need::Always,
        },
        Verb {
            name: "render",
            needs: Need::Always,
        },
        Verb {
            name: "refresh_text",
            needs: Need::Feature("runtime_text"),
        },
        Verb {
            name: "turns",
            needs: Need::Feature("transcript"),
        },
        Verb {
            name: "token_readings",
            needs: Need::Feature("telemetry"),
        },
        Verb {
            name: "prompt",
            needs: Need::Feature("subagents"),
        },
        Verb {
            name: "subagent",
            needs: Need::Feature("subagents"),
        },
    ],
    dispatch: None,
    events: &[],
    records: None,
    ref_kind: None,
    items: false,
    data: Some(HARNESS_DATA),
};

/// What a harness declares of itself (`agent::harness::HarnessData`).
pub const HARNESS_DATA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "instruction_files": { "type": "array", "items": { "type": "string", "minLength": 1 } },
    "env_markers": { "type": "array", "items": { "type": "string", "minLength": 1 } },
    "settings": {
      "type": "array",
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["key", "title"],
        "properties": {
          "key": { "type": "string", "minLength": 1 },
          "title": { "type": "string", "minLength": 1 },
          "hint": { "type": "string" },
          "placeholder": { "type": "string" }
        }
      }
    }
  }
}"#;

/// The implementation id of "nothing implements it", for an optional
/// capability.
pub const NONE: &str = "none";

/// Every capability core declares.
pub const CAPABILITIES: &[CapabilitySpec] = &[
    CapabilitySpec {
        id: "work_items",
        title: "Work list",
        choosable: true,
        optional: true,
        many: false,
        default: "oxplow",
        features: &[
            "hierarchy",
            "comments",
            "links",
            "delete",
            "idempotent_writes",
            "ordering",
            "lists",
        ],
        provider: Some(&WORK_ITEMS),
    },
    CapabilitySpec {
        id: "effort_policy",
        title: "Effort policy",
        choosable: true,
        optional: true,
        many: false,
        default: "oxplow",
        features: &[],
        provider: Some(&EFFORT_POLICY),
    },
    CapabilitySpec {
        id: "snapshots",
        title: "Snapshots",
        choosable: true,
        optional: false,
        many: false,
        default: "oxplow",
        features: &["contents"],
        provider: None,
    },
    CapabilitySpec {
        id: "vcs",
        title: "Version control",
        choosable: false,
        optional: false,
        many: false,
        default: "git",
        features: &[],
        provider: None,
    },
    CapabilitySpec {
        id: "knowledge",
        title: "Knowledge",
        choosable: false,
        optional: false,
        many: false,
        default: "oxplow",
        features: &[],
        provider: None,
    },
    CapabilitySpec {
        id: "agent_harness",
        title: "Agent harness",
        choosable: false,
        optional: false,
        many: true,
        default: "claude",
        features: &[
            "terminal",
            "structured_transcript",
            "permission_prompts",
            "resume",
            "programmatic",
            "transcript",
            "telemetry",
            "runtime_text",
            "subagents",
        ],
        provider: Some(&AGENT_HARNESS),
    },
    CapabilitySpec {
        id: "acp_adapter",
        title: "ACP agent",
        choosable: false,
        optional: false,
        many: true,
        default: "claude",
        features: &[],
        provider: None,
    },
    CapabilitySpec {
        id: "ai_provider",
        title: "AI provider",
        choosable: false,
        optional: false,
        many: true,
        default: "anthropic",
        features: &["decide_native"],
        provider: None,
    },
];

/// The contract of the capability `id`, when a process may implement it.
pub fn contract(id: &str) -> Option<&'static ProviderContract> {
    spec(id).and_then(|c| c.provider)
}

/// Why `id` can't name a provider or one of its instances — the segment
/// of its refs (`work_item:<id>:…`) and its command namespace — or `None`
/// when it can: lowercase letters, digits and underscores, starting with
/// a letter, and none of oxplow's own (`oxplow`, a core namespace). One
/// rule for a provider's id and an instance's.
pub fn instance_id_problem(id: &str) -> Option<String> {
    let well_formed = id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !well_formed {
        Some(format!(
            "`{id}` must be lowercase letters, digits and underscores, starting with a letter"
        ))
    } else if id == OXPLOW || crate::events::schema::CORE_NAMESPACES.contains(&id) {
        Some(format!("`{id}` is reserved for oxplow"))
    } else {
        None
    }
}

/// The capability `id`, if core declares it.
pub fn spec(id: &str) -> Option<&'static CapabilitySpec> {
    CAPABILITIES.iter().find(|c| c.id == id)
}

/// The capabilities a project chooses an implementation of.
pub fn choosable() -> impl Iterator<Item = &'static CapabilitySpec> {
    CAPABILITIES.iter().filter(|c| c.choosable)
}

/// Check a declaration's `config` against its built-in's JSON Schema
/// (`schema`, its text). The error says what's wrong, where.
pub fn check_config(schema: &str, config: &serde_json::Value) -> Result<(), String> {
    check_against(schema, config, "config")
}

/// Check `value` against the JSON Schema `schema` (its text); the error
/// names it `what`, then what's wrong, where.
pub fn check_against(schema: &str, value: &serde_json::Value, what: &str) -> Result<(), String> {
    let schema: serde_json::Value =
        serde_json::from_str(schema).map_err(|e| format!("the {what} schema: {e}"))?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|e| format!("the {what} schema: {e}"))?;
    let config = value;
    let errors: Vec<String> = validator
        .iter_errors(config)
        .map(|e| {
            let at = e.instance_path().to_string();
            if at.is_empty() {
                e.to_string()
            } else {
                format!("{at}: {e}")
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{what}: {}", errors.join("; ")))
    }
}

/// Check one declared need: a scope (`sql.read`,
/// [`crate::scope`]), a capability (`work_items`), or one of its
/// features (`snapshots.contents`), as core declares them.
pub fn check_need(need: &str) -> Result<(), String> {
    if crate::scope::scope(need).is_some() {
        return Ok(());
    }
    let (id, feature) = match need.split_once('.') {
        Some((id, feature)) => (id, Some(feature)),
        None => (need, None),
    };
    let Some(spec) = spec(id) else {
        return Err(format!(
            "needs `{need}`: `{id}` isn't a capability ({})",
            CAPABILITIES
                .iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if let Some(f) = feature {
        if !spec.features.contains(&f) {
            return Err(format!(
                "needs `{need}`: `{f}` isn't a feature of `{id}` ({})",
                if spec.features.is_empty() {
                    "it has none".to_string()
                } else {
                    spec.features.join(", ")
                }
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A harness's declared data passes the contract's schema as the
    /// domain shape writes it; anything else is named.
    #[test]
    fn harness_data_is_checked_against_the_contract() {
        use crate::agent::harness::{HarnessData, HarnessSetting};
        let schema = AGENT_HARNESS.data.unwrap();
        let data = HarnessData {
            instruction_files: vec!["AGENTS.md".into()],
            env_markers: vec!["ACME_SESSION".into()],
            settings: vec![HarnessSetting {
                key: "model".into(),
                title: "Model".into(),
                hint: String::new(),
                placeholder: "m".into(),
            }],
        };
        check_against(schema, &serde_json::to_value(&data).unwrap(), "data").unwrap();
        check_against(schema, &serde_json::json!({}), "data").unwrap();
        let err = check_against(schema, &serde_json::json!({"interact": "x"}), "data").unwrap_err();
        assert!(err.starts_with("data:"), "{err}");
        assert!(check_against(
            schema,
            &serde_json::json!({"settings": [{"title": "no key"}]}),
            "data"
        )
        .is_err());
    }

    #[test]
    fn a_need_may_name_a_scope() {
        assert!(check_need("sql.read").is_ok());
        assert!(check_need("sql.write").is_err());
    }

    #[test]
    fn every_capability_is_declared_once_with_a_default() {
        let mut ids: Vec<&str> = CAPABILITIES.iter().map(|c| c.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), CAPABILITIES.len());
        for c in CAPABILITIES {
            assert!(!c.default.is_empty() && c.default != NONE, "{}", c.id);
            assert!(
                c.choosable || !c.optional,
                "{}: only a choice may be none",
                c.id
            );
        }
        assert_eq!(
            choosable().map(|c| c.id).collect::<Vec<_>>(),
            ["work_items", "effort_policy", "snapshots"]
        );
        for c in CAPABILITIES.iter().filter(|c| c.many) {
            assert!(
                !c.choosable && !c.optional,
                "{}: many implementations are registered at once, never chosen or none",
                c.id
            );
        }
        assert_eq!(
            CAPABILITIES
                .iter()
                .filter(|c| c.many)
                .map(|c| (c.id, c.default))
                .collect::<Vec<_>>(),
            [
                ("agent_harness", "claude"),
                ("acp_adapter", "claude"),
                ("ai_provider", "anthropic")
            ]
        );
        assert_eq!(spec("snapshots").map(|c| c.optional), Some(false));
        assert!(spec("nope").is_none());
    }

    /// What a process may implement is what core gives a contract: the
    /// work list, the effort policy and an agent harness. The rest are
    /// core's or built-ins'.
    #[test]
    fn a_process_implements_only_a_capability_with_a_contract() {
        assert_eq!(
            CAPABILITIES
                .iter()
                .filter(|c| c.provider.is_some())
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            ["work_items", "effort_policy", "agent_harness"]
        );
        assert!(contract("snapshots").is_none() && contract("nope").is_none());
    }

    /// The verbs a declaration must carry follow its features; each verb's
    /// feature is one the capability declares.
    #[test]
    fn a_contracts_verbs_follow_the_declared_features() {
        let work = contract("work_items").unwrap();
        assert_eq!(
            work.required(&serde_json::json!({})),
            ["create", "update", "transition"]
        );
        assert_eq!(
            work.required(&serde_json::json!({"links": true, "lists": true, "comments": false})),
            ["create", "update", "transition", "link", "move"]
        );
        assert!(work.verb("reorder").is_some() && work.verb("estimate").is_none());
        let policy = contract("effort_policy").unwrap();
        assert_eq!(policy.required(&serde_json::json!({})), ["react"]);
        for c in CAPABILITIES {
            for verb in c.provider.map(|p| p.verbs).unwrap_or_default() {
                if let Need::Feature(f) = verb.needs {
                    assert!(c.features.contains(&f), "{}: {}", c.id, f);
                }
            }
        }
    }

    /// A work list owns its items' refs and streams them as records; an
    /// effort policy owns nothing and emits nothing.
    #[test]
    fn a_contract_says_what_its_provider_owns_and_emits() {
        let work = contract("work_items").unwrap();
        assert_eq!(work.events, [("work_item.recorded", 2)]);
        assert_eq!(work.ref_kind, Some("work_item"));
        assert_eq!(
            work.records.map(|r| (r.entity, r.event, r.key)),
            Some(("work_item", ("work_item.recorded", 2), "item"))
        );
        assert_eq!(work.dispatch, Some("oxplow.work_item"));
        assert!(work.items);
        let policy = contract("effort_policy").unwrap();
        assert!(policy.events.is_empty() && policy.records.is_none());
        assert_eq!(
            (policy.ref_kind, policy.dispatch, policy.items),
            (None, None, false)
        );
    }

    #[test]
    fn an_instance_id_is_lowercase_snake_case_and_not_oxplows() {
        assert_eq!(instance_id_problem("linear_2"), None);
        assert!(instance_id_problem("Linear").is_some());
        assert!(instance_id_problem("2x").is_some());
        assert!(instance_id_problem("oxplow").unwrap().contains("reserved"));
    }

    #[test]
    fn a_need_names_a_capability_or_one_of_its_features() {
        assert!(check_need("work_items").is_ok());
        assert!(check_need("snapshots.contents").is_ok());
        assert!(check_need("teleport")
            .unwrap_err()
            .contains("isn't a capability"));
        assert!(check_need("work_items.flying")
            .unwrap_err()
            .contains("isn't a feature"));
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    /// A declaration's `config:` is checked against its built-in's schema:
    /// the error names what's wrong.
    #[test]
    fn config_is_checked_against_a_schema() {
        let schema = r#"{"type":"object","required":["command"],"properties":{"command":{"type":"string"}},"additionalProperties":false}"#;
        assert!(check_config(schema, &serde_json::json!({"command": "gemini"})).is_ok());
        let err = check_config(schema, &serde_json::json!({})).unwrap_err();
        assert!(err.contains("command"), "{err}");
        let err =
            check_config(schema, &serde_json::json!({"command": "x", "nope": 1})).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }
}
