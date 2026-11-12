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
    /// `snapshots`, `knowledge`, …: the key under `activeProviders`.
    pub id: &'static str,
    /// How a person names it.
    pub title: &'static str,
    /// Whether a project chooses its implementation. One that can't has
    /// a single implementation, always active (`vcs`).
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
    /// The command family a verb runs as (`oxplow.task` →
    /// `oxplow.task.create`), so an inverse naming a verb undoes
    /// through it; `None` when core calls the verbs itself.
    pub dispatch: Option<&'static str>,
    /// The event types it may emit, each `(type, v)`; the schema it
    /// declares must be core's.
    pub events: &'static [(&'static str, u32)],
    /// What its collectors stream; `None` when it reads nothing in.
    pub records: Option<Record>,
    /// The resource whose URIs an instance owns (`oxplow/task`, its
    /// `provider` param the instance); `None` when it owns none.
    pub resource: Option<&'static str>,
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

/// A snapshot implementation's (`.context/providers.md` "A snapshots
/// provider"): core asks it to mark a stream's worktree and what changed
/// between two of its marks, and records what it answers in its own
/// ledger; with `contents` core pulls each new file's bytes at the mark.
pub const SNAPSHOTS: ProviderContract = ProviderContract {
    verbs: &[
        Verb {
            name: "mark",
            needs: Need::Always,
        },
        Verb {
            name: "changed",
            needs: Need::Always,
        },
        Verb {
            name: "read_at",
            needs: Need::Feature("contents"),
        },
    ],
    dispatch: None,
    events: &[],
    records: None,
    resource: None,
    items: false,
    data: None,
};

/// A knowledge store's (`.context/knowledge.md` "The capability"): the
/// page commands dispatch to its verbs, each answering the page's
/// `knowledge.page.recorded@1` (the body it kept and the refs it states),
/// and its collector streams its pages (edited outside oxplow) as the same
/// records. Its pages are `oxplow:oxplow/knowledge?id=<slug>&store=oxwiki`: bare refs, the active store's.
pub const KNOWLEDGE: ProviderContract = ProviderContract {
    verbs: &[
        Verb {
            name: "write_page",
            needs: Need::Always,
        },
        Verb {
            name: "delete_page",
            needs: Need::Always,
        },
    ],
    dispatch: None,
    events: &[("knowledge.page.recorded", 3)],
    records: Some(Record {
        entity: "knowledge_page",
        event: ("knowledge.page.recorded", 3),
        key: "page",
    }),
    resource: Some("oxplow/knowledge"),
    items: false,
    data: Some(KNOWLEDGE_DATA),
};

/// What a knowledge store declares of itself: its kinds of page, each in
/// one of core's buckets (`knowledge::KindDecl`); none declared: `note`.
pub const KNOWLEDGE_DATA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "kinds": {
      "type": "array",
      "minItems": 1,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["id", "title", "bucket"],
        "properties": {
          "id": { "type": "string", "minLength": 1 },
          "title": { "type": "string", "minLength": 1 },
          "bucket": { "enum": ["note", "memory", "instruction"] }
        }
      }
    }
  }
}"#;

/// An agent harness's (`.context/agent-model.md`): core launches its
/// sessions through it, has it map its tool hooks and render its answers,
/// and — as its features say — read its transcript and telemetry and
/// rewrite its runtimes' text. What a built-in answers from code
/// (environment markers, settings) it declares as its
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
    resource: None,
    items: false,
    data: Some(HARNESS_DATA),
};

/// A filesystem's (`.context/filesystems.md`): core asks it what a path is
/// (`stat`), for its bytes (`read`, whole or a range) and for its listing,
/// and — with `writable` — to replace a file. Its id is the
/// `filesystem` param of the file URIs it owns
/// (`oxplow:oxplow/file?filesystem=<extension>:<name>&path=…`).
pub const FILESYSTEM: ProviderContract = ProviderContract {
    verbs: &[
        Verb {
            name: "stat",
            needs: Need::Always,
        },
        Verb {
            name: "read",
            needs: Need::Always,
        },
        Verb {
            name: "list",
            needs: Need::Always,
        },
        Verb {
            name: "write",
            needs: Need::Feature("writable"),
        },
    ],
    dispatch: None,
    events: &[],
    records: None,
    resource: None,
    items: false,
    data: None,
};

/// What a harness declares of itself (`agent::harness::HarnessData`).
pub const HARNESS_DATA: &str = r#"{
  "type": "object",
  "additionalProperties": false,
  "properties": {
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

/// Where the active implementation is read from (the config as it is
/// now), so a person's choice applies to the next call with no copy to
/// refresh late.
pub type ActiveSource = std::sync::Arc<dyn Fn() -> String + Send + Sync>;

/// The implementation id of "nothing implements it", for an optional
/// capability.
pub const NONE: &str = "none";

/// Every capability core declares.
pub const CAPABILITIES: &[CapabilitySpec] = &[
    CapabilitySpec {
        id: "snapshots",
        title: "Snapshots",
        choosable: true,
        optional: false,
        many: false,
        default: "oxplow",
        features: &["contents"],
        provider: Some(&SNAPSHOTS),
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
        many: true,
        default: "oxwiki",
        features: &["edit"],
        provider: Some(&KNOWLEDGE),
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
            "permission_prompts",
            "resume",
            "programmatic",
            "transcript",
            "telemetry",
            "runtime_text",
            "subagents",
            "prompt_context",
            "tool_context",
            "conversation_list",
            "conversation_fork",
            "native_rename",
        ],
        provider: Some(&AGENT_HARNESS),
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
    CapabilitySpec {
        id: "filesystem",
        title: "Filesystem",
        choosable: false,
        optional: false,
        many: true,
        default: "local",
        features: &["writable"],
        provider: Some(&FILESYSTEM),
    },
];

/// The contract of the capability `id`, when a process may implement it.
pub fn contract(id: &str) -> Option<&'static ProviderContract> {
    spec(id).and_then(|c| c.provider)
}

