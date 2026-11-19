//! The agent harnesses core runs, as declared implementations
//! (`agent_harness` built-ins, `.context/agent-model.md`): each one an
//! `AgentHarness` (`oxplow_harnesses`) registered under the key its
//! declaration gives — the key an agent session's `harness` names. Many are
//! registered at once, the way collectors are.

use std::sync::Arc;

use oxplow_domain::agent::harness::AgentHarness;
use oxplow_domain::agent::registry::HarnessRegistry;

use crate::capabilities::{Implementation, Source};

/// Harnesses prepared from one declaration snapshot, without registry writes.
pub struct PreparedHarnesses {
    harnesses: Vec<Arc<dyn AgentHarness>>,
}

impl PreparedHarnesses {
    pub fn publish(self, registry: &HarnessRegistry) {
        registry.set_declared(self.harnesses);
    }
}

pub fn prepare_built_ins(declared: &[Implementation]) -> PreparedHarnesses {
    let harnesses = declared
        .iter()
        .filter(|i| i.capability == "agent_harness")
        .filter_map(|i| match i.source {
            Source::BuiltIn(entry) => oxplow_harnesses::built_in(entry, &i.id, &i.title),
            _ => None,
        })
        .collect();
    PreparedHarnesses { harnesses }
}

/// One registered harness, as the session picker and Settings see it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessListing {
    /// Its key (what `agents:` and a session's `harness` name).
    pub id: String,
    pub title: String,
    /// The project enables it (`agents:` names it, or names none).
    pub enabled: bool,
    /// The settings it reads from its `agentConfig` entry.
    pub settings: Vec<HarnessSettingListing>,
    /// The `agent_harness` features it declares (`resume`,
    /// `conversation_list`, …): what the UI offers for its sessions.
    pub features: Vec<String>,
    /// Its logo, a `data:image/svg+xml;base64,…` URI the UI draws as an
    /// image (never as markup), when its declaration has an `icon:`.
    pub icon: Option<String>,
}

/// A harness setting, as Settings → Agents shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSettingListing {
    pub key: String,
    pub title: String,
    pub hint: String,
    pub placeholder: String,
}

/// Every registered harness in alphabetical title order, including disabled ones.
pub fn listing(
    registry: &HarnessRegistry,
    capabilities: &crate::capabilities::CapabilityRegistry,
    enabled: &std::collections::BTreeMap<String, bool>,
) -> Vec<HarnessListing> {
    let mut all = registry.all();
    all.sort_by_key(|h| (h.title().to_lowercase(), h.id().to_string()));
    all.iter()
        .map(|h| HarnessListing {
            id: h.id().to_string(),
            title: h.title().to_string(),
            enabled: enabled.get(h.id()).copied().unwrap_or(true),
            settings: h
                .settings()
                .into_iter()
                .map(|s| HarnessSettingListing {
                    key: s.key,
                    title: s.title,
                    hint: s.hint,
                    placeholder: s.placeholder,
                })
                .collect(),
            features: capabilities
                .get("agent_harness", h.id())
                .and_then(|i| i.features.as_object().cloned())
                .map(|f| {
                    f.into_iter()
                        .filter(|(_, on)| on.as_bool() == Some(true))
                        .map(|(name, _)| name)
                        .collect()
                })
                .unwrap_or_default(),
            icon: capabilities
                .get("agent_harness", h.id())
                .and_then(|i| i.icon.clone()),
        })
        .collect()
}

