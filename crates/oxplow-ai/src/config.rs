//! `ai.yaml`: the providers you've connected and which model each role
//! uses. User-global (in oxplow's global config dir); a project can
//! override role bindings. Keys are NOT here; they live in the keychain
//! under the provider id.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// File name inside the global config dir.
pub const AI_CONFIG_FILE: &str = "ai.yaml";

/// The jobs oxplow itself gives models. Every extension can name them,
/// and each extension AI profile falls back to one.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinRole {
    Main,
    Summarize,
    Decide,
}

impl BuiltinRole {
    pub const ALL: [BuiltinRole; 3] = [
        BuiltinRole::Main,
        BuiltinRole::Summarize,
        BuiltinRole::Decide,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BuiltinRole::Main => "main",
            BuiltinRole::Summarize => "summarize",
            BuiltinRole::Decide => "decide",
        }
    }

    pub fn from_name(name: &str) -> Option<BuiltinRole> {
        BuiltinRole::ALL.into_iter().find(|r| r.name() == name)
    }

    /// What oxplow uses it for, as Settings → AI shows it.
    pub fn description(self) -> &'static str {
        match self {
            BuiltinRole::Main => {
                "General reasoning: extracting structured answers, such as an effort's inferred \
                 decisions"
            }
            BuiltinRole::Summarize => "Summaries of sessions, efforts and changes",
            BuiltinRole::Decide => "Typed yes/no, choice and score questions (e.g. Jev)",
        }
    }
}

/// What a model call is for: a built-in role, or an AI profile an
/// extension declares (`<extension>/<name>`). Extensions and features
/// name roles, never models; a person binds each to a provider + model.
/// Serialized as its name (`summarize`, `acme/review`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Builtin(BuiltinRole),
    Profile { extension: String, name: String },
}

/// Roles oxplow dropped because nothing asked them: a config naming one
/// fails saying so rather than as an unknown role.
pub const DROPPED_ROLES: [&str; 3] = ["fast", "embed", "review"];

impl Role {
    pub const MAIN: Role = Role::Builtin(BuiltinRole::Main);
    pub const SUMMARIZE: Role = Role::Builtin(BuiltinRole::Summarize);
    pub const DECIDE: Role = Role::Builtin(BuiltinRole::Decide);

    /// A role as a person writes it in `ai.yaml` or `project.yaml`: a
    /// built-in name or `<extension>/<profile>`.
    pub fn parse(text: &str) -> Result<Role, String> {
        if let Some(role) = BuiltinRole::from_name(text) {
            return Ok(Role::Builtin(role));
        }
        if DROPPED_ROLES.contains(&text) {
            return Err(format!(
                "oxplow has no `{text}` role: nothing asked it; remove it (roles: main, \
                 summarize, decide, or an extension's AI profile as `<extension>/<name>`)"
            ));
        }
        match text.split_once('/') {
            Some((extension, name)) if is_extension_name(extension) && is_profile_name(name) => {
                Ok(Role::Profile {
                    extension: extension.to_string(),
                    name: name.to_string(),
                })
            }
            _ => Err(format!(
                "unknown role `{text}` (roles: main, summarize, decide, or an extension's AI \
                 profile as `<extension>/<name>`)"
            )),
        }
    }

    /// A role as an extension's own manifest or script names it: a
    /// built-in name, or the bare name of one of `extension`'s profiles.
    /// Profiles aren't shared, so a qualified name is refused.
    pub fn parse_in(extension: &str, text: &str) -> Result<Role, String> {
        if let Some(role) = BuiltinRole::from_name(text) {
            return Ok(Role::Builtin(role));
        }
        if is_profile_name(text) {
            return Ok(Role::Profile {
                extension: extension.to_string(),
                name: text.to_string(),
            });
        }
        Err(format!(
            "`{text}` isn't a role: name `main`, `summarize`, `decide` or one of this \
             extension's `ai_profiles` by its id"
        ))
    }

    pub fn builtin(&self) -> Option<BuiltinRole> {
        match self {
            Role::Builtin(role) => Some(*role),
            Role::Profile { .. } => None,
        }
    }
}

/// A profile id: `[a-z][a-z0-9_]*`.
pub fn is_profile_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The shape of an extension name in a qualified role: no spaces or
/// slashes. Whether one is installed is known only at run time.
fn is_extension_name(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Builtin(role) => f.write_str(role.name()),
            Role::Profile { extension, name } => write!(f, "{extension}/{name}"),
        }
    }
}

impl From<BuiltinRole> for Role {
    fn from(role: BuiltinRole) -> Role {
        Role::Builtin(role)
    }
}

