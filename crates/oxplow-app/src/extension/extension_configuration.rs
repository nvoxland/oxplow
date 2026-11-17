//! Authorized nested projections of the shared settings service.
use crate::{
    extension::extension_catalog::ExtensionCatalog, extension::extension_state::StateHost,
    extension::extensions::config_schema::ConfigurationFile,
};
use oxplow_domain::run::RunContext;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetInput {
    pub key: String,
    pub level: Option<oxplow_config::settings::Level>,
}
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("extension configuration access denied")]
    Denied,
    #[error("the owning extension has no configuration declaration")]
    Undeclared,
    #[error("invalid extension configuration")]
    Invalid,
}
#[derive(Clone)]
pub struct ConfigurationHost {
    owner: StateHost,
    project: PathBuf,
    global: PathBuf,
    secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
}
#[derive(Clone)]
pub struct ConfigurationSource {
    host: ConfigurationHost,
}
impl ConfigurationSource {
    pub fn new(host: ConfigurationHost, _db: oxplow_db::Database) -> Self {
        Self { host }
    }
    pub fn get(
        &self,
        run: &RunContext,
        input: GetInput,
    ) -> Result<oxplow_config::settings::SettingValue, ConfigurationError> {
        self.host.get(run, input)
    }
}
impl ConfigurationHost {
    pub fn new(catalog: Arc<ExtensionCatalog>, project: PathBuf) -> Self {
        let project = catalog.project_of(&project).to_path_buf();
        Self {
            owner: StateHost::new(catalog, project.clone()),
            global: oxplow_config::global_config_dir()
                .unwrap_or_else(|| project.join(".oxplow/global")),
            secrets: Arc::new(oxplow_ai::secrets::KeychainSecrets),
            project,
        }
    }
    pub fn with_settings(
        mut self,
        global: PathBuf,
        secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    ) -> Self {
        self.global = global;
        self.secrets = secrets;
        self
    }
    pub fn get(
        &self,
        run: &RunContext,
        input: GetInput,
    ) -> Result<oxplow_config::settings::SettingValue, ConfigurationError> {
        let extension = self
            .owner
            .with_owner(run, "config.read", Clone::clone)
            .map_err(|_| ConfigurationError::Denied)?;
        let namespace = extension.namespace.clone();
        let store = crate::settings::store(
            &self.project,
            &self.global,
            self.secrets.clone(),
            &[extension],
        )
        .map_err(|_| ConfigurationError::Invalid)?;
        {
            let registry = store
                .registry
                .read()
                .unwrap_or_else(|error| error.into_inner());
            let definition = registry
                .get(&input.key)
                .ok_or(ConfigurationError::Invalid)?;
            if definition.secret && definition.owner != format!("extension/{namespace}") {
                return Err(ConfigurationError::Denied);
            }
        }
        let setting = store
            .snapshot()
            .and_then(|snapshot| snapshot.get(&input.key, input.level))
            .map_err(|_| ConfigurationError::Invalid)?;
        self.owner
            .with_owner(run, "config.read", |_| ())
            .map_err(|_| ConfigurationError::Denied)?;
        run.used_scope("config.read");
        run.trace().count("config.read");
        if setting.secret {
            run.trace().count("config.secret");
        }
        Ok(setting)
    }
}
/// Public settings projection for SQL and conditions; secret keys are omitted.
pub(crate) fn effective_values(
    project: &Path,
    global: &Path,
    namespace: &str,
    declaration: &ConfigurationFile,
) -> Result<Value, ConfigurationError> {
    static PUBLIC_SECRETS: std::sync::OnceLock<Arc<dyn oxplow_ai::secrets::SecretStore>> =
        std::sync::OnceLock::new();
    let store = crate::settings::store_for_declaration(
        project,
        global,
        PUBLIC_SECRETS
            .get_or_init(|| Arc::new(oxplow_ai::secrets::MemorySecrets::default()))
            .clone(),
        namespace,
        declaration,
    )
    .map_err(|_| ConfigurationError::Invalid)?;
    super::extension_settings::values_for(namespace, declaration, &store, true)
        .map_err(|_| ConfigurationError::Invalid)
}
