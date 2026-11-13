//! A scripted **external provider** speaking oxplow's provider protocol
//! (`.context/providers.md`) — what the host, the instances and the
//! conformance kit are tested against, the way the other test doubles
//! stand in for theirs. Which capability it implements comes from
//! `OXPLOW_FAKE_CAPABILITY`; unset, it is a **snapshots** provider.
//!
//! - `snapshots` (the default): declares [`snapshots_declarations`]
//!   (`read_at` only with `OXPLOW_FAKE_FEATURES=contents`), marks a worktree
//!   and answers what changed between marks; what it marked stays in
//!   memory and the `mark`s answered are counted in its state (`marks`).
//! - `filesystem`: declares [`filesystem_declarations`] (`write` only with
//!   `OXPLOW_FAKE_FEATURES=writable`) and serves an in-memory tree, seeded
//!   from `OXPLOW_FAKE_FILES` (a JSON object of path → base64).
//! - `agent_harness`: declares [`harness_declarations`].
//!
//! Every mode has the same config (it needs a `team`, a string; a clean
//! `check` returns the handle `fake:<team>`) and has the hooks below.
//! With `OXPLOW_FAKE_STATE` (a file) its service's state is kept there and
//! read back at start, so it outlives the process as a real service's
//! does; without, it lives in memory.
//!
//! Two notifications act at once: `fake/changed { collectors? }` makes
//! it send the host `host/changed` (what a webhook would), and
//! `fake/exit` makes it exit on its own (status 3).
//!
//! **Hooks**, from `OXPLOW_FAKE_HOOKS` at start or a `fake/hooks { hooks }`
//! notification later (comma-separated). A notification **replaces** the
//! hooks — `""` clears them — keeping only what the fake declared at
//! `initialize` (`bad-declarations`), which a running process can't take
//! back. They act in every mode; the ones on a `read` need a collector,
//! which only the snapshots mode has:
//! - `fail-next:<n>` — the next `n` `check` / `invoke` / `read` calls fail
//!   (`Internal`);
//! - `slow-check:<ms>` — every `check` takes `ms` first;
//! - `slow:<ms>` — every `invoke` and `read` takes `ms` first (and honours
//!   `$/cancel` meanwhile);
//! - `crash` — drop the connection on the next request (the binary exits
//!   with status 3);
//! - `shutdown-file:<path>` — a `shutdown` writes `<path>` before it
//!   answers; `ignore-shutdown` — a `shutdown` is answered and it keeps
//!   running;
//! - `bad-declarations` — `initialize` declares an extra command (named
//!   `undeclared`) the checked-in declarations don't have;
//! - `rate-limit:<ms>` — the next `invoke` or `read` is refused
//!   `RateLimited` with `retry_after_ms: <ms>`;
//! - `needs:<NAME>` — `check` reports a problem unless the credential
//!   `<NAME>` reached the process;
//! - `accepts:<NAME>=<value>` — `check`, `invoke` and `read` answer `Auth`
//!   naming `<NAME>` unless the credential `<NAME>` the process holds is
//!   `<value>` (its service takes that token and no other);
//! - `lax-check` — `check` doesn't look at its credentials (the
//!   `accepts` hook then shows only on `invoke` and `read`);
//! - `refuse-auth` — `invoke` and `read` answer `Auth` naming no
//!   credential (a service that doesn't say which token it refused);
//! - `lose-reply` — the next `invoke` lands but is never answered (a
//!   reply lost on the way);
//! - `started-file:<path>` — each `invoke` writes `<path>` as it begins
//!   (before `slow`), so a test knows a call is under way without timing
//!   it;
//! - `host-read` — an `invoke` first reads the host (`host/call`
//!   `sql.read`, `SELECT 7 AS n`, naming the call's idempotency key) and
//!   answers what it read beside its result (`read`);
//! - `config-read` — as `host-read`, but the read is of the provider's own
//!   extension configuration (`host/call` `extension_config.read`);
//! - on a snapshots `read`: `checkpoint-at-end` (one `$/state`, after the
//!   last record), `progress` (`$/progress` before each record, then 100 ms
//!   over it), `read-fail-after:<n>` (`Internal` after streaming and
//!   checkpointing `n` records), `stuck-cursor` (every `$/state` is
//!   `{ cursor: 0 }`) and `hold-read-file:<path>` (a `read` waits, after
//!   `slow`, until `<path>` exists, sending `$/progress` meanwhile).
//!
//! **Idempotency keys:** an `invoke` sent again with its `idempotency_key`
//! is done once and answered as the first was; the key sent with another
//! write is `InvalidInput` at `/idempotency_key`. (The state's `answered`
//! is kept, so it holds across a restart.)
//!
//! **Generic hooks the app tests can use, by command:** the snapshots
//! mode's `mark { stream, worktree, trigger }` and `changed { stream,
//! from, to }` (a `mark` of an empty directory is enough) carry every
//! invoke hook above, and its collector `marks` (one `mark` record per
//! `mark` answered, in order, with a `$/state` checkpoint `{ cursor, seen }`
//! — opaque to the host) carries the read hooks.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Id, Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, Mutex};

