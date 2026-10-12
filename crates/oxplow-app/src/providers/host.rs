//! Starting a provider: consent, the spawn and the handshake.
//!
//! Consent precedes execution: a provider runs only at the version a
//! person approved (`exec_consent`, [`ProgramKind::Provider`]) — its
//! extension folder's files, its entry, args, env, credentials and
//! network, and so its declarations file. The spawn mirrors an `exec`
//! source: a scrubbed environment (PATH, HOME, the declared `env` names,
//! the credentials from the keychain, `OXPLOW_*` context), the egress
//! proxy and `sandbox-exec` where the OS enforces `network`, and the
//! process ends with its connection: asked to `shutdown` first, killed
//! once its grace is up ([`Connection::shutdown`]). After the spawn, the
//! live `initialize` must equal the approved declarations.
//!
//! [`ProgramKind::Provider`]: crate::exec_consent::ProgramKind::Provider

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::{Access, Confirm};
use oxplow_provider_protocol::model::{
    method, InitializeParams, InitializeResult, Party, PROTOCOL_VERSION,
};
use oxplow_provider_protocol::{Incoming, Peer, ProtocolError};
use tokio::io::AsyncBufReadExt;

use super::spec::ProviderSpec;

/// How long a provider has to answer `initialize`.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a provider isn't running.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HostError {
    /// Nobody on this machine approved this version. Nothing ran.
    #[error(
        "provider `{0}` needs a person's approval first (Settings → Data → Programs). \
         Approval is per machine and per version of the provider and its declarations."
    )]
    Unapproved(String),
    /// It ran, but what it says it is isn't what was approved.
    #[error("provider `{name}` declares something other than its approved declarations: {detail}")]
    DeclarationsChanged { name: String, detail: String },
    /// It runs, but its `check` found problems with the instance's config.
    #[error("provider `{name}` isn't configured: {}", problems.iter().map(|p| format!("{} {}", if p.path.is_empty() { "/" } else { &p.path }, p.message)).collect::<Vec<_>>().join("; "))]
    Unconfigured {
        name: String,
        problems: Vec<oxplow_provider_protocol::model::Problem>,
    },
    /// It couldn't start, or the connection failed.
    #[error("provider `{name}`: {message}")]
    Failed { name: String, message: String },
}

/// The bus's `Confirm` for a declared `confirm`.
pub fn confirm_of(raw: &str) -> Result<Confirm, String> {
    match raw {
        "never" => Ok(Confirm::Never),
        "always" => Ok(Confirm::Always),
        "destructive" => Ok(Confirm::Destructive),
        other => Err(format!(
            "confirm `{other}` isn't never, always or destructive"
        )),
    }
}

/// The bus's `Access` for a declared `access`.
pub fn access_of(raw: &str) -> Result<Access, String> {
    match raw {
        "write" => Ok(Access::Write),
        "read" => Ok(Access::Read),
        "record" => Ok(Access::Record),
        other => Err(format!("access `{other}` isn't write, read or record")),
    }
}

/// Reads a host environment variable (tests pass their own).
pub type HostEnv = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// This process's environment, as a [`HostEnv`].
pub fn process_env() -> HostEnv {
    Arc::new(|name| std::env::var(name).ok())
}

/// Everything a spawn needs.
pub struct Launch {
    /// `<extension>/<instance id>`.
    pub name: String,
    /// The instance it runs as: what it's told its refs' segment is
    /// (`OXPLOW_PROVIDER_ID`).
    pub instance_id: String,
    pub ext_dir: PathBuf,
    pub spec: ProviderSpec,
    /// The declarations as approved.
    pub declared: InitializeResult,
    /// Credential name → value (from the keychain).
    pub credentials: BTreeMap<String, String>,
    pub host_env: HostEnv,
    /// What it may call of the host (`host/call`); none (the conformance
    /// kit, which has no project) answers `MethodNotFound`.
    pub host_calls: Option<Arc<HostCalls>>,
    /// Told when it announces a change (`host/changed`), with the
    /// collectors it names (all of them: `None`).
    pub on_changed: Option<OnChanged>,
}

