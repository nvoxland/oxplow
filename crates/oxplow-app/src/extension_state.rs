//! Private state services over a host-owned project and execution context.
//! Namespace identity is never part of an extension's request. Writes compose
//! with the command bus transaction; no script or external wait runs here.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_domain::extension_state::{Area, Namespace, Scope, Snapshot, Stamp, StateError};
use oxplow_domain::grant::{Decision, Need};
use oxplow_domain::run::{Mode, RunContext};
use rusqlite::Connection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::TxCtx;
use crate::extension_catalog::ExtensionCatalog;

/// Only storage scope/area are selectable. Both remain in this host's project.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadInput {
    pub scope: Scope,
    pub area: Area,
}

/// Whole-namespace update, with no extension, path, SQL or authority fields.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteInput {
    pub scope: Scope,
    pub area: Area,
    pub expected: Option<Stamp>,
    pub schema_version: u32,
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum StateHostError {
    #[error("extension state access denied: {0}")]
    Denied(&'static str),
    #[error(transparent)]
    State(#[from] StateError),
}

/// Backend service bound to the primary project. It supplies no raw RPC surface.
/// Callers provide their already-owned connection, never open another transaction.
#[derive(Clone)]
pub struct StateHost {
    catalog: Arc<ExtensionCatalog>,
    project: PathBuf,
}

/// State reads for hosts that release the database between script calls.
/// This is separate from SQL: it exposes only the host-derived namespace.
#[derive(Clone)]
pub struct StateSource {
    host: StateHost,
    db: oxplow_db::Database,
}

impl StateSource {
    pub fn new(host: StateHost, db: oxplow_db::Database) -> Self {
        Self { host, db }
    }

    pub async fn snapshot(
        &self,
        run: RunContext,
        input: ReadInput,
    ) -> Result<Option<Snapshot>, StateHostError> {
        let host = self.host.clone();
        self.db
            .read_or(move |conn| {
                host.snapshot(conn, &run, input)
                    .map_err(|error| match error {
                        StateHostError::State(StateError::Storage(storage)) => {
                            oxplow_db::TxError::Storage(storage)
                        }
                        other => oxplow_db::TxError::Aborted(other),
                    })
            })
            .await
            .map_err(|error| match error {
                oxplow_db::TxError::Storage(storage) => {
                    StateHostError::State(StateError::Storage(storage))
                }
                oxplow_db::TxError::Aborted(other) => other,
            })
    }
}

impl StateHost {
    pub fn new(catalog: Arc<ExtensionCatalog>, project: PathBuf) -> Self {
        let project = catalog.project_of(&project).to_path_buf();
        Self { catalog, project }
    }

    /// Read on the caller's snapshot, including a command composer's connection.
    pub fn snapshot(
        &self,
        conn: &Connection,
        run: &RunContext,
        input: ReadInput,
    ) -> Result<Option<Snapshot>, StateHostError> {
        let namespace = self.authorize(run, "state.read", input.scope, input.area)?;
        let snapshot = oxplow_db::extension_state_store::snapshot_tx(conn, &namespace)?;
        self.authorize(run, "state.read", input.scope, input.area)?;
        run.trace().count("state.read");
        run.used_scope("state.read");
        Ok(snapshot)
    }

    /// Trusted native handlers only: persist alongside their audit/domain writes.
    /// The returned values stay private; public command results should use only
    /// the stamp, and private inputs need audit/proposal handling before exposure.
    /// Propagate storage errors to abort the outer transaction, including a
    /// failure to release or roll back the publication savepoint.
    pub fn compare_exchange(
        &self,
        ctx: &TxCtx<'_>,
        input: &WriteInput,
    ) -> Result<Snapshot, StateHostError> {
        if ctx.run.mode() == Mode::Pure {
            return Err(StateHostError::Denied("a pure run cannot change state"));
        }
        if ctx.conn.is_autocommit() || ctx.events.cause.is_none() {
            return Err(StateHostError::Denied(
                "state changes require the caller's audited transaction",
            ));
        }
        let namespace = self.authorize(ctx.run, "state.write", input.scope, input.area)?;
        // If a live check rejects publication, leave no mutation even if the
        // trusted caller catches the error and continues its outer transaction.
        ctx.conn
            .execute_batch("SAVEPOINT extension_state_publication")
            .map_err(storage)?;
        let result = (|| {
            let snapshot = oxplow_db::extension_state_store::compare_exchange_tx(
                ctx.conn,
                &namespace,
                input.expected,
                input.schema_version,
                &input.values,
            )?;
            self.authorize(ctx.run, "state.write", input.scope, input.area)?;
            Ok(snapshot)
        })();
        match result {
            Ok(snapshot) => {
                ctx.conn
                    .execute_batch("RELEASE extension_state_publication")
                    .map_err(storage)?;
                ctx.trace.count("state.write");
                ctx.run.used_scope("state.write");
                Ok(snapshot)
            }
            Err(error) => {
                ctx.conn
                    .execute_batch(
                        "ROLLBACK TO extension_state_publication; RELEASE extension_state_publication",
                    )
                    .map_err(storage)?;
                Err(error)
            }
        }
    }

    fn authorize(
        &self,
        run: &RunContext,
        service: &str,
        scope: Scope,
        area: Area,
    ) -> Result<Namespace, StateHostError> {
        self.with_owner(run, service, |ext| Namespace {
            extension: ext.name.clone(),
            scope,
            area,
        })
    }

    /// Shared private-service ownership boundary. The callback reads captured
    /// declaration data only; it cannot accept an owner from service arguments.
    pub(crate) fn with_owner<T>(
        &self,
        run: &RunContext,
        service: &str,
        read: impl FnOnce(&crate::extensions::Extension) -> T,
    ) -> Result<T, StateHostError> {
        let owner = run
            .contribution_ref()
            .filter(|owner| owner.id.problem().is_none())
            .ok_or(StateHostError::Denied(
                "a valid contribution owner is required",
            ))?;
        if !run.declares_scope(service) {
            return Err(StateHostError::Denied("the service was not declared"));
        }
        if !matches!(run.decide(Need::Scope(service.into())), Decision::Allowed) {
            return Err(StateHostError::Denied("live run authority refused access"));
        }
        let snapshot = self.catalog.runtime_snapshot(&self.project);
        if owner.generation.as_deref() != Some(snapshot.generation()) {
            return Err(StateHostError::Denied(
                "the owning catalog generation is not current",
            ));
        }
        // A reserved bundled identity cannot be shadowed by a project package.
        let ext = snapshot
            .extensions()
            .iter()
            .find(|ext| ext.name == owner.id.extension && ext.origin == "bundled")
            .or_else(|| {
                snapshot
                    .extensions()
                    .iter()
                    .find(|ext| ext.name == owner.id.extension)
            })
            .filter(|ext| {
                ext.enabled
                    && ext.version == owner.version
                    && ext.contributions.iter().any(|item| item.id == owner.id)
            })
            .ok_or(StateHostError::Denied(
                "the contribution is not currently enabled",
            ))?;
        Ok(read(ext))
    }
}

fn storage(error: rusqlite::Error) -> StateHostError {
    StateError::Storage(oxplow_db::map_sql_err(error)).into()
}