/// Independent settings for every declared built-in harness, including disabled ones.
pub(crate) fn setting_definitions(
    extensions: &[crate::extension::extensions::Extension],
) -> Vec<oxplow_config::settings::SettingDefinition> {
    use oxplow_config::settings::SettingDefinition;
    let mut definitions = std::collections::BTreeMap::new();
    for declaration in extensions
        .iter()
        .filter(|e| e.enabled)
        .flat_map(|e| &e.implementations)
        .filter(|i| i.capability == "agent_harness")
    {
        let Some(harness) = declaration.entry.as_deref().and_then(|entry| {
            oxplow_harnesses::built_in(
                entry,
                &declaration.id,
                declaration.title.as_deref().unwrap_or(&declaration.id),
            )
        }) else {
            continue;
        };
        let prefix = format!("oxplow.agents.{}", harness.id());
        let base = |key: String,
                    title: String,
                    description: String,
                    schema: serde_json::Value,
                    default| SettingDefinition {
            key,
            title,
            description,
            category: "agents".into(),
            schema,
            default,
            secret: false,
            order: 0,
            ui: Default::default(),
            mask: Default::default(),
            owner: "core".into(),
        };
        let enabled = base(
            format!("{prefix}.enabled"),
            format!("Enable {}", harness.title()),
            "Allow new sessions with this agent".into(),
            serde_json::json!({"type":"boolean"}),
            Some(serde_json::json!(true)),
        );
        definitions.insert(enabled.key.clone(), enabled);
        for setting in harness.settings() {
            let definition = base(
                format!(
                    "{prefix}.{}",
                    oxplow_config::settings::key_segment(&setting.key)
                ),
                format!("{} {}", harness.title(), setting.title),
                setting.hint,
                setting.schema,
                None,
            );
            definitions.insert(definition.key.clone(), definition);
        }
    }
    definitions.into_values().collect()
}