/// What a `host/changed` from the provider calls.
pub type OnChanged = Arc<dyn Fn(Option<Vec<String>>) + Send + Sync>;

/// A running, handshaken provider. Dropping it kills the process; ending
/// it the orderly way is [`Self::shutdown`].
pub struct Connection {
    pub peer: Peer,
    child: tokio::process::Child,
    _proxy: Option<crate::net_sandbox::EgressProxy>,
    _group: crate::external_process::ProcessGroup,
    _watch: Option<crate::external_process::RevocationWatch>,
}

impl Connection {
    /// The process has exited (its connection is gone).
    pub fn exited(&mut self) -> bool {
        let exited = !matches!(self.child.try_wait(), Ok(None));
        if exited {
            self._group.stop();
        }
        exited
    }

    /// End the process the orderly way: `shutdown` (it finishes what is in
    /// flight, persists and exits), then wait for it to go; past `grace`
    /// it is killed. A provider with state on disk isn't cut mid-write.
    pub async fn shutdown(mut self, grace: Duration) {
        let ended = tokio::time::timeout(grace, async {
            if !self.peer.is_closed() {
                let _ = self
                    .peer
                    .request(method::SHUTDOWN, serde_json::Value::Null)
                    .await;
            }
            let _ = self.child.wait().await;
        })
        .await;
        self._group.stop();
        if ended.is_err() {
            let _ = self.child.kill().await;
        }
    }

    /// How the process ended, once it has (it's given a moment to be
    /// reaped after its pipes close).
    pub async fn exit_status(&mut self) -> Option<std::process::ExitStatus> {
        let status = tokio::time::timeout(Duration::from_millis(500), self.child.wait())
            .await
            .ok()
            .and_then(Result::ok);
        if status.is_some() {
            self._group.stop();
        }
        status
    }
}

/// A spawned provider process and its pipes, before any handshake.
pub struct Spawned {
    pub child: tokio::process::Child,
    pub stdin: tokio::process::ChildStdin,
    pub stdout: tokio::process::ChildStdout,
    /// The egress proxy it reaches the network through, where enforced;
    /// it lives as long as this does.
    pub proxy: Option<crate::net_sandbox::EgressProxy>,
    pub group: crate::external_process::ProcessGroup,
}

