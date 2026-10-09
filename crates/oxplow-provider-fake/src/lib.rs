//! A scripted **work-items provider** speaking oxplow's provider protocol
//! (P5.D2, `.context/providers.md`) — what the host, the instances and the
//! conformance kit are tested against, the way `oxplow-acp-fake` stands in
//! for an agent.
//!
//! It keeps items in memory (`work_item:<id>:W-<n>`, `<id>` being the
//! instance the host says it is — `OXPLOW_PROVIDER_ID`, `fake` by
//! default), with native states
//! `Backlog` / `Doing` / `Stuck` / `Shipped` / `Dropped` for the canonical
//! `todo` / `in_progress` / `blocked` / `done` / `canceled`. Its config
//! needs a `team` (a string); a clean `check` returns the handle
//! `fake:<team>`. The work-items verbs `create`, `update`, `transition`
//! (undoable: its inverse moves the item back), `link`, `comment` and
//! `delete`, over the contract's inputs (`state` / `native_state`, the
//! fake's `native.points`), plus one command of its own, `estimate` —
//! each returns `work_item.recorded` events (the item as it now stands).
//! Collector `work_items` streams every item changed after the cursor
//! (each write bumps an item's revision; a deleted item stays, marked
//! `deleted`), in revision order, as `$/record` followed by a `$/state`
//! checkpoint `{ cursor, seen }` — opaque to the host.
//!
//! With `OXPLOW_FAKE_STATE` (a file) its service's state — the items and
//! each idempotency key's answer — is kept there and read back at start,
//! so it outlives the process as a real service's does; without, it lives
//! in memory.
//!
//! With `OXPLOW_FAKE_CAPABILITY=effort_policy` it is an **effort policy**
//! instead (`policy`): it declares [`policy_declarations`], keeps no items,
//! and answers `react { event }` with the commands to run; the `react`s it
//! answered are counted in its state (`reacts`).
//!
//! Two notifications act at once: `fake/changed { collectors? }` makes
//! it send the host `host/changed` (what a webhook would), and
//! `fake/exit` makes it exit on its own (status 3).
//!
//! **Hooks**, from `OXPLOW_FAKE_HOOKS` at start or a `fake/hooks { hooks }`
//! notification later (comma-separated). A notification **replaces** the
//! hooks — `""` clears them — keeping only what the fake declared at
//! `initialize` (`plain-writes`, `bad-declarations`), which a running
//! process can't take back (tsk1001):
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
//! - `bogus-react` — as an effort policy, `react` composes a command that
//!   doesn't exist;
//! - `checkpoint-at-end` — a read sends one `$/state`, after its last
//!   record;
//! - `bad-declarations` — `initialize` declares an extra command the
//!   checked-in declarations don't have;
//! - `progress` — a `read` sends `$/progress` before each record, then
//!   takes 100 ms over it (as a long read would);
//! - `rate-limit:<ms>` — the next `invoke` or `read` is refused
//!   `RateLimited` with `retry_after_ms: <ms>`;
//! - `read-fail-after:<n>` — a `read` fails (`Internal`) after streaming
//!   (and checkpointing) `n` records;
//! - `bad-record` — a `read` streams a record of another provider's item;
//! - `stale-read` — a `read` streams each item with its title prefixed
//!   `stale ` (what it reads back isn't what its writes recorded);
//! - `stuck-cursor` — every `$/state` checkpoint is `{ cursor: 0 }`, so a
//!   read from it streams everything again;
//! - `needs:<NAME>` — `check` reports a problem unless the credential
//!   `<NAME>` reached the process;
//! - `accepts:<NAME>=<value>` — `check`, `invoke` and `read` answer `Auth`
//!   naming `<NAME>` unless the credential `<NAME>` the process holds is
//!   `<value>` (its service takes that token and no other).
//! - `lax-check` — `check` doesn't look at its credentials (the
//!   `accepts` hook then shows only on `invoke` and `read`, as with a
//!   service whose check doesn't validate the token);
//! - `refuse-auth` — `invoke` and `read` answer `Auth` naming no
//!   credential (a service that doesn't say which token it refused);
//! - `lose-reply` — the next `invoke` lands but is never answered (a
//!   reply lost on the way);
//! - `started-file:<path>` — each `invoke` writes `<path>` as it begins
//!   (before `slow`), so a test knows a call is under way without timing
//!   it;
//! - `forget-keys` — it declares `idempotent_writes` but does a write sent
//!   again with its key a second time (what the kit must catch);
//! - `plain-writes` (at start) — it doesn't declare `idempotent_writes`
//!   ([`plain_declarations`]) and ignores keys.
//!
//! **Idempotent writes:** it declares `idempotent_writes` and keeps it — an
//! `invoke` sent again with its `idempotency_key` is done once and
//! answered as the first was; the key sent with another write is
//! `InvalidInput` at `/idempotency_key`.