mod filesystem;
mod harness;
mod snapshots;

pub use filesystem::declarations as filesystem_declarations;
pub use harness::declarations as harness_declarations;
pub use snapshots::declarations as snapshots_declarations;

pub const PROVIDER: &str = "fake";

/// The capability it implements (`OXPLOW_FAKE_CAPABILITY`): snapshots, the
/// default, an agent harness ([`harness_declarations`]) or a filesystem
/// provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    AgentHarness,
    /// A snapshots provider; `contents` is its feature of that name
    /// (`OXPLOW_FAKE_FEATURES`, see [`Capability::with_features`]).
    Snapshots {
        contents: bool,
    },
    /// A filesystem provider ([`filesystem_declarations`]); `writable` is
    /// its feature of that name.
    FileSystem {
        writable: bool,
    },
}

impl Default for Capability {
    fn default() -> Self {
        Capability::Snapshots { contents: false }
    }
}

impl Capability {
    /// The capability `name` names (`snapshots` — also when unset —,
    /// `agent_harness`, …).
    pub fn named(name: Option<&str>) -> Result<Self, String> {
        match name.unwrap_or("snapshots") {
            "agent_harness" => Ok(Capability::AgentHarness),
            "snapshots" => Ok(Capability::Snapshots { contents: false }),
            "filesystem" => Ok(Capability::FileSystem { writable: false }),
            other => Err(format!(
                "the fake implements agent_harness, snapshots or filesystem, not `{other}`"
            )),
        }
    }

    /// The capability with the features `list` names (`OXPLOW_FAKE_FEATURES`,
    /// comma-separated); a snapshots provider has `contents`, a filesystem
    /// provider `writable`.
    pub fn with_features(self, list: Option<&str>) -> Result<Self, String> {
        let mut features = list
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|f| !f.is_empty())
            .peekable();
        if features.peek().is_none() {
            return Ok(self);
        }
        match self {
            Capability::Snapshots { .. } => {
                let mut contents = false;
                for f in features {
                    match f {
                        "contents" => contents = true,
                        other => {
                            return Err(format!(
                                "a snapshots provider's feature is `contents`, not `{other}`"
                            ))
                        }
                    }
                }
                Ok(Capability::Snapshots { contents })
            }
            Capability::FileSystem { .. } => {
                let mut writable = false;
                for f in features {
                    match f {
                        "writable" => writable = true,
                        other => {
                            return Err(format!(
                                "a filesystem provider's feature is `writable`, not `{other}`"
                            ))
                        }
                    }
                }
                Ok(Capability::FileSystem { writable })
            }
            _ => Err("only a snapshots or filesystem provider takes features".into()),
        }
    }
}