/// Start the provider's program the trusted way: a scrubbed environment
/// (PATH, HOME, its declared `env` names, its credentials, `OXPLOW_*`
/// context), the egress proxy and `sandbox-exec` where enforced, stderr
/// to the log, killed on drop. Consent is the caller's to have checked.
pub async fn spawn(launch: &Launch) -> Result<Spawned, HostError> {
    let failed = |message: String| HostError::Failed {
        name: launch.name.clone(),
        message,
    };
    if launch.spec.credentials.iter().any(|credential| {
        credential.secret_ref.is_some() && !launch.spec.is_host_secret_bearer(credential)
    }) {
        return Err(failed(
            "host secret reference transport is unavailable".into(),
        ));
    }
    let (own, _) = launch.spec.program();
    // A server by url has no program of its own in the folder.
    if !launch.spec.is_remote() && !launch.ext_dir.join(&own).is_file() {
        return Err(failed(format!(
            "`{own}` doesn't exist in the extension folder"
        )));
    }
    let adapter = adapter_bin();
    let (runs, args) = launch.spec.launch(&adapter);
    let entry = match runs {
        Some(adapter) if !adapter.is_file() => {
            return Err(failed(format!(
                "oxplow's MCP adapter isn't installed beside it ({})",
                adapter.display()
            )))
        }
        Some(adapter) => adapter,
        None => launch.ext_dir.join(&own),
    };
    let proxy = if crate::net_sandbox::enforced() {
        Some(
            crate::net_sandbox::EgressProxy::start(launch.spec.network.clone())
                .await
                .map_err(|e| failed(format!("couldn't start the egress proxy: {e}")))?,
        )
    } else {
        None
    };
    let mut cmd = match &proxy {
        None => tokio::process::Command::new(&entry),
        Some(_) => {
            let mut c = tokio::process::Command::new(crate::net_sandbox::SANDBOX_EXEC);
            c.arg("-p").arg(crate::net_sandbox::PROFILE).arg(&entry);
            c
        }
    };
    cmd.args(&args)
        .current_dir(&launch.ext_dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("OXPLOW_EXTENSION_DIR", &launch.ext_dir)
        .env("OXPLOW_PROVIDER_ID", &launch.instance_id);
    for name in PASSED_ENV
        .iter()
        .copied()
        .chain(launch.spec.env.iter().map(String::as_str))
    {
        if let Some(v) = (launch.host_env)(name) {
            cmd.env(name, v);
        }
    }
    for (name, value) in &launch.credentials {
        cmd.env(name, value);
    }
    if let Some(p) = &proxy {
        for (name, value) in p.env() {
            cmd.env(name, value);
        }
    }
    if let Some(gate) = launch
        .host_calls
        .as_ref()
        .and_then(|calls| calls.gate.lock().clone())
    {
        gate().map_err(&failed)?;
    }
    crate::external_process::ProcessGroup::prepare(cmd.as_std_mut());
    let mut child = cmd
        .spawn()
        .map_err(|e| failed(format!("couldn't start `{own}`: {e}")))?;
    let group = crate::external_process::ProcessGroup::new(
        child.id().ok_or_else(|| failed("no process id".into()))?,
    );
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failed("no stdout".into()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| failed("no stdin".into()))?;
    if let Some(stderr) = child.stderr.take() {
        let name = launch.name.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(provider = %name, "{line}");
            }
        });
    }
    Ok(Spawned {
        child,
        stdin,
        stdout,
        proxy,
        group,
    })
}

/// The host's `initialize` params.
pub fn initialize_params() -> InitializeParams {
    InitializeParams {
        protocol_version: PROTOCOL_VERSION.into(),
        host: Party {
            name: "oxplow".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
    }
}

/// Spawn the approved provider and handshake with it: the live
/// `initialize` must equal `launch.declared`.
pub async fn connect(launch: &Launch) -> Result<Connection, HostError> {
    let failed = |message: String| HostError::Failed {
        name: launch.name.clone(),
        message,
    };
    let Spawned {
        child,
        stdin,
        stdout,
        proxy,
        group,
    } = spawn(launch).await?;
    let (peer, incoming) = Peer::spawn(stdout, stdin);
    serve_incoming(
        peer.clone(),
        incoming,
        launch.host_calls.clone(),
        launch.on_changed.clone(),
    );
    let conn = Connection {
        _watch: launch
            .host_calls
            .as_ref()
            .and_then(|calls| calls.gate.lock().clone())
            .map(|gate| crate::external_process::RevocationWatch::start(group.clone(), gate)),
        _group: group,
        peer,
        child,
        _proxy: proxy,
    };
    let live: InitializeResult = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        conn.peer.call(method::INITIALIZE, &initialize_params()),
    )
    .await
    .map_err(|_| failed(format!("no answer to initialize in {HANDSHAKE_TIMEOUT:?}")))?
    .map_err(|e| failed(format!("initialize: {e}")))?;
    if live != launch.declared {
        return Err(HostError::DeclarationsChanged {
            name: launch.name.clone(),
            detail: first_difference(&launch.declared, &live),
        });
    }
    Ok(conn)
}