impl Serialize for Role {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Role::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

impl specta::Type for Role {
    fn definition(types: &mut specta::Types) -> specta::datatype::DataType {
        <String as specta::Type>::definition(types)
    }
}

/// An AI profile as an active extension declares it: what the service
/// resolves a profile role against.
#[derive(Debug, Clone, PartialEq)]
pub struct AiProfile {
    pub extension: String,
    pub name: String,
    pub description: String,
    /// The built-in role whose binding it uses until a person binds it.
    pub fallback: BuiltinRole,
    /// What the bound model must support.
    pub features: Vec<crate::capabilities::ModelFeature>,
}

impl AiProfile {
    pub fn role(&self) -> Role {
        Role::Profile {
            extension: self.extension.clone(),
            name: self.name.clone(),
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
                    "role `{role}` uses AI provider `{}`, which isn't configured",
                    b.provider
                )));
            }
            if b.model.trim().is_empty() {
                return Err(ConfigError::Invalid(format!("role `{role}` needs a model")));
            }
        }
        Ok(())
    }

    /// Role bindings with `overrides` (from a project) layered on top.
    pub fn with_overrides(&self, overrides: &BTreeMap<Role, RoleBinding>) -> AiConfig {
        let mut out = self.clone();
        for (role, b) in overrides {
            out.roles.insert(role.clone(), b.clone());
        }
        out
    }

    /// The provider and binding `role` uses, if it's configured: its own
    /// binding, else — for a profile — its fallback's. A profile none of
    /// `profiles` declares resolves to nothing, bound or not.
    pub fn resolve(
        &self,
        role: &Role,
        profiles: &[AiProfile],
    ) -> Option<(&ProviderConfig, &RoleBinding)> {
        let binding = match role {
            Role::Builtin(_) => self.roles.get(role)?,
            Role::Profile { .. } => {
                let profile = profiles.iter().find(|p| &p.role() == role)?;
                self.roles
                    .get(role)
                    .or_else(|| self.roles.get(&Role::Builtin(profile.fallback)))?
            }
        };
        let provider = self.providers.iter().find(|p| p.id == binding.provider)?;
        Some((provider, binding))
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
        let (p, b) = c.resolve(&Role::SUMMARIZE, &[]).unwrap();
        assert_eq!(p.kind, "openai_compatible");
        assert_eq!(b.model, "qwen3:14b");
        assert!(AiConfig::default().resolve(&Role::MAIN, &[]).is_none());
    }

    #[test]
    fn project_overrides_win() {
        let mut o = BTreeMap::new();
        o.insert(
            Role::MAIN,
            RoleBinding {
                provider: "local".into(),
                model: "llama".into(),
            },
        );
        let c = cfg().with_overrides(&o);
        assert_eq!(c.resolve(&Role::MAIN, &[]).unwrap().1.model, "llama");
        assert_eq!(
            c.resolve(&Role::DECIDE, &[]).unwrap().1.model,
            "jev-latest",
            "others untouched"
        );
    }

    #[test]
    fn validation_catches_unusable_config() {
        cfg().validate().unwrap();
        let mut bad = cfg();
        bad.roles.insert(
            Role::MAIN,
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

    fn review() -> AiProfile {
        AiProfile {
            extension: "acme".into(),
            name: "review".into(),
            description: "Reviews things.".into(),
            fallback: BuiltinRole::Summarize,
            features: vec![],
        }
    }

    /// A profile uses its own binding when a person made one, else its
    /// fallback's; one no active extension declares resolves to nothing.
    #[test]
    fn a_profile_resolves_to_its_binding_else_its_fallbacks() {
        let role = Role::parse("acme/review").unwrap();
        let base = cfg();
        let (_, b) = base.resolve(&role, &[review()]).unwrap();
        assert_eq!(b.model, "qwen3:14b", "the summarize binding");
        let mut bound = cfg();
        bound.roles.insert(
            role.clone(),
            RoleBinding {
                provider: "anthropic".into(),
                model: "claude-opus-5-5".into(),
            },
        );
        assert_eq!(
            bound.resolve(&role, &[review()]).unwrap().1.model,
            "claude-opus-5-5"
        );
        assert!(bound.resolve(&role, &[]).is_none(), "undeclared");
    }

    #[test]
    fn roles_are_written_as_names() {
        let role = Role::parse("acme/review").unwrap();
        assert_eq!(serde_json::to_value(&role).unwrap(), "acme/review");
        assert_eq!(serde_json::to_value(Role::DECIDE).unwrap(), "decide");
        assert_eq!(
            serde_yaml::from_str::<Role>("acme/review").unwrap(),
            Role::Profile {
                extension: "acme".into(),
                name: "review".into()
            }
        );
        assert!(Role::parse("acme/Review").is_err());
        assert!(Role::parse("nope")
            .unwrap_err()
            .contains("unknown role `nope`"));
        assert_eq!(Role::parse_in("acme", "review").unwrap(), role);
        assert_eq!(Role::parse_in("acme", "main").unwrap(), Role::MAIN);
        assert!(
            Role::parse_in("acme", "other/review").is_err(),
            "not shared"
        );
    }

    #[test]
    fn ai_yaml_binds_a_profile_by_its_qualified_name() {
        let c: AiConfig = serde_yaml::from_str(
            "providers: [{ id: p, kind: openai }]\nroles:\n  acme/review: { provider: p, model: m }\n",
        )
        .unwrap();
        c.validate().unwrap();
        assert!(c.roles.contains_key(&Role::parse("acme/review").unwrap()));
    }
}