/// The script hooks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hooks {
    pub fail_next: u32,
    pub slow_ms: u64,
    pub slow_check_ms: u64,
    pub crash: bool,
    pub bad_declarations: bool,
    pub progress: bool,
    pub rate_limit_ms: Option<u64>,
    pub read_fail_after: Option<u64>,
    pub stuck_cursor: bool,
    /// `needs:<NAME>`: `check` reports a problem unless the credential
    /// `<NAME>` reached it (as an environment variable).
    pub needs: Option<String>,
    /// `accepts:<NAME>=<value>`: every `check`, `invoke` and `read` is
    /// refused `Auth` (naming `<NAME>`) unless the credential `<NAME>` is
    /// `<value>`.
    pub accepts: Option<(String, String)>,
    /// `refuse-auth`: every `invoke` and `read` is refused `Auth` naming
    /// no credential.
    pub refuse_auth: bool,
    /// `lax-check`: `check` doesn't look at its credentials.
    pub lax_check: bool,
    /// `lose-reply`: the next `invoke` lands and is never answered.
    pub lose_reply: bool,
    /// `started-file:<path>`: each `invoke` writes `<path>` as it begins.
    pub started_file: Option<String>,
    /// `hold-read-file:<path>`: a `read` waits until `<path>` exists.
    pub hold_read_file: Option<String>,
    /// `host-read`: an `invoke` first reads the host (`host/call`
    /// `sql.read`, `SELECT 7 AS n`, naming its key) and answers what it
    /// read beside its result (`read`).
    pub host_read: bool,
    /// `config-read`: as `host-read`, but the read is of the provider's own
    /// extension configuration (`host/call` `extension_config.read`).
    pub config_read: bool,
    /// `shutdown-file:<path>`: a `shutdown` writes `<path>` before it
    /// answers — a graceful stop, which a kill never is.
    pub shutdown_file: Option<String>,
    /// `ignore-shutdown`: a `shutdown` is answered and the process keeps
    /// running, so the host must kill it.
    pub ignore_shutdown: bool,
    /// `checkpoint-at-end`: a read sends one `$/state`, after its last
    /// record, rather than one per record.
    pub checkpoint_at_end: bool,
}

impl Hooks {
    pub fn parse(spec: &str) -> Hooks {
        let mut hooks = Hooks::default();
        hooks.apply(spec);
        hooks
    }

    /// The hooks a `fake/hooks` notification sets: `spec`'s, over what was
    /// declared at `initialize`.
    pub fn replaced(&self, spec: &str) -> Hooks {
        Hooks {
            bad_declarations: self.bad_declarations,
            ..Hooks::parse(spec)
        }
    }

    fn apply(&mut self, spec: &str) {
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once(':') {
                Some(("fail-next", n)) => self.fail_next = n.parse().unwrap_or(1),
                Some(("slow", ms)) => self.slow_ms = ms.parse().unwrap_or(0),
                Some(("slow-check", ms)) => self.slow_check_ms = ms.parse().unwrap_or(0),
                Some(("read-fail-after", n)) => self.read_fail_after = n.parse().ok(),
                Some(("rate-limit", ms)) => self.rate_limit_ms = ms.parse().ok(),
                Some(("needs", name)) => self.needs = Some(name.to_string()),
                Some(("started-file", path)) => self.started_file = Some(path.to_string()),
                Some(("hold-read-file", path)) => self.hold_read_file = Some(path.to_string()),
                Some(("shutdown-file", path)) => self.shutdown_file = Some(path.to_string()),
                Some(("accepts", pair)) => {
                    self.accepts = pair
                        .split_once('=')
                        .map(|(name, value)| (name.to_string(), value.to_string()))
                }
                None if part == "fail-next" => self.fail_next = 1,
                None if part == "crash" => self.crash = true,
                None if part == "refuse-auth" => self.refuse_auth = true,
                None if part == "lax-check" => self.lax_check = true,
                None if part == "lose-reply" => self.lose_reply = true,
                None if part == "bad-declarations" => self.bad_declarations = true,
                None if part == "progress" => self.progress = true,
                None if part == "stuck-cursor" => self.stuck_cursor = true,
                None if part == "host-read" => self.host_read = true,
                None if part == "config-read" => self.config_read = true,
                None if part == "ignore-shutdown" => self.ignore_shutdown = true,
                None if part == "checkpoint-at-end" => self.checkpoint_at_end = true,
                _ => {}
            }
        }
    }
}