/// What the provider sends that isn't a reply: `host/call` is answered
/// by `host_calls` (protocol 3; none — the conformance kit — answers
/// `MethodNotFound`), any other request `MethodNotFound`; `host/changed`
/// (protocol 4) goes to `on_changed`, and any other notification outside
/// a read is dropped.
pub fn serve_incoming(
    peer: Peer,
    mut incoming: tokio::sync::mpsc::Receiver<Incoming>,
    host_calls: Option<Arc<HostCalls>>,
    on_changed: Option<OnChanged>,
) {
    tokio::spawn(async move {
        while let Some(message) = incoming.recv().await {
            let (id, method, params) = match message {
                Incoming::Request { id, method, params } => (id, method, params),
                Incoming::Notification { method, params } => {
                    if method == oxplow_provider_protocol::model::method::HOST_CHANGED {
                        match (
                            &on_changed,
                            serde_json::from_value::<
                                oxplow_provider_protocol::model::HostChangedParams,
                            >(params),
                        ) {
                            (Some(changed), Ok(p)) => changed(p.collectors),
                            (_, Err(e)) => {
                                tracing::debug!(error = %e, "a malformed host/changed is dropped")
                            }
                            (None, Ok(_)) => {}
                        }
                    }
                    continue;
                }
            };
            match (&host_calls, method.as_str()) {
                (Some(calls), oxplow_provider_protocol::model::method::HOST_CALL) => {
                    let (peer, calls) = (peer.clone(), calls.clone());
                    tokio::spawn(async move {
                        let answer = calls.call(params).await;
                        let _ = peer.respond(id, answer).await;
                    });
                }
                _ => {
                    let _ = peer
                        .respond(id, Err(ProtocolError::MethodNotFound(method)))
                        .await;
                }
            }
        }
    });
}

/// What a provider may call of the host over the protocol (`host/call`):
/// the scopes its manifest `needs`, through the gate a
/// command's handler meets (`scope_calls::Calls`), each call part of the
/// run of the `invoke` it serves — named by that call's idempotency key —
/// so the run's audit row records it. A call naming no invoke under way
/// is refused: nothing a provider does of oxplow is outside a run.
pub struct HostCalls {
    gate: parking_lot::Mutex<Option<crate::exec_consent::Gate>>,
    needs: Vec<String>,
    db: oxplow_db::Database,
    state: Option<(
        crate::extension_state::StateSource,
        crate::extension_configuration::ConfigurationSource,
        oxplow_domain::run::ContributionRef,
    )>,
    runs: parking_lot::Mutex<std::collections::HashMap<String, oxplow_domain::run::RunContext>>,
}

/// Keeps an invocation's host-call authority registered for its full future
/// lifetime. Dropping it also cleans up when the invocation is cancelled.
pub(crate) struct Registration<'a> {
    host_calls: &'a HostCalls,
    key: String,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.host_calls.finish(&self.key);
    }
}

impl HostCalls {
    pub fn new(
        needs: Vec<String>,
        db: oxplow_db::Database,
        state: Option<(
            crate::extension_state::StateSource,
            crate::extension_configuration::ConfigurationSource,
            oxplow_domain::run::ContributionRef,
        )>,
    ) -> Arc<Self> {
        Arc::new(Self {
            gate: parking_lot::Mutex::new(None),
            needs,
            db,
            state,
            runs: parking_lot::Mutex::default(),
        })
    }

    pub fn set_gate(&self, gate: crate::exec_consent::Gate) {
        *self.gate.lock() = Some(gate);
    }

    pub(crate) fn authority_current(&self) -> bool {
        let gate = self.gate.lock().clone();
        gate.is_some_and(|gate| gate().is_ok())
    }