use oxplow_domain::vocabulary::Vocabulary;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::events::schema::{EventType, WorkItemRecorded};
use oxplow_domain::work_items::{CanonicalState, CommentRecord, LinkRecord, WorkItemRecord};
use oxplow_provider_protocol::codec::notify;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::{Id, Incoming, Peer, ProtocolError};
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, Mutex};

mod policy;

pub use policy::declarations as policy_declarations;

pub const PROVIDER: &str = "fake";

/// The capability it implements (`OXPLOW_FAKE_CAPABILITY`): a work list,
/// the default, or an effort policy ([`policy_declarations`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Capability {
    #[default]
    WorkItems,
    EffortPolicy,
}

impl Capability {
    /// The capability `name` names (`work_items` — also when unset — or
    /// `effort_policy`).
    pub fn named(name: Option<&str>) -> Result<Self, String> {
        match name.unwrap_or("work_items") {
            "work_items" => Ok(Capability::WorkItems),
            "effort_policy" => Ok(Capability::EffortPolicy),
            other => Err(format!(
                "the fake implements work_items or effort_policy, not `{other}`"
            )),
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
    pub bad_record: bool,
    pub stale_read: bool,
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
    /// `forget-keys`: a write sent again with its key is done again.
    pub forget_keys: bool,
    /// `plain-writes`: it doesn't declare `idempotent_writes`, and ignores
    /// keys.
    pub plain_writes: bool,
    /// `started-file:<path>`: each `invoke` writes `<path>` as it begins.
    pub started_file: Option<String>,
    /// `host-read`: an `estimate` or an `update` first reads the host
    /// (`host/call` `sql.read`, `SELECT 7 AS n`, naming its key) and
    /// answers what it read beside its result (`read`): its own command's
    /// read, and a capability verb's.
    pub host_read: bool,
    /// `shutdown-file:<path>`: a `shutdown` writes `<path>` before it
    /// answers — a graceful stop, which a kill never is.
    pub shutdown_file: Option<String>,
    /// `ignore-shutdown`: a `shutdown` is answered and the process keeps
    /// running, so the host must kill it.
    pub ignore_shutdown: bool,
    /// `checkpoint-at-end`: a read sends one `$/state`, after its last
    /// record, rather than one per record.
    pub checkpoint_at_end: bool,
    /// `bogus-react`: an effort policy's `react` composes a command that
    /// doesn't exist.
    pub bogus_react: bool,
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
            plain_writes: self.plain_writes,
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
                None if part == "forget-keys" => self.forget_keys = true,
                None if part == "plain-writes" => self.plain_writes = true,
                None if part == "bad-declarations" => self.bad_declarations = true,
                None if part == "progress" => self.progress = true,
                None if part == "bad-record" => self.bad_record = true,
                None if part == "stale-read" => self.stale_read = true,
                None if part == "stuck-cursor" => self.stuck_cursor = true,
                None if part == "host-read" => self.host_read = true,
                None if part == "ignore-shutdown" => self.ignore_shutdown = true,
                None if part == "checkpoint-at-end" => self.checkpoint_at_end = true,
                None if part == "bogus-react" => self.bogus_react = true,
                _ => {}
            }
        }
    }
}

fn native_of(state: CanonicalState) -> &'static str {
    match state {
        CanonicalState::Todo => "Backlog",
        CanonicalState::InProgress => "Doing",
        CanonicalState::Blocked => "Stuck",
        CanonicalState::Done => "Shipped",
        CanonicalState::Canceled => "Dropped",
    }
}