pub(crate) fn command(name: &str, summary: &str, input_schema: Value) -> CommandDecl {
    CommandDecl {
        name: name.into(),
        summary: summary.into(),
        input_schema,
        confirm: "never".into(),
        access: "record".into(),
        undoable: false,
    }
}

/// What `initialize` answers under `bad-declarations`: the declarations
/// of its mode plus an extra command the checked-in ones don't list
/// (checking this in instead is an updated provider's new declarations).
pub fn bad_declarations(declared: InitializeResult) -> InitializeResult {
    let mut declared = declared;
    declared.commands.push(command(
        "undeclared",
        "A command the checked-in declarations don't list.",
        json!({ "type": "object" }),
    ));
    declared
}

/// What the fake declares by default (snapshots mode) — the checked-in
/// declarations a host approves.
pub fn declarations() -> InitializeResult {
    snapshots_declarations(false)
}

#[derive(Default)]
struct World {
    capability: Capability,
    /// How many `mark`s it answered (a snapshots provider's), kept with
    /// its state; what it marked stays in memory.
    marks: u64,
    /// The run the last `invoke` said it was part of, kept with its state
    /// so a test can see what the host sent.
    last_context: Option<Value>,
    snapshot_marks: snapshots::Marks,
    /// A filesystem provider's files.
    files: filesystem::Files,
    hooks: Hooks,
    /// Bumped by every write.
    rev: u64,
    /// Each idempotency key's write — its command and input — and answer.
    answered: HashMap<String, (String, Value, Value)>,
    in_flight: HashMap<Id, oneshot::Sender<()>>,
    /// Where its service's state is kept (`OXPLOW_FAKE_STATE`), so it
    /// outlives the process as a real service's does — what a re-send
    /// after a restart is checked against. In memory without.
    state: Option<std::path::PathBuf>,
}

/// What of the world is its service's — kept across restarts.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    rev: u64,
    answered: HashMap<String, (String, Value, Value)>,
    #[serde(default)]
    marks: u64,
    #[serde(default)]
    last_context: Option<Value>,
}

impl World {
    /// Keep the service's state, when it has somewhere to.
    fn save(&self) {
        let Some(path) = &self.state else {
            return;
        };
        let saved = Saved {
            rev: self.rev,
            answered: self.answered.clone(),
            marks: self.marks,
            last_context: self.last_context.clone(),
        };
        let kept = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, serde_json::to_vec(&saved)?));
        if let Err(e) = kept {
            eprintln!("fake: can't keep its state at {}: {e}", path.display());
        }
    }

    /// The service's state as last kept, if any.
    fn restore(&mut self) {
        let Some(saved) = self
            .state
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<Saved>(&text).ok())
        else {
            return;
        };
        self.rev = saved.rev;
        self.answered = saved.answered;
        self.marks = saved.marks;
        self.last_context = saved.last_context;
    }
}

type Shared = Arc<Mutex<World>>;

/// How serving ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    /// The stream ended, or `shutdown`.
    Ended,
    /// The `crash` hook fired: the connection is dropped without a reply
    /// (the binary exits non-zero).
    Crashed,
}