    /// Register invocation authority until the returned guard is dropped.
    /// Production invokes use this guard so cancellation cannot leave a stale
    /// run in the host-call table.
    pub(crate) fn register(
        &self,
        key: &str,
        run: oxplow_domain::run::RunContext,
    ) -> Result<Registration<'_>, oxplow_domain::CommandError> {
        let mut runs = self.runs.lock();
        if runs.contains_key(key) {
            return Err(oxplow_domain::CommandError::Denied {
                reason: "an invocation is already registered for this key".into(),
            });
        }
        runs.insert(key.to_string(), run);
        drop(runs);
        Ok(Registration {
            host_calls: self,
            key: key.to_string(),
        })
    }

    /// Legacy registration for compatibility callers; production invokes use
    /// [`Self::register`] so cancellation releases the run automatically.
    pub fn begin(&self, key: &str, run: oxplow_domain::run::RunContext) {
        self.runs.lock().insert(key.to_string(), run);
    }

    pub fn finish(&self, key: &str) {
        self.runs.lock().remove(key);
    }

    /// Answer one `host/call`.
    pub async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ProtocolError> {
        if let Some(gate) = self.gate.lock().clone() {
            gate().map_err(|message| ProtocolError::InvalidInput {
                field: "/scope".into(),
                message,
            })?;
        }
        let params: oxplow_provider_protocol::model::HostCallParams =
            serde_json::from_value(params)
                .map_err(|e| ProtocolError::InvalidParams(e.to_string()))?;
        let refused = |message: String| ProtocolError::InvalidInput {
            field: "/scope".into(),
            message,
        };
        if let Some(op) = &params.op {
            return Err(refused(format!(
                "`{}` has no operation `{op}` a provider calls",
                params.scope
            )));
        }
        let Some(run) = params
            .key
            .as_ref()
            .and_then(|k| self.runs.lock().get(k).cloned())
        else {
            return Err(refused(match &params.key {
                Some(key) => format!(
                    "no call under way is keyed `{key}`: a `host/call` names the invoke it serves"
                ),
                None => "a `host/call` names the invoke it serves (`key`)".to_string(),
            }));
        };
        let (needs, db, gate, state) = (
            self.needs.clone(),
            self.db.clone(),
            self.gate.lock().clone(),
            self.state.clone(),
        );
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let read = Box::new(|q: oxplow_db::SqlQuery| {
                runtime.block_on(db.read(move |tx| oxplow_db::semantic_layer::read_on(tx, &q)))
            });
            // Decided against the provider's own declaration, on the run
            // it serves.
            let mut run = run.clone().authorized(oxplow_domain::run::Authority {
                declared: crate::grants::script_request(&needs),
                grant: None,
                enforcement: crate::grants::enforcement(),
            });
            if let Some(gate) = gate {
                run = run.guarded(gate);
            }
            let mut calls = crate::scope_calls::Calls::new(&run, &needs, read);
            if let Some((source, configuration, owner)) = state {
                // A provider reads its own namespace, even when another
                // extension's command invoked it. Its owner is captured at
                // instance construction, never supplied by the protocol call.
                let state_run = run
                    .child(Some(owner), oxplow_domain::run::Mode::External)
                    .authorized(oxplow_domain::run::Authority {
                        declared: crate::grants::script_request(&needs),
                        grant: None,
                        enforcement: crate::grants::enforcement(),
                    });
                let configuration_run = state_run.clone();
                let (state_runtime, parent) = (runtime.clone(), &run);
                calls = calls.with_state_reader(Box::new(move |input| {
                    let result = state_runtime.block_on(source.snapshot(state_run.clone(), input));
                    if result.is_ok() {
                        parent.trace().count("state.read");
                        parent.used_scope("state.read");
                    }
                    result
                }));
                let configuration_runtime = runtime.clone();
                calls = calls.with_configuration_reader(Box::new(move || {
                    let result = configuration_runtime
                        .block_on(configuration.snapshot(configuration_run.clone()));
                    if result.is_ok() {
                        parent.trace().count("extension_config.read");
                        parent.used_scope("extension_config.read");
                    }
                    result
                }));
            }
            calls.serve(&params.scope, params.args)
        })
        .await
        .map_err(|e| ProtocolError::Internal(e.to_string()))?
        .map_err(refused)
    }
}

