//! Starting a provider: consent, the spawn and the handshake.
//!
//! Consent precedes execution: a provider runs only at the version a
//! person approved (`exec_consent`, [`ProgramKind::Provider`]) — its
//! extension folder's files, its entry, args, env, credentials and
//! network, and so its declarations file. The spawn mirrors an `exec`
//! source: a scrubbed environment (PATH, HOME, the declared `env` names,
//! the credentials from the keychain, `OXPLOW_*` context), the egress
//! proxy and `sandbox-exec` where the OS enforces `network`, and the
//! process dies with its connection. After the spawn, the live
//! `initialize` must equal the approved declarations.
//!
//! [`ProgramKind::Provider`]: crate::exec_consent::ProgramKind::Provider

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::{CommandEffect, Confirm};
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

/// The bus's `CommandEffect` for a declared `effect`.
pub fn effect_of(raw: &str) -> Result<CommandEffect, String> {
    match raw {
        "write" => Ok(CommandEffect::Write),
        "read" => Ok(CommandEffect::Read),
        "record" => Ok(CommandEffect::Record),
        other => Err(format!("effect `{other}` isn't write, read or record")),
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
}

/// A running, handshaken provider. Dropping it kills the process.
pub struct Connection {
    pub peer: Peer,
    child: tokio::process::Child,
    _proxy: Option<crate::net_sandbox::EgressProxy>,
}

impl Connection {
    /// The process has exited (its connection is gone).
    pub fn exited(&mut self) -> bool {
        !matches!(self.child.try_wait(), Ok(None))
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
    for name in ["PATH", "HOME"]
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
    let mut child = cmd
        .spawn()
        .map_err(|e| failed(format!("couldn't start `{own}`: {e}")))?;
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
    } = spawn(launch).await?;
    let (peer, incoming) = Peer::spawn(stdout, stdin);
    serve_incoming(peer.clone(), incoming, launch.host_calls.clone());
    let conn = Connection {
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
/// `MethodNotFound`), any other request `MethodNotFound`, and a
/// notification outside a read is dropped.
pub fn serve_incoming(
    peer: Peer,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<Incoming>,
    host_calls: Option<Arc<HostCalls>>,
) {
    tokio::spawn(async move {
        while let Some(message) = incoming.recv().await {
            let Incoming::Request { id, method, params } = message else {
                continue;
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
/// command's handler meets (`scope_calls::Calls`), each call counted
/// with the `invoke` it serves — named by that call's idempotency key — so
/// the run's audit row records it.
pub struct HostCalls {
    needs: Vec<String>,
    db: oxplow_db::Database,
    traces:
        parking_lot::Mutex<std::collections::HashMap<String, Arc<crate::scope_calls::ScopeTrace>>>,
}

impl HostCalls {
    pub fn new(needs: Vec<String>, db: oxplow_db::Database) -> Arc<Self> {
        Arc::new(Self {
            needs,
            db,
            traces: parking_lot::Mutex::default(),
        })
    }

    /// Count the calls naming `key` into `trace` until [`Self::finish`].
    pub fn begin(&self, key: &str, trace: Arc<crate::scope_calls::ScopeTrace>) {
        self.traces.lock().insert(key.to_string(), trace);
    }

    pub fn finish(&self, key: &str) {
        self.traces.lock().remove(key);
    }

    /// Answer one `host/call`.
    pub async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ProtocolError> {
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
        let trace = params
            .key
            .as_ref()
            .and_then(|k| self.traces.lock().get(k).cloned())
            .unwrap_or_default();
        let (needs, db) = (self.needs.clone(), self.db.clone());
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let read = Box::new(|q: oxplow_db::SqlQuery| {
                runtime.block_on(db.read(move |tx| oxplow_db::semantic_layer::read_on(tx, &q)))
            });
            let mut calls = crate::scope_calls::Calls::new(&needs, &trace, read);
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
    crate::extension_effects::json_difference(&a, &b)
        .map(|(path, a, b)| format!("at `{path}`: approved {a}, running {b}"))
        .unwrap_or_else(|| "they differ".into())
}

/// A verified copy of an approved provider's extension folder: what a
/// start runs, so the bytes that run are the bytes whose hash was
/// approved — not the live tree, which a checkout or an edit can change
/// between the check and the exec (or under a provider that loads its
/// modules lazily).
pub struct ApprovedCopy {
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
    let copied =
        copy_tree(&project_dir.join(rel), &tmp.join(rel)).and_then(|()| program.hash(&tmp));
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
    Ok(ApprovedCopy { ext_dir, declared })
}

/// The advisory lock on a program's copies (`<base>.lock`, beside the
/// folder its cleanup empties), held until the file is dropped.
fn lock_copies(base: &Path) -> std::io::Result<std::fs::File> {
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
    let entries = std::fs::read_dir(copies.join(&ext.name).join(&spec.id)).ok()?;
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
    let Ok(entries) = std::fs::read_dir(copies.join(ext).join(provider)) else {
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

/// Copy the regular files and directories under `from` to `to`
/// (permissions kept); a symlink is refused, as the approval refuses it.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let target = to.join(entry.file_name());
        if kind.is_symlink() {
            return Err(std::io::Error::other(format!(
                "{} is a symlink",
                entry.path().display()
            )));
        } else if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
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