/// Serve the protocol on `reader` / `writer` as a provider of
/// `capability` until the stream ends, a `shutdown`, or a `crash` hook.
pub async fn serve<R, W>(
    reader: R,
    writer: W,
    hooks: &str,
    state: Option<std::path::PathBuf>,
    capability: Capability,
) -> Served
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (peer, mut incoming) = Peer::spawn(reader, writer);
    let mut world = World {
        capability,
        hooks: Hooks::parse(hooks),
        state,
        ..World::default()
    };
    if matches!(capability, Capability::FileSystem { .. }) {
        if let Ok(json) = std::env::var("OXPLOW_FAKE_FILES") {
            world.files = filesystem::Files::seeded(&json).expect("OXPLOW_FAKE_FILES is valid");
        }
    }
    world.restore();
    let world: Shared = Arc::new(Mutex::new(world));
    while let Some(message) = incoming.recv().await {
        match message {
            Incoming::Notification { method, params } if method == notify::CANCEL => {
                if let Some(id) = params.get("id").and_then(Value::as_u64) {
                    if let Some(stop) = world.lock().await.in_flight.remove(&id) {
                        let _ = stop.send(());
                    }
                }
            }
            Incoming::Notification { method, params } if method == "fake/hooks" => {
                let spec = params.get("hooks").and_then(Value::as_str).unwrap_or("");
                let mut world = world.lock().await;
                world.hooks = world.hooks.replaced(spec);
            }
            // What a webhook would tell it: say so to the host.
            Incoming::Notification { method, params } if method == "fake/changed" => {
                let _ = peer.notify(method::HOST_CHANGED, params).await;
            }
            // Its process ends on its own, as one that crashed.
            Incoming::Notification { method, .. } if method == "fake/exit" => {
                return Served::Crashed;
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } => {
                if world.lock().await.hooks.crash {
                    return Served::Crashed;
                }
                if method == method::SHUTDOWN {
                    let (file, ignore) = {
                        let w = world.lock().await;
                        (w.hooks.shutdown_file.clone(), w.hooks.ignore_shutdown)
                    };
                    if let Some(file) = file {
                        let _ = std::fs::write(file, "shut down");
                    }
                    let _ = peer.respond(id, Ok(Value::Null)).await;
                    if ignore {
                        continue;
                    }
                    return Served::Ended;
                }
                let (stop_tx, stop_rx) = oneshot::channel();
                world.lock().await.in_flight.insert(id, stop_tx);
                let (peer, world) = (peer.clone(), world.clone());
                tokio::spawn(async move {
                    let result = tokio::select! {
                        r = handle(&peer, &world, id, &method, params) => r,
                        _ = stop_rx => Err(ProtocolError::Cancelled),
                    };
                    world.lock().await.in_flight.remove(&id);
                    let _ = peer.respond(id, result).await;
                });
            }
        }
    }
    Served::Ended
}

/// The `rate-limit` hook, once: an `invoke` or `read` refused.
async fn take_rate_limit(world: &Shared) -> Result<(), ProtocolError> {
    match world.lock().await.hooks.rate_limit_ms.take() {
        Some(ms) => Err(ProtocolError::RateLimited {
            message: "scripted rate limit (rate-limit)".into(),
            retry_after_ms: Some(ms),
        }),
        None => Ok(()),
    }
}

/// The `accepts` hook: the service refuses any token but the one named,
/// saying which credential it refused.
async fn require_token(world: &Shared) -> Result<(), ProtocolError> {
    match &world.lock().await.hooks.accepts {
        Some((name, value)) if std::env::var(name).ok().as_deref() != Some(value) => {
            Err(ProtocolError::Auth {
                message: format!("scripted: `{name}` isn't the token it accepts"),
                credential: Some(name.clone()),
            })
        }
        _ => Ok(()),
    }
}

/// The `refuse-auth` hook: the service refuses the call's credentials
/// without saying which.
async fn refuse_auth(world: &Shared) -> Result<(), ProtocolError> {
    if world.lock().await.hooks.refuse_auth {
        return Err(ProtocolError::Auth {
            message: "scripted: refused, no credential named".into(),
            credential: None,
        });
    }
    Ok(())
}

async fn take_failure(world: &Shared) -> Result<(), ProtocolError> {
    let mut w = world.lock().await;
    if w.hooks.fail_next > 0 {
        w.hooks.fail_next -= 1;
        return Err(ProtocolError::Internal(
            "scripted failure (fail-next)".into(),
        ));
    }
    Ok(())
}

async fn slow(world: &Shared) {
    let ms = world.lock().await.hooks.slow_ms;
    if ms > 0 {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }
}

fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, ProtocolError> {
    serde_json::from_value(params).map_err(|e| ProtocolError::InvalidParams(e.to_string()))
}

fn handle_of(team: &str) -> Handle {
    Handle(format!("fake:{team}"))
}

fn require_handle(handle: &Handle) -> Result<(), ProtocolError> {
    if handle.0.starts_with("fake:") {
        Ok(())
    } else {
        Err(ProtocolError::NotConfigured(format!(
            "`{}` isn't a checked instance",
            handle.0
        )))
    }
}