/// Where two declarations first differ, for the person reading why.
pub fn first_difference(approved: &InitializeResult, live: &InitializeResult) -> String {
    let a = serde_json::to_value(approved).unwrap_or_default();
    let b = serde_json::to_value(live).unwrap_or_default();
    crate::extension_impact::json_difference(&a, &b)
        .map(|(path, a, b)| format!("at `{path}`: approved {a}, running {b}"))
        .unwrap_or_else(|| "they differ".into())
}

/// The host environment every provider is given, besides what its spec
/// names. `LLVM_PROFILE_FILE` is set only in a coverage run, to an absolute
/// path: without it an instrumented provider writes its `.profraw` into its
/// working directory — the approved copy — which changes the copy's hash
/// and loses the baseline `last_approved` reads.
pub(crate) const PASSED_ENV: [&str; 3] = ["PATH", "HOME", "LLVM_PROFILE_FILE"];

/// A verified copy of an approved provider's extension folder: what a
/// start runs, so the bytes that run are the bytes whose hash was
/// approved — not the live tree, which a checkout or an edit can change
/// between the check and the exec (or under a provider that loads its
/// modules lazily).
pub struct ApprovedCopy {
    pub hash: String,
    pub root: PathBuf,
    /// The provider's folder inside the copy.
    pub ext_dir: PathBuf,
    /// Its declarations, read from the copy.
    pub declared: InitializeResult,
}

