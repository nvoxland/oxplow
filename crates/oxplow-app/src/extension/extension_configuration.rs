//! Private configuration services: host-derived ownership, validated effective
//! layers and atomic override updates. Settings data never conveys authority.

use std::{path::PathBuf, sync::Arc};

use oxplow_db::extension_config_store::{self, ConfigNamespace, ConfigSnapshot, ConfigStoreError};
use oxplow_domain::{
    extension_state::{Scope, Stamp},
    run::{Mode, RunContext},
};
use rusqlite::Connection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    commands::TxCtx,
    extension::extension_catalog::ExtensionCatalog,
    extension::extension_state::{StateHost, StateHostError},
    extension::extensions::{config::ResolvedConfig, config_schema::ConfigurationFile},
};

#[derive(Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {}

#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayerUpdate {
    pub expected: Option<Stamp>,
    pub values: Value,
}

/// A single override or both layers during an explicit schema migration.
/// The host fixes the extension; no credential/authority selector is accepted.
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteInput {
    pub schema_version: u32,
    pub project: Option<LayerUpdate>,
    pub personal: Option<LayerUpdate>,
}

#[derive(Clone, Serialize)]
pub struct Stamps {
    pub schema_version: u32,
    pub project: Option<Stamp>,
    pub personal: Option<Stamp>,
    /// Which stored layers are at a schema version other than the declared
    /// one: reads of them fail closed until activation migrates them (or an
    /// explicit write replaces them).
    pub needs_migration: NeedsMigration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct NeedsMigration {
    pub project: bool,
    pub personal: bool,
}

impl Stamps {
    fn of(schema_version: u32, project: Option<Stamp>, personal: Option<Stamp>) -> Self {
        let behind = |stamp: &Option<Stamp>| {
            stamp.is_some_and(|stamp| stamp.schema_version != schema_version)
        };
        Self {
            schema_version,
            needs_migration: NeedsMigration {
                project: behind(&project),
                personal: behind(&personal),
            },
            project,
            personal,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct ConfigurationSnapshot {
    #[serde(flatten)]
    pub stamps: Stamps,
    #[serde(flatten)]
    pub resolved: ResolvedConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigurationError {
    #[error("extension configuration access denied")]
    Denied,
    #[error("the owning extension has no configuration declaration")]
    Undeclared,
    #[error("extension configuration requires an explicit schema migration")]
    VersionMismatch,
    #[error("invalid extension configuration")]
    Invalid,
    #[error(transparent)]
    Store(#[from] ConfigStoreError),
}

#[derive(Clone)]
pub struct ConfigurationHost {
    owner: StateHost,
}

#[derive(Clone)]
pub struct ConfigurationSource {
    host: ConfigurationHost,
    db: oxplow_db::Database,
}

impl ConfigurationSource {
    pub fn new(host: ConfigurationHost, db: oxplow_db::Database) -> Self {
        Self { host, db }
    }

    pub async fn snapshot(
        &self,
        run: RunContext,
    ) -> Result<ConfigurationSnapshot, ConfigurationError> {
        let host = self.host.clone();
        self.db
            .read_or(move |conn| {
                host.snapshot(conn, &run).map_err(|error| match error {
                    ConfigurationError::Store(ConfigStoreError::Storage(storage)) => {
                        oxplow_db::TxError::Storage(storage)
                    }
                    error => oxplow_db::TxError::Aborted(error),
                })
            })
            .await
            .map_err(|error| match error {
                oxplow_db::TxError::Storage(storage) => {
                    ConfigurationError::Store(ConfigStoreError::Storage(storage))
                }
                oxplow_db::TxError::Aborted(error) => error,
            })
    }
}

impl ConfigurationHost {
    pub fn new(catalog: Arc<ExtensionCatalog>, project: PathBuf) -> Self {
        Self {
            owner: StateHost::new(catalog, project),
        }
    }

    pub fn snapshot(
        &self,
        conn: &Connection,
        run: &RunContext,
    ) -> Result<ConfigurationSnapshot, ConfigurationError> {
        let (extension, declaration) = self.declaration(run, "extension_config.read")?;
        let (project, personal) = layers(conn, &extension)?;
        let resolved = resolve(
            &extension,
            &declaration,
            project.as_ref(),
            personal.as_ref(),
        )?;
        self.declaration(run, "extension_config.read")?;
        run.trace().count("extension_config.read");
        run.used_scope("extension_config.read");
        Ok(ConfigurationSnapshot {
            stamps: stamps(declaration.version, &project, &personal),
            resolved,
        })
    }

    /// Metadata survives a schema change so callers can submit a fenced,
    /// explicit migration without receiving invalid effective settings.
    pub fn inspect(
        &self,
        conn: &Connection,
        run: &RunContext,
    ) -> Result<Stamps, ConfigurationError> {
        let (extension, declaration) = self.declaration(run, "extension_config.read")?;
        let project = extension_config_store::inspect_tx(
            conn,
            &ConfigNamespace::new(extension.clone(), Scope::Project)?,
        )?;
        let personal = extension_config_store::inspect_tx(
            conn,
            &ConfigNamespace::new(extension, Scope::Personal)?,
        )?;
        self.declaration(run, "extension_config.read")?;
        run.trace().count("extension_config.read");
        run.used_scope("extension_config.read");
        Ok(Stamps::of(declaration.version, project, personal))
    }

    /// Validate the complete candidate before mutating either layer. Both CAS
    /// changes and the command audit commit together; no external code runs.
    pub fn compare_exchange(
        &self,
        ctx: &TxCtx<'_>,
        input: &WriteInput,
    ) -> Result<Stamps, ConfigurationError> {
        if ctx.run.mode() == Mode::Pure || ctx.conn.is_autocommit() || ctx.events.cause.is_none() {
            return Err(ConfigurationError::Denied);
        }
        let (extension, declaration) = self.declaration(ctx.run, "extension_config.write")?;
        if input.schema_version != declaration.version
            || (input.project.is_none() && input.personal.is_none())
        {
            return Err(ConfigurationError::Invalid);
        }
        let (mut project, mut personal) = layers(ctx.conn, &extension)?;
        // Untouched layers must already speak the target schema. A migration
        // can replace both old layers in the same audited transaction.
        let candidate_project = candidate(&project, input.project.as_ref(), declaration.version)?;
        let candidate_personal =
            candidate(&personal, input.personal.as_ref(), declaration.version)?;
        declaration
            .resolve(&extension, candidate_project, candidate_personal)
            .map_err(|_| ConfigurationError::Invalid)?;
        ctx.conn
            .execute_batch("SAVEPOINT extension_configuration_host")
            .map_err(storage)?;
        let result = (|| {
            for (scope, update, stored) in [
                (Scope::Project, input.project.as_ref(), &mut project),
                (Scope::Personal, input.personal.as_ref(), &mut personal),
            ] {
                if let Some(update) = update {
                    let namespace = ConfigNamespace::new(extension.clone(), scope)?;
                    *stored = Some(extension_config_store::compare_exchange_tx(
                        ctx.conn,
                        &namespace,
                        update.expected,
                        input.schema_version,
                        &update.values,
                    )?);
                }
            }
            self.declaration(ctx.run, "extension_config.write")?;
            ctx.conn
                .execute_batch("RELEASE extension_configuration_host")
                .map_err(storage)?;
            Ok(stamps(declaration.version, &project, &personal))
        })();
        match result {
            Ok(stamps) => {
                ctx.trace.count("extension_config.write");
                ctx.run.used_scope("extension_config.write");
                Ok(stamps)
            }
            Err(error) => {
                ctx.conn.execute_batch("ROLLBACK TO extension_configuration_host; RELEASE extension_configuration_host").map_err(storage)?;
                Err(error)
            }
        }
    }

    fn declaration(
        &self,
        run: &RunContext,
        service: &str,
    ) -> Result<(String, ConfigurationFile), ConfigurationError> {
        let (extension, declaration) = self
            .owner
            .with_owner(run, service, |extension| {
                (extension.name.clone(), extension.configuration.clone())
            })
            .map_err(|error| match error {
                StateHostError::Denied(_) => ConfigurationError::Denied,
                StateHostError::State(_) => ConfigurationError::Invalid,
            })?;
        Ok((
            extension,
            declaration.ok_or(ConfigurationError::Undeclared)?,
        ))
    }
}

/// An extension's effective configuration as stored now — its declared defaults
/// under the project and personal layers — which the SQL that reads it with
/// `config()` is compiled and run against.
pub(crate) fn effective_values(
    conn: &Connection,
    extension: &str,
    declaration: &ConfigurationFile,
) -> Result<Value, ConfigurationError> {
    let (project, personal) = layers(conn, extension)?;
    Ok(resolve(extension, declaration, project.as_ref(), personal.as_ref())?.values)
}

fn layers(
    conn: &Connection,
    extension: &str,
) -> Result<(Option<ConfigSnapshot>, Option<ConfigSnapshot>), ConfigurationError> {
    Ok((
        extension_config_store::read_tx(
            conn,
            &ConfigNamespace::new(extension.into(), Scope::Project)?,
        )?,
        extension_config_store::read_tx(
            conn,
            &ConfigNamespace::new(extension.into(), Scope::Personal)?,
        )?,
    ))
}

fn candidate<'a>(
    stored: &'a Option<ConfigSnapshot>,
    update: Option<&'a LayerUpdate>,
    version: u32,
) -> Result<Option<&'a Value>, ConfigurationError> {
    match update {
        Some(update) => {
            if update.expected != stored.as_ref().map(ConfigSnapshot::stamp) {
                return Err(ConfigStoreError::Conflict {
                    actual: stored.as_ref().map(ConfigSnapshot::stamp),
                }
                .into());
            }
            Ok(Some(&update.values))
        }
        None => {
            check_version(stored.as_ref(), version)?;
            Ok(stored.as_ref().map(ConfigSnapshot::values))
        }
    }
}

fn check_version(stored: Option<&ConfigSnapshot>, version: u32) -> Result<(), ConfigurationError> {
    if stored.is_some_and(|stored| stored.stamp().schema_version != version) {
        return Err(ConfigurationError::VersionMismatch);
    }
    Ok(())
}

fn resolve(
    extension: &str,
    declaration: &ConfigurationFile,
    project: Option<&ConfigSnapshot>,
    personal: Option<&ConfigSnapshot>,
) -> Result<ResolvedConfig, ConfigurationError> {
    check_version(project, declaration.version)?;
    check_version(personal, declaration.version)?;
    declaration
        .resolve(
            extension,
            project.map(ConfigSnapshot::values),
            personal.map(ConfigSnapshot::values),
        )
        .map_err(|_| ConfigurationError::Invalid)
}

fn stamps(
    schema_version: u32,
    project: &Option<ConfigSnapshot>,
    personal: &Option<ConfigSnapshot>,
) -> Stamps {
    Stamps::of(
        schema_version,
        project.as_ref().map(ConfigSnapshot::stamp),
        personal.as_ref().map(ConfigSnapshot::stamp),
    )
}

fn storage(error: rusqlite::Error) -> ConfigurationError {
    ConfigurationError::Store(ConfigStoreError::Storage(oxplow_db::map_sql_err(error)))
}