async fn handle(
    peer: &Peer,
    world: &Shared,
    id: Id,
    method: &str,
    params: Value,
) -> Result<Value, ProtocolError> {
    match method {
        method::INITIALIZE => {
            let p: InitializeParams = parse(params)?;
            if p.protocol_version != PROTOCOL_VERSION {
                return Err(ProtocolError::InvalidParams(format!(
                    "protocol {} unsupported; this provider speaks {PROTOCOL_VERSION}",
                    p.protocol_version
                )));
            }
            let (hooks, capability) = {
                let w = world.lock().await;
                (w.hooks.clone(), w.capability)
            };
            let declared = match capability {
                Capability::AgentHarness => harness_declarations(),
                Capability::Snapshots { contents } => snapshots_declarations(contents),
                Capability::FileSystem { writable } => filesystem_declarations(writable),
            };
            let declared = if hooks.bad_declarations {
                bad_declarations(declared)
            } else {
                declared
            };
            Ok(serde_json::to_value(declared).expect("declarations serialize"))
        }
        method::CHECK => {
            take_failure(world).await?;
            if !world.lock().await.hooks.lax_check {
                require_token(world).await?;
            }
            let ms = world.lock().await.hooks.slow_check_ms;
            if ms > 0 {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            let p: CheckParams = parse(params)?;
            let team = p.config.get("team").and_then(Value::as_str);
            let missing = world
                .lock()
                .await
                .hooks
                .needs
                .clone()
                .filter(|name| std::env::var_os(name).is_none());
            let result = match team {
                _ if missing.is_some() => CheckResult {
                    problems: vec![Problem {
                        path: format!("/credentials/{}", missing.unwrap_or_default()),
                        message: "the credential isn't set".into(),
                    }],
                    handle: None,
                },
                Some(team) if !team.is_empty() => CheckResult {
                    problems: Vec::new(),
                    handle: Some(handle_of(team)),
                },
                _ => CheckResult {
                    problems: vec![Problem {
                        path: "/team".into(),
                        message: "a team is required".into(),
                    }],
                    handle: None,
                },
            };
            Ok(serde_json::to_value(result).expect("check result serializes"))
        }
        method::DISCOVER => {
            let p: DiscoverParams = parse(params)?;
            require_handle(&p.handle)?;
            // A snapshots provider reads its marks; the others read nothing.
            let (name, description) = match world.lock().await.capability {
                Capability::Snapshots { .. } => ("mark", "A mark as the fake counts it."),
                _ => return Ok(json!({ "entities": [] })),
            };
            Ok(serde_json::to_value(DiscoverResult {
                entities: vec![EntityDecl {
                    name: name.into(),
                    description: description.into(),
                    schema: json!({ "type": "object" }),
                }],
            })
            .expect("discover result serializes"))
        }
        method::INVOKE => {
            take_rate_limit(world).await?;
            take_failure(world).await?;
            require_token(world).await?;
            refuse_auth(world).await?;
            let p: InvokeParams = parse(params)?;
            require_handle(&p.handle)?;
            world.lock().await.last_context = p
                .context
                .as_ref()
                .map(|c| serde_json::to_value(c).unwrap_or_default());
            if let Some(path) = world.lock().await.hooks.started_file.clone() {
                let _ = std::fs::write(path, p.command.as_bytes());
            }
            slow(world).await;
            let hooks = world.lock().await.hooks.clone();
            let sent_key = p.idempotency_key.clone();
            let key = p.idempotency_key;
            if let Some(key) = &key {
                if let Some((command, input, answer)) = world.lock().await.answered.get(key) {
                    if *command == p.command && *input == p.input {
                        return Ok(answer.clone());
                    }
                    return Err(ProtocolError::InvalidInput {
                        field: "/idempotency_key".into(),
                        message: format!("key `{key}` was sent with another write"),
                    });
                }
            }
            let read = if hooks.host_read || hooks.config_read {
                let (scope, args) = if hooks.config_read {
                    ("extension_config.read", json!({}))
                } else {
                    ("sql.read", json!({ "sql": "SELECT 7 AS n" }))
                };
                Some(
                    peer.request(
                        method::HOST_CALL,
                        json!({ "key": sent_key, "scope": scope, "args": args }),
                    )
                    .await?,
                )
            } else {
                None
            };
            let capability = world.lock().await.capability;
            let mut answer = match capability {
                Capability::AgentHarness => {
                    json!({ "result": harness::answer(&p.command, &p.input)?, "events": [] })
                }
                Capability::FileSystem { writable } => {
                    let result = world
                        .lock()
                        .await
                        .files
                        .answer(&p.command, &p.input, writable)?;
                    json!({ "result": result, "events": [] })
                }
                Capability::Snapshots { contents } => {
                    let mut w = world.lock().await;
                    let marked = w.marks;
                    let result = w
                        .snapshot_marks
                        .answer(&p.command, &p.input, marked, contents)?;
                    if p.command == "mark" {
                        w.marks += 1;
                    }
                    json!({ "result": result, "events": [] })
                }
            };
            if let Some(read) = read.filter(|_| answer["result"].is_object()) {
                answer["result"]["read"] = read;
            }
            let mut w = world.lock().await;
            if let Some(key) = key {
                w.answered.insert(key, (p.command, p.input, answer.clone()));
            }
            w.save();
            if std::mem::take(&mut w.hooks.lose_reply) {
                drop(w);
                // Landed, and never answered.
                std::future::pending::<()>().await;
            }
            Ok(answer)
        }
        method::READ => {
            take_rate_limit(world).await?;
            take_failure(world).await?;
            require_token(world).await?;
            refuse_auth(world).await?;
            let p: ReadParams = parse(params)?;
            require_handle(&p.handle)?;
            let (collector, entity) = ("marks", "mark");
            if p.collector != collector
                || !matches!(world.lock().await.capability, Capability::Snapshots { .. })
            {
                return Err(ProtocolError::InvalidInput {
                    field: "/collector".into(),
                    message: format!("no collector `{}`", p.collector),
                });
            }
            slow(world).await;
            if let Some(path) = world.lock().await.hooks.hold_read_file.clone() {
                // Held, it still says it is alive, as a long read does, so a
                // host's idle timeout is not what ends it.
                while !std::path::Path::new(&path).exists() {
                    peer.notify(
                        notify::PROGRESS,
                        json!({ "id": id, "message": "held", "fraction": 0.0 }),
                    )
                    .await?;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            let after = p
                .state
                .as_ref()
                .and_then(|s| s.get("cursor"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let seen = p
                .state
                .as_ref()
                .and_then(|s| s.get("seen"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let (hooks, rows) = {
                let w = world.lock().await;
                // One record per mark answered, numbered from 1.
                let rows: Vec<(u64, Value)> = (after + 1..=w.marks)
                    .map(|n| (n, json!({ "mark": n })))
                    .collect();
                (w.hooks.clone(), rows)
            };
            let total = rows.len();
            for (i, (rev, row)) in rows.into_iter().enumerate() {
                if hooks.read_fail_after == Some(i as u64) {
                    return Err(ProtocolError::Internal(format!(
                        "scripted read failure after {i} records"
                    )));
                }
                if hooks.progress {
                    peer.notify(
                        notify::PROGRESS,
                        json!({ "id": id, "message": format!("record {} of {total}", i + 1),
                                "fraction": (i + 1) as f64 / total as f64 }),
                    )
                    .await?;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                peer.notify(
                    notify::RECORD,
                    json!({ "id": id, "entity": entity, "row": row }),
                )
                .await?;
                if hooks.checkpoint_at_end && i + 1 < total {
                    continue;
                }
                let cursor = if hooks.stuck_cursor { 0 } else { rev };
                peer.notify(
                    notify::STATE,
                    json!({ "id": id, "state": { "cursor": cursor, "seen": seen + i as u64 + 1 } }),
                )
                .await?;
            }
            Ok(json!({ "records": total }))
        }
        other => Err(ProtocolError::MethodNotFound(other.into())),
    }
}