/// Why `id` can't name a provider or one of its instances — the segment
/// of its refs (`oxplow:oxplow/task?id=…&provider=<id>`) and its command namespace — or `None`
/// when it can: lowercase letters, digits and underscores, starting with
/// a letter, and none of oxplow's own (`oxplow`, `oxtask`, a core namespace). One
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
    } else if id == "oxplow"
        || id == "oxtask"
        || crate::events::schema::CORE_NAMESPACES.contains(&id)
    {
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

/// The shared model a `model:<namespace>/<model>` need names, if it is one.
pub fn model_need(need: &str) -> Option<&str> {
    need.strip_prefix("model:")
}

/// Check one declared need: a scope (`sql.read`,
/// [`crate::scope`]), a capability (`snapshots`), one of its
/// features (`snapshots.contents`), as core declares them, or a shared
/// model with a live store (`model:oxplow/task_record`).
pub fn check_need(need: &str) -> Result<(), String> {
    if crate::scope::scope(crate::service::need_scope(need)).is_some() {
        return Ok(());
    }
    if let Some(model) = model_need(need) {
        let word = |w: &str| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        };
        return match model.split('/').collect::<Vec<_>>()[..] {
            [ns, name] if word(ns) && word(name) => Ok(()),
            _ => Err(format!(
                "needs `{need}`: a model need is `model:<namespace>/<model>`"
            )),
        };
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
        assert_eq!(choosable().map(|c| c.id).collect::<Vec<_>>(), ["snapshots"]);
        assert_eq!(spec("knowledge").map(|c| c.optional), Some(false));
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
                ("knowledge", "oxwiki"),
                ("agent_harness", "claude"),
                ("ai_provider", "anthropic"),
                ("filesystem", "local")
            ]
        );
        assert_eq!(spec("snapshots").map(|c| c.optional), Some(false));
        assert!(spec("nope").is_none());
    }

    /// What a process may implement is what core gives a contract:
    /// snapshots, knowledge, an agent harness and a filesystem. The rest are core's or built-ins'.
    #[test]
    fn a_process_implements_only_a_capability_with_a_contract() {
        assert_eq!(
            CAPABILITIES
                .iter()
                .filter(|c| c.provider.is_some())
                .map(|c| c.id)
                .collect::<Vec<_>>(),
            ["snapshots", "knowledge", "agent_harness", "filesystem"]
        );
        assert!(contract("vcs").is_none() && contract("nope").is_none());
    }

    /// The verbs a declaration must carry follow its features; each verb's
    /// feature is one the capability declares.
    #[test]
    fn a_contracts_verbs_follow_the_declared_features() {
        let knowledge = contract("knowledge").unwrap();
        assert_eq!(
            knowledge.required(&serde_json::json!({})),
            ["write_page", "delete_page"]
        );
        assert_eq!(knowledge.resource, Some("oxplow/knowledge"));
        assert_eq!(
            knowledge.records.map(|r| (r.entity, r.event, r.key)),
            Some(("knowledge_page", ("knowledge.page.recorded", 3), "page"))
        );
        let snapshots = contract("snapshots").unwrap();
        assert_eq!(
            snapshots.required(&serde_json::json!({})),
            ["mark", "changed"]
        );
        assert_eq!(
            snapshots.required(&serde_json::json!({"contents": true})),
            ["mark", "changed", "read_at"]
        );
        let files = contract("filesystem").unwrap();
        assert_eq!(
            files.required(&serde_json::json!({})),
            ["stat", "read", "list"]
        );
        assert_eq!(
            files.required(&serde_json::json!({"writable": true})),
            ["stat", "read", "list", "write"]
        );
        for c in CAPABILITIES {
            for verb in c.provider.map(|p| p.verbs).unwrap_or_default() {
                if let Need::Feature(f) = verb.needs {
                    assert!(c.features.contains(&f), "{}: {}", c.id, f);
                }
            }
        }
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
        assert!(check_need("snapshots").is_ok());
        assert!(check_need("snapshots.contents").is_ok());
        assert!(check_need("teleport")
            .unwrap_err()
            .contains("isn't a capability"));
        assert!(check_need("snapshots.flying")
            .unwrap_err()
            .contains("isn't a feature"));
    }

    /// `model:<owner namespace>/<model>` needs a shared model with a live
    /// store; its shape is checked, whether the model exists is the app's.
    #[test]
    fn a_need_may_name_a_shared_model() {
        assert!(check_need("model:oxplow/task_record").is_ok());
        for bad in [
            "model:",
            "model:task_record",
            "model:oxplow/",
            "model:a/b/c",
        ] {
            assert!(
                check_need(bad)
                    .unwrap_err()
                    .contains("model:<namespace>/<model>"),
                "{bad}"
            );
        }
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