fn canonical_of(native: &str) -> Option<CanonicalState> {
    CanonicalState::ALL
        .into_iter()
        .find(|s| native_of(*s).eq_ignore_ascii_case(native))
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

/// What `initialize` answers under `bad-declarations`: an extra command
/// the checked-in [`declarations`] don't list (checking this in instead
/// is an updated provider's new declarations).
pub fn bad_declarations() -> InitializeResult {
    let mut declared = declarations();
    declared.commands.push(command(
        "undeclared",
        "A command the checked-in declarations don't list.",
        json!({ "type": "object" }),
    ));
    declared
}

/// What the fake declares — the checked-in declarations a host approves.
pub fn declarations() -> InitializeResult {
    declared(true)
}

/// What it declares under `plain-writes`: no `idempotent_writes`.
pub fn plain_declarations() -> InitializeResult {
    declared(false)
}

fn declared(idempotent_writes: bool) -> InitializeResult {
    let string = json!({ "type": "string" });
    let state = json!({ "type": "string",
                        "enum": ["todo", "in_progress", "blocked", "done", "canceled"] });
    let native = json!({ "type": "object", "additionalProperties": false,
                         "properties": { "points": { "type": "integer" } } });
    let recorded_schema = Vocabulary::core()
        .schema(WorkItemRecorded::TYPE, WorkItemRecorded::V)
        .cloned()
        .unwrap_or(Value::Null);
    let mut transition = command(
        "transition",
        "Move a work item to a canonical state, optionally naming a fake state.",
        json!({ "type": "object", "required": ["ref", "to"], "additionalProperties": false,
                "properties": { "ref": string, "to": state, "native_state": string } }),
    );
    transition.undoable = true;
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "work_items".into(),
            features: json!({
                "hierarchy": true, "comments": true, "links": true, "delete": true, "idempotent_writes": idempotent_writes
            }),
            data: serde_json::Value::Null,
        }],
        commands: vec![
            command(
                "create",
                "Create a work item.",
                json!({ "type": "object", "required": ["title"], "additionalProperties": false,
                        "properties": { "title": string, "body": string, "parent_ref": string,
                                        "state": state, "native_state": string,
                                        "native": native } }),
            ),
            command(
                "update",
                "Edit a work item's title, body, parent, state or points.",
                json!({ "type": "object", "required": ["ref"], "additionalProperties": false,
                        "properties": { "ref": string, "title": string, "body": string,
                                        "parent_ref": string, "state": state,
                                        "native_state": string, "native": native } }),
            ),
            transition,
            command(
                "link",
                "Link one work item to another.",
                json!({ "type": "object", "required": ["ref", "target", "link_type"],
                        "additionalProperties": false,
                        "properties": { "ref": string, "target": string, "link_type": string } }),
            ),
            command(
                "comment",
                "Comment on a work item.",
                json!({ "type": "object", "required": ["ref", "body"],
                        "additionalProperties": false,
                        "properties": { "ref": string, "body": string } }),
            ),
            command(
                "delete",
                "Delete a work item.",
                json!({ "type": "object", "required": ["ref"], "additionalProperties": false,
                        "properties": { "ref": string } }),
            ),
            command(
                "estimate",
                "Set a work item's points (a fake-only command).",
                json!({ "type": "object", "required": ["ref", "points"],
                        "additionalProperties": false,
                        "properties": { "ref": string, "points": { "type": "integer" } } }),
            ),
        ],
        event_types: vec![EventTypeDecl {
            event_type: WorkItemRecorded::TYPE.into(),
            v: WorkItemRecorded::V,
            schema: recorded_schema,
        }],
        collectors: vec![CollectorDecl {
            name: "work_items".into(),
            entity: "work_item".into(),
            description: "Every work item, after the cursor.".into(),
        }],
        config_schema: json!({ "type": "object", "required": ["team"],
                               "properties": { "team": { "type": "string" } } }),
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Item {
    record: WorkItemRecord,
    /// The world's revision when it last changed: a read streams the
    /// items changed after its cursor.
    rev: u64,
}