/// Copy `ext`'s folder into `copies/<ext>/<id>/<hash>` and check that the
/// copy's hash is approved on this machine. An existing copy is re-hashed
/// (it lives outside the repo, but a process running as the person can
/// still write there), and older copies of the provider are removed.
///
/// One start of a program copies at a time (tsk839): instances of one
/// program start together routinely, and each start's cleanup removes
/// what isn't its own hash — another's temp copy, or a kept copy it
/// replaced — so the whole copy, keep and cleanup runs under an advisory
/// lock on `copies/<ext>/<id>.lock`, which threads and processes alike
/// wait on. A temp copy found under the lock was abandoned.
pub fn approved_copy(
    project_dir: &Path,
    copies: &Path,
    approvals: &crate::exec_consent::ApprovalStore,
    ext: &crate::extensions::Extension,
    spec: &ProviderSpec,
) -> Result<ApprovedCopy, HostError> {
    let name = spec.approval_name(&ext.name);
    let failed = |message: String| HostError::Failed {
        name: name.clone(),
        message,
    };
    let program = crate::exec_consent::provider_program(ext, spec);
    let rel = ext.path.trim_end_matches('/');
    let base = copies.join(&ext.name).join(&spec.id);
    let _one_at_a_time =
        lock_copies(&base).map_err(|e| failed(format!("copying it to run: {e}")))?;
    let tmp = base.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    let copied = crate::approved_code::copy_directory(&project_dir.join(rel), &tmp.join(rel))
        .and_then(|()| program.hash(&tmp));
    let hash = match copied {
        Ok(h) => h,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(failed(format!("copying it to run: {e}")));
        }
    };
    if !approvals.is_approved(&program.key(), &hash) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(HostError::Unapproved(name));
    }
    let root = base.join(&hash);
    let intact = root.is_dir() && program.hash(&root).is_ok_and(|h| h == hash);
    if intact {
        let _ = std::fs::remove_dir_all(&tmp);
    } else {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::rename(&tmp, &root).map_err(|e| failed(format!("keeping its copy: {e}")))?;
    }
    // Older versions (and abandoned temp copies) go.
    if let Ok(entries) = std::fs::read_dir(&base) {
        for entry in entries.flatten() {
            if entry.file_name() != hash.as_str() {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let ext_dir = root.join(rel);
    let declared =
        super::spec::read_declarations(spec, &|f| std::fs::read_to_string(ext_dir.join(f)).ok())
            .map_err(&failed)?;
    Ok(ApprovedCopy {
        ext_dir,
        declared,
        hash,
        root,
    })
}

/// The advisory lock on a program's copies (`<base>.lock`, beside the
/// folder its cleanup empties), held until the file is dropped.
pub(crate) fn lock_copies(base: &Path) -> std::io::Result<std::fs::File> {
    use fs2::FileExt;
    let parent = base.parent().unwrap_or(base);
    std::fs::create_dir_all(parent)?;
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(parent.join(name))?;
    lock.lock_exclusive()?;
    Ok(lock)
}

/// The copy of `spec` a start last ran (`copies/<ext>/<id>/<hash>`, kept
/// until a newer approved copy replaces it), when it is still intact: its
/// spec and declarations, read from the copy — what the person approved
/// last, whether or not an instance runs now. `None` before it first ran.
pub fn last_approved(
    copies: &Path,
    ext: &crate::extensions::Extension,
    spec: &ProviderSpec,
) -> Option<super::spec::DeclaredProvider> {
    let rel = ext.path.trim_end_matches('/');
    let base = copies.join(&ext.name).join(&spec.id);
    if !base.is_dir() {
        return None;
    }
    // A start replacing the copy holds this lock: read between its removal
    // of the old one and its rename of the new and there is no copy to see.
    let _one_at_a_time = lock_copies(&base).ok()?;
    let entries = std::fs::read_dir(&base).ok()?;
    entries.flatten().find_map(|entry| {
        let hash = entry.file_name().into_string().ok()?;
        if hash.starts_with('.') {
            return None;
        }
        let root = entry.path();
        let copied = crate::extensions::load_project_extension(&root, &ext.name);
        let copied_spec = copied.providers.iter().find(|p| p.id == spec.id)?;
        let program = crate::exec_consent::provider_program(&copied, copied_spec);
        if program.hash(&root).ok()? != hash {
            return None;
        }
        let dir = root.join(rel);
        let declared = super::spec::DeclaredProvider::read(copied_spec, &|f| {
            std::fs::read_to_string(dir.join(f)).ok()
        });
        declared.declarations.is_some().then_some(declared)
    })
}

/// The credentials provider `provider` of extension `ext` declared in the
/// copy a start of it last ran: what a removed instance's are when the
/// extension no longer declares the provider (tsk841). None when it never
/// ran.
pub fn last_ran_credentials(copies: &Path, ext: &str, provider: &str) -> Vec<String> {
    let base = copies.join(ext).join(provider);
    if !base.is_dir() {
        return Vec::new();
    }
    let Ok(_one_at_a_time) = lock_copies(&base) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .find_map(|e| {
            crate::extensions::load_project_extension(&e.path(), ext)
                .providers
                .into_iter()
                .find(|p| p.id == provider)
                .map(|p| p.credential_names())
        })
        .unwrap_or_default()
}

/// oxplow's MCP adapter (`oxplow-provider-mcp`, P7.A6): shipped beside
/// the running executable (the bundle's sidecar, a dev `target/<profile>`),
/// or one directory up from a test binary in `target/<profile>/deps`.
pub fn adapter_bin() -> PathBuf {
    let name = if cfg!(windows) {
        "oxplow-provider-mcp.exe"
    } else {
        "oxplow-provider-mcp"
    };
    let exe = std::env::current_exe().unwrap_or_default();
    let beside = exe.parent().map(|d| d.join(name));
    let above = exe.parent().and_then(Path::parent).map(|d| d.join(name));
    beside
        .clone()
        .filter(|p| p.is_file())
        .or(above.filter(|p| p.is_file()))
        .or(beside)
        .unwrap_or_else(|| PathBuf::from(name))
}

/// The provider's extension folder under `root`.
pub fn ext_dir(root: &Path, ext: &crate::extensions::Extension) -> PathBuf {
    root.join(&ext.path)
}
