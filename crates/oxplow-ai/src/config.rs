//! `ai.yaml`: the providers you've connected and which model each role
//! uses. User-global (in oxplow's global config dir); a project can
//! override role bindings. Keys are NOT here; they live in the keychain
//! under the provider id.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// File name inside the global config dir.
pub const AI_CONFIG_FILE: &str = "ai.yaml";

/// Jobs oxplow gives models. Extensions and features refer to roles,
/// never to models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Main,
    Summarize,
    Decide,
}

impl Role {
    pub const ALL: [Role; 3] = [Role::Main, Role::Summarize, Role::Decide];
}

/// Roles oxplow dropped because nothing asked them: a config naming one
/// fails saying so rather than with a bare unknown variant.
pub const DROPPED_ROLES: [&str; 3] = ["fast", "embed", "review"];

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        match name.as_str() {
            "main" => Ok(Role::Main),
            "summarize" => Ok(Role::Summarize),
            "decide" => Ok(Role::Decide),
            other if DROPPED_ROLES.contains(&other) => Err(serde::de::Error::custom(format!(
                "oxplow has no `{other}` role: nothing asked it; remove it (roles: main, \
                 summarize, decide)"
            ))),
            other => Err(serde::de::Error::custom(format!(
                "unknown role `{other}` (roles: main, summarize, decide)"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfig {
    /// Your name for it, e.g. `anthropic` or `local-ollama`. Also the
    /// keychain entry name for its key.
    pub id: String,
    /// What it is: a declared `ai_provider` id (`anthropic`, `openai`,
    /// `openai_compatible`, `openrouter`, `typesafe`).
    pub kind: String,
    /// Its API base: required for a kind with no default
    /// (`openai_compatible`), an override for the others.
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleBinding {
    /// A provider `id`.
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiConfig {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub roles: BTreeMap<Role, RoleBinding>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigError {
    #[error("{0}")]
    Invalid(String),
    #[error("ai.yaml: {0}")]
    Io(String),
}

impl AiConfig {
    /// Load from `dir/ai.yaml`; a missing file is an empty config.
    pub fn load(dir: &Path) -> Result<AiConfig, ConfigError> {
        match std::fs::read_to_string(dir.join(AI_CONFIG_FILE)) {
            Ok(text) => serde_yaml::from_str(&text)
                .map_err(|e| ConfigError::Invalid(format!("ai.yaml: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AiConfig::default()),
            Err(e) => Err(ConfigError::Io(e.to_string())),
        }
    }

    /// Validate and write to `dir/ai.yaml`.
    pub fn save(&self, dir: &Path) -> Result<(), ConfigError> {
        self.validate()?;
        let text = serde_yaml::to_string(self).map_err(|e| ConfigError::Io(e.to_string()))?;
        std::fs::create_dir_all(dir).map_err(|e| ConfigError::Io(e.to_string()))?;
        std::fs::write(dir.join(AI_CONFIG_FILE), text).map_err(|e| ConfigError::Io(e.to_string()))
    }

    /// Problems that would make a role unusable, as messages. Whether each
    /// kind is registered, and needs a `baseUrl`, is the service's to say
    /// (`ai_service`): it knows the providers.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut seen = std::collections::HashSet::new();
        for p in &self.providers {
            if p.id.trim().is_empty() {
                return Err(ConfigError::Invalid("an AI provider needs an id".into()));
            }
            if !seen.insert(p.id.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "AI provider `{}` is defined twice",
                    p.id
                )));
            }
        }
        for (role, b) in &self.roles {
            if !seen.contains(b.provider.as_str()) {
                return Err(ConfigError::Invalid(format!(
                    "role `{role:?}` uses AI provider `{}`, which isn't configured",
                    b.provider
                )));
            }
            if b.model.trim().is_empty() {
                return Err(ConfigError::Invalid(format!(
                    "role `{role:?}` needs a model"
                )));
            }
        }
        Ok(())
    }

    /// Role bindings with `overrides` (from a project) layered on top.
    pub fn with_overrides(&self, overrides: &BTreeMap<Role, RoleBinding>) -> AiConfig {
        let mut out = self.clone();
        for (role, b) in overrides {
            out.roles.insert(*role, b.clone());
        }
        out
    }

    /// The provider and binding a role uses, if it's configured.
    pub fn resolve(&self, role: Role) -> Option<(&ProviderConfig, &RoleBinding)> {
        let b = self.roles.get(&role)?;
        let p = self.providers.iter().find(|p| p.id == b.provider)?;
        Some((p, b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
providers:
  - { id: anthropic, kind: anthropic }
  - { id: local, kind: openai_compatible, baseUrl: "http://localhost:11434/v1" }
  - { id: jev, kind: typesafe }
roles:
  main: { provider: anthropic, model: claude-opus-5-5 }
  summarize: { provider: local, model: qwen3:14b }
  decide: { provider: jev, model: jev-latest }
"#;

    fn cfg() -> AiConfig {
        serde_yaml::from_str(YAML).unwrap()
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            AiConfig::load(dir.path()).unwrap(),
            AiConfig::default(),
            "missing file = empty"
        );
        cfg().save(dir.path()).unwrap();
        assert_eq!(AiConfig::load(dir.path()).unwrap(), cfg());
    }

    /// A role nothing asked is gone: an `ai.yaml` naming one fails saying
    /// so, not with a bare unknown variant.
    #[test]
    fn a_role_oxplow_dropped_fails_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(AI_CONFIG_FILE),
            "providers: [{ id: p, kind: openai }]\nroles:\n  fast: { provider: p, model: m }\n",
        )
        .unwrap();
        let err = AiConfig::load(dir.path()).unwrap_err().to_string();
        assert!(err.contains("oxplow has no `fast` role"), "{err}");
        assert!(!err.contains("unknown variant"), "{err}");
    }

    #[test]
    fn resolves_roles_to_their_provider() {
        let c = cfg();
        let (p, b) = c.resolve(Role::Summarize).unwrap();
        assert_eq!(p.kind, "openai_compatible");
        assert_eq!(b.model, "qwen3:14b");
        assert!(AiConfig::default().resolve(Role::Main).is_none());
    }

    #[test]
    fn project_overrides_win() {
        let mut o = BTreeMap::new();
        o.insert(
            Role::Main,
            RoleBinding {
                provider: "local".into(),
                model: "llama".into(),
            },
        );
        let c = cfg().with_overrides(&o);
        assert_eq!(c.resolve(Role::Main).unwrap().1.model, "llama");
        assert_eq!(
            c.resolve(Role::Decide).unwrap().1.model,
            "jev-latest",
            "others untouched"
        );
    }

    #[test]
    fn validation_catches_unusable_config() {
        cfg().validate().unwrap();
        let mut bad = cfg();
        bad.roles.insert(
            Role::Main,
            RoleBinding {
                provider: "nope".into(),
                model: "m".into(),
            },
        );
        assert!(
            matches!(bad.validate(), Err(ConfigError::Invalid(m)) if m.contains("uses AI provider `nope`"))
        );
        let mut bad = cfg();
        bad.providers.push(ProviderConfig {
            id: "anthropic".into(),
            kind: "openai".into(),
            base_url: None,
        });
        assert!(
            matches!(bad.validate(), Err(ConfigError::Invalid(m)) if m.contains("AI provider `anthropic` is defined twice"))
        );
        assert!(cfg().save(tempfile::tempdir().unwrap().path()).is_ok());
        assert!(
            bad.save(tempfile::tempdir().unwrap().path()).is_err(),
            "never writes an invalid config"
        );
    }
}