#[derive(Default)]
struct World {
    /// The instance it is: its refs' provider segment.
    id: String,
    capability: Capability,
    /// How many `react`s it answered (an effort policy's), kept with its
    /// state so a test can tell whether it was asked.
    reacts: u64,
    hooks: Hooks,
    items: BTreeMap<u64, Item>,
    next: u64,
    /// Bumped by every write.
    rev: u64,
    /// Each idempotency key's write — its command and input — and answer.
    answered: HashMap<String, (String, Value, Value)>,
    in_flight: HashMap<Id, oneshot::Sender<()>>,
    /// Where its service's state is kept (`OXPLOW_FAKE_STATE`), so it
    /// outlives the process as a real service's does — what a re-send
    /// after a restart is checked against (tsk916). In memory without.
    state: Option<std::path::PathBuf>,
}

/// What of the world is its service's — kept across restarts.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    items: BTreeMap<u64, Item>,
    next: u64,
    rev: u64,
    answered: HashMap<String, (String, Value, Value)>,
    #[serde(default)]
    reacts: u64,
}

impl World {
    /// Keep the service's state, when it has somewhere to.
    fn save(&self) {
        let Some(path) = &self.state else {
            return;
        };
        let saved = Saved {
            items: self.items.clone(),
            next: self.next,
            rev: self.rev,
            answered: self.answered.clone(),
            reacts: self.reacts,
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
        self.items = saved.items;
        self.next = saved.next;
        self.rev = saved.rev;
        self.answered = saved.answered;
        self.reacts = saved.reacts;
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

/// Serve the protocol on `reader` / `writer` as instance `id` of
/// `capability` until the stream ends, a `shutdown`, or a `crash` hook.
pub async fn serve<R, W>(
    reader: R,
    writer: W,
    hooks: &str,
    id: &str,
    state: Option<std::path::PathBuf>,
    capability: Capability,
) -> Served
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (peer, mut incoming) = Peer::spawn(reader, writer);
    let mut world = World {
        id: id.to_string(),
        capability,
        hooks: Hooks::parse(hooks),
        next: 1,
        state,
        ..World::default()
    };
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
            let declared = if capability == Capability::EffortPolicy {
                policy_declarations()
            } else if hooks.bad_declarations {
                bad_declarations()
            } else if hooks.plain_writes {
                plain_declarations()
            } else {
                declarations()
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
            // A policy reads nothing.
            if world.lock().await.capability == Capability::EffortPolicy {
                return Ok(json!({ "entities": [] }));
            }
            Ok(serde_json::to_value(DiscoverResult {
                entities: vec![EntityDecl {
                    name: "work_item".into(),
                    description: "A work item as the fake has it.".into(),
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
            if let Some(path) = world.lock().await.hooks.started_file.clone() {
                let _ = std::fs::write(path, p.command.as_bytes());
            }
            slow(world).await;
            let hooks = world.lock().await.hooks.clone();
            // Under `plain-writes` a key means nothing; `forget-keys`
            // drops it (breaking the promise it declares).
            let sent_key = p.idempotency_key.clone();
            let key = p
                .idempotency_key
                .filter(|_| !hooks.plain_writes && !hooks.forget_keys);
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
            let read = if hooks.host_read && matches!(p.command.as_str(), "estimate" | "update") {
                Some(
                    peer.request(
                        method::HOST_CALL,
                        json!({
                            "key": sent_key,
                            "scope": "sql.read",
                            "args": { "sql": "SELECT 7 AS n" },
                        }),
                    )
                    .await?,
                )
            } else {
                None
            };
            let policy = world.lock().await.capability == Capability::EffortPolicy;
            let mut answer = match (policy, p.command.as_str()) {
                (true, "react") if hooks.bogus_react => {
                    world.lock().await.reacts += 1;
                    json!({ "result": { "commands": [{ "name": "oxplow.nope.never", "input": {} }] },
                            "events": [] })
                }
                (true, "react") => {
                    let result = policy::react(peer, sent_key.clone(), &p.input).await?;
                    world.lock().await.reacts += 1;
                    json!({ "result": result, "events": [] })
                }
                (true, other) => {
                    return Err(ProtocolError::InvalidInput {
                        field: "/command".into(),
                        message: format!("an effort policy answers `react`, not `{other}`"),
                    })
                }
                (false, _) => invoke(world, &p.command, p.input.clone()).await?,
            };
            if let Some(read) = read {
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
            if p.collector != "work_items" {
                return Err(ProtocolError::InvalidInput {
                    field: "/collector".into(),
                    message: format!("no collector `{}`", p.collector),
                });
            }
            slow(world).await;
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
            let (hooks, mut rows) = {
                let w = world.lock().await;
                let mut rows: Vec<(u64, Value)> = w
                    .items
                    .values()
                    .filter(|item| item.rev > after)
                    .map(|item| {
                        (
                            item.rev,
                            serde_json::to_value(&item.record).expect("record serializes"),
                        )
                    })
                    .collect();
                rows.sort_by_key(|(rev, _)| *rev);
                (w.hooks.clone(), rows)
            };
            if hooks.bad_record {
                let mut foreign = rows.first().map(|(_, r)| r.clone()).unwrap_or_else(
                    || json!({ "title": "x", "state": "todo", "native_state": "Backlog" }),
                );
                foreign["ref"] = json!("work_item:other:X-1");
                rows.insert(0, (after + 1, foreign));
            }
            if hooks.stale_read {
                for (_, row) in rows.iter_mut() {
                    let title = row["title"].as_str().unwrap_or_default().to_string();
                    row["title"] = json!(format!("stale {title}"));
                }
            }
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
                    json!({ "id": id, "entity": "work_item", "row": row }),
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

/// The number of instance `id`'s item `item_ref`.
fn number_of(id: &str, item_ref: &str) -> Result<u64, ProtocolError> {
    item_ref
        .strip_prefix(&format!("work_item:{id}:W-"))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: "/ref".into(),
            message: format!(
                "`{item_ref}` isn't one of `{id}`'s work items (work_item:{id}:W-<n>)"
            ),
        })
}

fn str_field(input: &Value, field: &str) -> Result<String, ProtocolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: format!("/{field}"),
            message: "required".into(),
        })
}

fn recorded(record: &WorkItemRecord) -> EventDraft {
    EventDraft {
        event_type: WorkItemRecorded::TYPE.into(),
        v: WorkItemRecorded::V,
        payload: json!({ "item": record }),
        subject: vec![record.item_ref.clone()],
    }
}

/// The canonical state `input` asks for (`field` and `native_state`),
/// checked against each other: a fake state must map to the canonical
/// one. `None` when neither is given.
fn requested_state(input: &Value, field: &str) -> Result<Option<CanonicalState>, ProtocolError> {
    let canonical = match input.get(field).and_then(Value::as_str) {
        None => None,
        Some(raw) => Some(
            serde_json::from_value::<CanonicalState>(Value::String(raw.into())).map_err(|_| {
                ProtocolError::InvalidInput {
                    field: format!("/{field}"),
                    message: format!("`{raw}` isn't a canonical state"),
                }
            })?,
        ),
    };
    let native = match input.get("native_state").and_then(Value::as_str) {
        None => None,
        Some(raw) => Some(
            canonical_of(raw).ok_or_else(|| ProtocolError::InvalidInput {
                field: "/native_state".into(),
                message: format!("`{raw}` isn't a fake state"),
            })?,
        ),
    };
    match (canonical, native) {
        (Some(c), Some(n)) if c != n => Err(ProtocolError::InvalidInput {
            field: "/native_state".into(),
            message: format!("`{}` is {}, not {}", native_of(n), n.as_str(), c.as_str()),
        }),
        (c, n) => Ok(c.or(n)),
    }
}

fn points_of(input: &Value) -> Option<i64> {
    input
        .get("native")
        .and_then(|n| n.get("points"))
        .and_then(Value::as_i64)
}

async fn invoke(world: &Shared, command: &str, input: Value) -> Result<Value, ProtocolError> {
    let mut w = world.lock().await;
    let mut inverse = None;
    let (result, events) = match command {
        "create" => {
            let title = str_field(&input, "title")?;
            let parent_ref = input
                .get("parent_ref")
                .and_then(Value::as_str)
                .map(str::to_string);
            if let Some(parent) = &parent_ref {
                let n = number_of(&w.id, parent)?;
                if !w.items.get(&n).is_some_and(|i| !i.record.deleted) {
                    return Err(ProtocolError::InvalidInput {
                        field: "/parent_ref".into(),
                        message: format!("no item `{parent}`"),
                    });
                }
            }
            let state = requested_state(&input, "state")?.unwrap_or(CanonicalState::Todo);
            let n = w.next;
            w.next += 1;
            let mut native = json!({ "team": "fake" });
            if let Some(points) = points_of(&input) {
                native["points"] = points.into();
            }
            let record = WorkItemRecord {
                item_ref: format!("work_item:{}:W-{n}", w.id),
                title,
                body: input
                    .get("body")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .into(),
                state,
                native_state: native_of(state).into(),
                native,
                parent_ref,
                deleted: false,
                rank: None,
                // It keeps its links and comments, so every record states
                // them.
                links: Some(Vec::new()),
                comments: Some(Vec::new()),
                // It keeps no lists: the item stays where it was filed.
                list: None,
            };
            let events = vec![recorded(&record)];
            let result = json!({ "ref": record.item_ref });
            w.rev += 1;
            let rev = w.rev;
            w.items.insert(n, Item { record, rev });
            (result, events)
        }
        "update" | "transition" | "link" | "comment" | "delete" | "estimate" => {
            let item_ref = str_field(&input, "ref")?;
            let n = number_of(&w.id, &item_ref)?;
            w.rev += 1;
            let rev = w.rev;
            let item = w
                .items
                .get_mut(&n)
                .filter(|item| !item.record.deleted)
                .ok_or_else(|| ProtocolError::InvalidInput {
                    field: "/ref".into(),
                    message: format!("no item `{item_ref}`"),
                })?;
            let mut result = json!({ "ref": item_ref });
            match command {
                "update" => {
                    if let Some(t) = input.get("title").and_then(Value::as_str) {
                        item.record.title = t.into();
                    }
                    if let Some(b) = input.get("body").and_then(Value::as_str) {
                        item.record.body = b.into();
                    }
                    if let Some(p) = input.get("parent_ref").and_then(Value::as_str) {
                        item.record.parent_ref = (!p.is_empty()).then(|| p.to_string());
                    }
                    if let Some(state) = requested_state(&input, "state")? {
                        item.record.state = state;
                        item.record.native_state = native_of(state).into();
                    }
                    if let Some(points) = points_of(&input) {
                        item.record.native["points"] = points.into();
                    }
                }
                "transition" => {
                    let state = requested_state(&input, "to")?.ok_or_else(|| {
                        ProtocolError::InvalidInput {
                            field: "/to".into(),
                            message: "required".into(),
                        }
                    })?;
                    inverse = Some(CommandCall {
                        command: "transition".into(),
                        input: json!({
                            "ref": item_ref,
                            "to": item.record.state.as_str(),
                            "native_state": item.record.native_state,
                        }),
                    });
                    item.record.state = state;
                    item.record.native_state = native_of(state).into();
                }
                "link" => {
                    item.record
                        .links
                        .get_or_insert_with(Vec::new)
                        .push(LinkRecord {
                            target: str_field(&input, "target")?,
                            link_type: str_field(&input, "link_type")?,
                        });
                }
                "comment" => {
                    let comments = item.record.comments.get_or_insert_with(Vec::new);
                    let id = format!("c{}", comments.len() + 1);
                    // The answer names it: the list's own id for the comment.
                    result["comment"] = Value::String(id.clone());
                    comments.push(CommentRecord {
                        id,
                        body: str_field(&input, "body")?,
                        author: None,
                        created_at: None,
                    });
                }
                "estimate" => {
                    let points = input.get("points").and_then(Value::as_i64).ok_or_else(|| {
                        ProtocolError::InvalidInput {
                            field: "/points".into(),
                            message: "required".into(),
                        }
                    })?;
                    item.record.native["points"] = points.into();
                }
                _ => item.record.deleted = true,
            }
            // Only a write that happened moves its revision.
            item.rev = rev;
            let events = vec![recorded(&item.record)];
            (result, events)
        }
        other => {
            return Err(ProtocolError::InvalidInput {
                field: "/command".into(),
                message: format!("no command `{other}`"),
            })
        }
    };
    Ok(serde_json::to_value(InvokeResult {
        result,
        events,
        inverse,
    })
    .expect("invoke result serializes"))
}