pub(crate) fn declared_instance_settings(
    project: &std::path::Path,
    extension: &crate::extension::extensions::Extension,
    provider: &crate::providers::ProviderSpec,
    instance: &str,
) -> Vec<oxplow_config::settings::SettingDefinition> {
    if provider.capability != "agent_harness" {
        return Vec::new();
    }
    let declared = crate::providers::spec::read_declarations(provider, &|rel| {
        crate::extension::extensions::read_extension_file(project, &extension.path, rel)
    })
    .ok();
    let data = declared
        .as_ref()
        .and_then(|d| {
            d.capabilities
                .iter()
                .find(|c| c.capability == "agent_harness")
        })
        .and_then(|c| {
            serde_json::from_value::<oxplow_domain::agent::harness::HarnessData>(c.data.clone())
                .ok()
        });
    let title = declared
        .as_ref()
        .map(|d| d.provider.name.as_str())
        .unwrap_or(instance);
    let mut fields = vec![(
        "enabled".to_owned(),
        format!("Enable {title}"),
        "Allow new sessions with this agent".to_owned(),
        serde_json::json!({"type":"boolean"}),
        Some(serde_json::json!(true)),
    )];
    for setting in data.into_iter().flat_map(|d| d.settings) {
        fields.push((
            setting.key,
            format!("{title} {}", setting.title),
            setting.hint,
            setting.schema,
            None,
        ));
    }
    fields
        .into_iter()
        .map(|(field, title, description, schema, default)| {
            oxplow_config::settings::SettingDefinition {
                key: format!(
                    "oxplow.agents.{}.{}",
                    instance.replace('/', "."),
                    oxplow_config::settings::key_segment(&field)
                ),
                title,
                description,
                category: "agents".into(),
                schema,
                default,
                secret: false,
                order: 0,
                ui: Default::default(),
                mask: Default::default(),
                owner: "core".into(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    fn register(registry: &HarnessRegistry, declared: &[Implementation]) {
        prepare_built_ins(declared).publish(registry);
    }

    use super::*;

    fn declared(capability: &str, id: &str, entry: &'static str) -> Implementation {
        Implementation {
            capability: capability.into(),
            id: id.into(),
            title: id.into(),
            extension: Some("oxplow-foundation".into()),
            source: Source::BuiltIn(entry),
            features: serde_json::json!({}),
            offered: true,
            icon: None,
            config: serde_json::json!({}),
            description: None,
        }
    }

    fn no_capabilities() -> crate::capabilities::CapabilityRegistry {
        crate::capabilities::CapabilityRegistry::new(
            Vec::new(),
            oxplow_domain::vocabulary::VocabularyHandle::core(),
        )
    }

    /// A harness's declared features ride its listing, so the UI offers
    /// what its sessions can do (choosing another conversation).
    #[test]
    fn the_listing_carries_each_harnesss_features() {
        let r = HarnessRegistry::new();
        let claude = declared("agent_harness", "claude", "oxplow:claude-code");
        register(&r, std::slice::from_ref(&claude));
        let capabilities = no_capabilities();
        capabilities.set_declared(vec![Implementation {
            features: serde_json::json!({ "resume": true, "conversation_list": true, "transcript": false }),
            ..claude
        }]);
        let mut features = listing(&r, &capabilities, &Default::default())[0]
            .features
            .clone();
        features.sort();
        assert_eq!(features, ["conversation_list", "resume"]);
        assert!(listing(&r, &no_capabilities(), &Default::default())[0]
            .features
            .is_empty());
    }

    /// A harness's declared logo rides its listing, as the data URI it
    /// was declared as; one that declares none lists none.
    #[test]
    fn the_listing_carries_each_harnesss_logo() {
        let r = HarnessRegistry::new();
        let claude = declared("agent_harness", "claude", "oxplow:claude-code");
        let codex = declared("agent_harness", "codex", "oxplow:codex-cli");
        register(&r, &[claude.clone(), codex.clone()]);
        let capabilities = no_capabilities();
        capabilities.set_declared(vec![
            Implementation {
                icon: Some("data:image/svg+xml;base64,PHN2Zy8+".into()),
                ..claude
            },
            codex,
        ]);
        let listed = listing(&r, &capabilities, &Default::default());
        let icon = |id: &str| listed.iter().find(|h| h.id == id).unwrap().icon.clone();
        assert_eq!(
            icon("claude").as_deref(),
            Some("data:image/svg+xml;base64,PHN2Zy8+")
        );
        assert_eq!(icon("codex"), None);
    }

    /// The foundation extension ships a mark for each built-in agent, and
    /// the running registry lists it.
    #[tokio::test]
    async fn the_built_in_agents_ship_a_logo() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let listed = listing(&fx.svc.harnesses, &fx.svc.capabilities, &Default::default());
        for id in ["claude", "codex", "opencode"] {
            let icon = listed
                .iter()
                .find(|h| h.id == id)
                .and_then(|h| h.icon.clone())
                .unwrap_or_else(|| panic!("{id} lists no logo"));
            assert!(
                icon.starts_with("data:image/svg+xml;base64,"),
                "{id}: {icon}"
            );
        }
    }

    /// What's declared is registered under its key; what no longer is goes.
    #[test]
    fn the_declared_harnesses_are_registered() {
        let r = HarnessRegistry::new();
        register(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "codex", "oxplow:codex-cli"),
            ],
        );
        assert_eq!(r.names(), ["claude", "codex"]);
        register(
            &r,
            &[declared("agent_harness", "codex", "oxplow:codex-cli")],
        );
        assert_eq!(r.names(), ["codex"]);
    }

    /// The listing is the registry's, the enabled ones first in `agents:`
    /// order; with no `agents:` every harness is enabled, as declared.
    #[test]
    fn the_listing_says_which_are_enabled() {
        let r = HarnessRegistry::new();
        register(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "codex", "oxplow:codex-cli"),
                declared("agent_harness", "opencode", "oxplow:opencode"),
            ],
        );
        let rows = |agents: &[&str]| {
            let agents: Vec<String> = agents.iter().map(|a| a.to_string()).collect();
            listing(
                &r,
                &no_capabilities(),
                &r.names()
                    .into_iter()
                    .map(|id| {
                        let on = agents.contains(&id);
                        (id, on)
                    })
                    .collect(),
            )
            .into_iter()
            .map(|h| (h.id, h.enabled))
            .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(&[]),
            [
                ("claude".into(), false),
                ("codex".into(), false),
                ("opencode".into(), false)
            ]
        );
        assert_eq!(
            rows(&["opencode"]),
            [
                ("claude".into(), false),
                ("codex".into(), false),
                ("opencode".into(), true)
            ]
        );
    }

    /// A harness's settings ride its listing: opencode's model, Claude's MCP
    /// servers.
    #[test]
    fn the_listing_carries_each_harnesss_settings() {
        let r = HarnessRegistry::new();
        register(
            &r,
            &[
                declared("agent_harness", "claude", "oxplow:claude-code"),
                declared("agent_harness", "opencode", "oxplow:opencode"),
            ],
        );
        let rows = listing(&r, &no_capabilities(), &Default::default());
        let keys = |id: &str| -> Vec<String> {
            rows.iter()
                .find(|h| h.id == id)
                .unwrap()
                .settings
                .iter()
                .map(|s| s.key.clone())
                .collect()
        };
        assert_eq!(keys("opencode"), ["model"]);
        assert_eq!(keys("claude"), ["mcpServers", "model", "effort"]);
    }
}
