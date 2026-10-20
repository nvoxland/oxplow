//! Starting a provider: the spawn and the handshake.
//!
//! A provider runs from its extension's deployment, which never changes,
//! under the `process` permission (and the `network` and `secrets` its
//! extension declares) a person approved when they deployed it
//! (`.context/extensions.md` "Permissions"). The spawn mirrors an `exec`
//! source: a scrubbed environment (PATH, HOME, the declared `env` names,
//! the credentials from the keychain, `OXPLOW_*` context), the egress
//! proxy and `sandbox-exec` where the OS enforces `network`, and the
//! process ends with its connection: asked to `shutdown` first, killed
//! once its grace is up ([`Connection::shutdown`]). After the spawn, the
//! live `initialize` must equal the deployment's declarations.

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
    /// It ran, but what it says it is isn't what its deployment declares.
    #[error(
        "provider `{name}` declares something other than its deployment's declarations: {detail}"
    )]
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
    /// The declarations its deployment holds.
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

/// Spawn the provider and handshake with it: the live
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
    gate: parking_lot::Mutex<Option<crate::run_context::LiveCheck>>,
    needs: Vec<String>,
    /// What its extension may reach: its calls are decided within it.
    grant: oxplow_domain::grant::Grant,
    /// What its `sql.read` reads through, the metric functions included.
    sql: crate::sql_gateway::SqlGateway,
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
        grant: oxplow_domain::grant::Grant,
        sql: crate::sql_gateway::SqlGateway,
        state: Option<(
            crate::extension_state::StateSource,
            crate::extension_configuration::ConfigurationSource,
            oxplow_domain::run::ContributionRef,
        )>,
    ) -> Arc<Self> {
        Arc::new(Self {
            gate: parking_lot::Mutex::new(None),
            needs,
            grant,
            sql,
            state,
            runs: parking_lot::Mutex::default(),
        })
    }

    pub fn set_gate(&self, gate: crate::run_context::LiveCheck) {
        *self.gate.lock() = Some(gate);
    }

    /// Whether its gate now refuses it (the extension or instance was
    /// turned off, its approval withdrawn): the host stops its process then.
    pub(crate) fn revoked(&self) -> bool {
        let gate = self.gate.lock().clone();
        gate.is_some_and(|gate| gate().is_err())
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
        let (needs, grant, sql, gate, state) = (
            self.needs.clone(),
            self.grant.clone(),
            self.sql.clone(),
            self.gate.lock().clone(),
            self.state.clone(),
        );
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            // Decided against the provider's own declaration, within its
            // extension's permissions, on the run it serves.
            let mut run = run.clone().authorized(oxplow_domain::run::Authority {
                declared: crate::grants::script_request(&needs),
                grant: grant.clone(),
                enforcement: crate::grants::enforcement(),
            });
            if let Some(gate) = gate {
                run = run.guarded(gate);
            }
            // Read as a command's or an effect's script does: through the
            // SQL gateway, on the run's activation pin.
            let read = Box::new(|q: oxplow_db::SqlQuery| runtime.block_on(sql.run_in(&run, q)));
            let mut calls = crate::scope_calls::Calls::new(&run, &needs, read);
            if let Some((source, configuration, owner)) = state {
                // A provider reads its own namespace, even when another
                // extension's command invoked it. Its owner is captured at
                // instance construction, never supplied by the protocol call.
                let state_run = run
                    .child(Some(owner), oxplow_domain::run::Mode::External)
                    .authorized(oxplow_domain::run::Authority {
                        declared: crate::grants::script_request(&needs),
                        grant,
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
pub fn first_difference(declared: &InitializeResult, live: &InitializeResult) -> String {
    let a = serde_json::to_value(declared).unwrap_or_default();
    let b = serde_json::to_value(live).unwrap_or_default();
    crate::extension_impact::json_difference(&a, &b)
        .map(|(path, a, b)| format!("at `{path}`: declared {a}, running {b}"))
        .unwrap_or_else(|| "they differ".into())
}

/// The host environment every provider is given, besides what its spec
/// names. `LLVM_PROFILE_FILE` is set only in a coverage run, to an absolute
/// path: without it an instrumented provider would write its `.profraw`
/// into its working directory — its deployment, which is read-only.
pub(crate) const PASSED_ENV: [&str; 3] = ["PATH", "HOME", "LLVM_PROFILE_FILE"];

/// A provider as a start runs it: from its extension's deployment, which
/// nothing changes after it is made — under the `process` permission the
/// person approved when they deployed it.
pub struct Deployment {
    /// The provider's folder: its extension's deployment.
    pub ext_dir: PathBuf,
    /// Its declarations, read from the deployment.
    pub declared: InitializeResult,
}

/// `ext`'s provider `spec` in its deployment, its declarations read.
pub fn deployment(
    project_dir: &Path,
    ext: &crate::extensions::Extension,
    spec: &ProviderSpec,
) -> Result<Deployment, HostError> {
    let ext_dir = ext_dir(project_dir, ext);
    let declared =
        super::spec::read_declarations(spec, &|f| std::fs::read_to_string(ext_dir.join(f)).ok())
            .map_err(|message| HostError::Failed {
                name: spec.approval_name(&ext.name),
                message,
            })?;
    Ok(Deployment { ext_dir, declared })
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
