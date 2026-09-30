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

/// Everything a spawn needs.
pub struct Launch {
    /// `<extension>/<id>`.
    pub name: String,
    pub ext_dir: PathBuf,
    pub spec: ProviderSpec,
    /// The declarations as approved.
    pub declared: InitializeResult,
    /// Credential name → value (from the keychain).
    pub credentials: BTreeMap<String, String>,
    pub host_env: HostEnv,
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

/// Spawn the approved provider and handshake with it.
pub async fn connect(launch: &Launch) -> Result<Connection, HostError> {
    let failed = |message: String| HostError::Failed {
        name: launch.name.clone(),
        message,
    };
    let entry = launch.ext_dir.join(&launch.spec.entry);
    if !entry.is_file() {
        return Err(failed(format!(
            "entry `{}` doesn't exist in the extension folder",
            launch.spec.entry
        )));
    }
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
    cmd.args(&launch.spec.args)
        .current_dir(&launch.ext_dir)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("OXPLOW_EXTENSION_DIR", &launch.ext_dir)
        .env("OXPLOW_PROVIDER_ID", &launch.spec.id);
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
        .map_err(|e| failed(format!("couldn't start `{}`: {e}", launch.spec.entry)))?;
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
    let (peer, incoming) = Peer::spawn(stdout, stdin);
    serve_incoming(peer.clone(), incoming);
    let conn = Connection {
        peer,
        child,
        _proxy: proxy,
    };
    let live: InitializeResult = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        conn.peer.call(
            method::INITIALIZE,
            &InitializeParams {
                protocol_version: PROTOCOL_VERSION.into(),
                host: Party {
                    name: "oxplow".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                },
            },
        ),
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

/// What the provider sends that isn't a reply: the host serves no
/// requests yet, and a notification outside a read is dropped.
fn serve_incoming(peer: Peer, mut incoming: tokio::sync::mpsc::UnboundedReceiver<Incoming>) {
    tokio::spawn(async move {
        while let Some(message) = incoming.recv().await {
            if let Incoming::Request { id, method, .. } = message {
                let _ = peer
                    .respond(id, Err(ProtocolError::MethodNotFound(method)))
                    .await;
            }
        }
    });
}

/// Where two declarations first differ, for the person reading why.
fn first_difference(approved: &InitializeResult, live: &InitializeResult) -> String {
    let a = serde_json::to_value(approved).unwrap_or_default();
    let b = serde_json::to_value(live).unwrap_or_default();
    fn walk(path: &str, a: &serde_json::Value, b: &serde_json::Value) -> Option<String> {
        use serde_json::Value;
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
                keys.into_iter().find_map(|k| {
                    walk(
                        &format!("{path}/{k}"),
                        x.get(k).unwrap_or(&Value::Null),
                        y.get(k).unwrap_or(&Value::Null),
                    )
                })
            }
            (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
                .iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (p, q))| walk(&format!("{path}/{i}"), p, q)),
            _ if a == b => None,
            _ => Some(format!(
                "at `{}`: approved {a}, running {b}",
                if path.is_empty() { "/" } else { path }
            )),
        }
    }
    walk("", &a, &b).unwrap_or_else(|| "they differ".into())
}

/// The provider's extension folder under `root`.
pub fn ext_dir(root: &Path, ext: &crate::extensions::Extension) -> PathBuf {
    root.join(&ext.path)
}
