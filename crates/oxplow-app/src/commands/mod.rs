//! The command bus: the one write path (`.context/commands.md`).
//!
//! Every command runs the same pipeline — validate the input against the
//! spec's schema, check the invoker, apply the agent policy, require a
//! person's confirmation when the spec says so, run the handler, and
//! record `command.executed` in the event log plus a `command_audit` row
//! **in one transaction**. `undo(audit_id)` runs the recorded inverse
//! through the same pipeline.
//!
//! A handler is either `Tx` (runs inside the bus's transaction, so its
//! writes, the event and the audit commit together) or `External` (runs
//! outside it, against a system the bus doesn't own — a VCS, a provider
//! process, a gauge script — and is audited after it returns).
//! [`CommandBus::external_commands`] is pinned by a test, so every
//! `External` command is a reviewed choice, not a shortcut.

pub mod agent_reads;
pub mod agent_session;
pub mod agent_settings;
pub mod ai_settings;
pub mod bookmark;
pub mod comment;
pub mod compose;
pub mod config_commands;
pub mod credentials;
pub mod dashboard;
pub mod delivery;
pub mod effect;
pub mod effort;
pub mod effort_report;
pub mod extension_configuration;
pub mod extension_deploy;
pub mod extension_jobs;
pub mod extension_state;
pub mod file;
pub mod handoff;
pub mod hint;
pub mod lens;
pub mod lsp;
pub mod metric;
pub mod note;
pub mod op_error;
pub mod ops;
mod proposals;
pub mod provider_instances;
pub mod reasoning;
mod record;
pub mod review;
pub mod snapshot;
mod steps;
pub mod stream;
pub mod test_runs;
pub mod thread;
pub mod ui;
mod undo;
pub mod util;
pub mod vcs;
pub mod work_item;

use oxplow_domain::run::{Mode, RunContext};
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use oxplow_db::command_audit_store::{insert_tx, mark_undone_tx, set_event_id_tx, NewCommandAudit};
use oxplow_db::event_log_store::append_tx;
use oxplow_db::proposal_store::{self, NewProposal};
use oxplow_db::{
    CommandAudit, Database, ProposalDecision, SqliteCommandAuditStore, SqliteEventLogStore,
    SqliteProposalStore, TxError,
};
use oxplow_domain::events::schema::{
    CommandApproved, CommandApprovedV2, CommandDeclined, CommandDeclinedV2, CommandExecuted,
    CommandExecutedV4, CommandOutcome as Outcome, CommandProposed, CommandProposedV3,
};
use oxplow_domain::uri::build::{command_uri, proposal_uri};
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandOutcome, CommandSpec, Envelope,
    InputValidator, Preview,
};
use oxplow_runtime::policy::PolicyDecision;
use parking_lot::RwLock;
use serde::Serialize;
use serde_json::Value;

use crate::agent_policy::AgentPolicy;
use crate::event_pump::EventPump;
use proposals::log_approved_tx;
use record::{command_error, finish, record_tx, unrecorded, Abort, Executed, Recorded};
use undo::{finish_undo_claim_tx, lost_race};

/// What a handler produced.
#[derive(Default)]
pub struct HandlerOutput {
    /// Returned to the caller (`run_command`'s result).
    pub result: Value,
    /// The call that undoes this run; `None` for a command that declares
    /// `undoable: false`, or when there is nothing to undo.
    pub inverse: Option<CommandCall>,
    /// Domain events the run produced, appended after `command.executed`
    /// in the same transaction. An `External` handler's own transactions
    /// log their events themselves; this is for `Tx` handlers.
    pub events: Vec<Envelope>,
    /// Runs once the run has committed: the in-memory broadcast that wakes
    /// the UI (`OxplowEvent`), and side effects outside the database that
    /// mirror the committed change (`oxplow.config.set` writes project.yaml here).
    /// Never a database write, and it must tolerate failure — the run is
    /// already recorded. The handler itself must stay pure: it can run
    /// more than once (`Database::transaction` retries on SQLITE_BUSY).
    pub after_commit: Option<Box<dyn FnOnce() + Send + Sync>>,
    /// The run found nothing to change (a re-located comment anchor
    /// already where it was). A `Tx` call that says so leaves no record —
    /// no audit row, no `command.executed` — like a read, and its
    /// transaction is rolled back, so nothing it wrote lands unaudited; one
    /// that also returns events or an inverse (a change) is refused
    /// (tsk901). An undo, an approval or an effect's reaction is recorded
    /// anyway, since its row must be marked. Honoured for `Tx` handlers
    /// only: an `External` one's work is outside the database, so it is
    /// always recorded.
    pub unchanged: bool,
}

/// One child of a composite run (`steps::run_composed`): what ran,
/// with what, what it returned and how it undoes.
#[derive(Debug, Clone, Serialize)]
pub struct NestedChild {
    pub name: String,
    pub input: Value,
    pub result: Value,
    pub inverse: Option<CommandCall>,
}

/// What a `Tx` handler runs with: the run's transaction, the actor, and
/// the context its store cores log events through — the actor's `source`,
/// caused by this run's `command.executed` (whose id is fixed before the
/// handler runs, so the cause can be named; `command.executed` itself is
/// appended after the handler, so a core's events precede it in `seq`).
pub struct TxCtx<'a> {
    pub conn: &'a rusqlite::Connection,
    /// The run: who runs it, what caused it, where it sits.
    pub run: &'a RunContext,
    pub events: oxplow_db::EventCtx<'a>,
    /// Step 4's answer: a person confirmed this call. A handler that
    /// learns only while running that a confirmation is needed — a
    /// composite whose child asks (`steps::run_composed`) — checks it
    /// and raises `NeedsConfirmation`, which the bus treats as step 4
    /// would: rolled back, nothing audited.
    pub confirmed: bool,
    /// Step 3's gate: whether the agent's thread may write (it is open);
    /// `None` for a person, the system, or a command the gate doesn't
    /// apply to. A composite applies it to each child it runs.
    pub may_write: Option<bool>,
    /// How many composites this run is inside: 0 for the bus's own call,
    /// one more for each nested composite. Past [`MAX_NESTING`] a
    /// composition is refused rather than recursing until the stack
    /// overflows (a command that composes itself).
    pub depth: usize,
    /// The scopes the run has called: shared by the runs
    /// nested in it, recorded on its audit row.
    pub trace: &'a oxplow_domain::scope::ScopeTrace,
}

/// The deepest a composite may nest (`oxplow.command.sequence` and extension
/// commands composing one another).
pub const MAX_NESTING: usize = 8;

impl TxCtx<'_> {
    /// Who runs it.
    pub fn actor(&self) -> &Actor {
        self.run.actor()
    }
}

pub type TxHandler = dyn Fn(&TxCtx<'_>, Value) -> Result<HandlerOutput, CommandError> + Send + Sync;
pub type ExternalFuture = Pin<Box<dyn Future<Output = Result<HandlerOutput, CommandError>> + Send>>;
pub type ExternalHandler = dyn Fn(Invocation, Value) -> ExternalFuture + Send + Sync;

/// Who runs an `External` handler, and the write's idempotency key (P10,
/// `.context/providers.md` "Idempotency"): for a step of an effect's
/// reaction, the same on every attempt at it ([`effect_step_key`]), so a
/// provider that keeps `idempotent_writes` does it once. `None` for any
/// other run: the system the handler calls mints one per call where it
/// re-sends (`providers::Instance::invoke`).
#[derive(Debug, Clone)]
pub struct Invocation {
    /// The call's run — a child of the command's run, so the scopes it
    /// calls on its way (a provider's `host/call`s) are counted on its
    /// trace and recorded with the command's run.
    pub run: RunContext,
    pub idempotency_key: Option<String>,
    /// A person confirmed this call. An External handler that finds it
    /// needs a person's word (a deploy that adds permissions) raises
    /// `NeedsConfirmation` while this is false: a person is asked, an
    /// agent's call becomes a proposal.
    pub confirmed: bool,
}

impl Invocation {
    /// Who runs it.
    pub fn actor(&self) -> &Actor {
        self.run.actor()
    }
}

/// The idempotency key of step `index` (calling `call` with `input`) of
/// the reaction `run`: `effect:<effect>:<event id>:<index>:<hash>`. Every
/// attempt at the reaction composes it again, so the step's position
/// alone isn't enough — the hash of what it calls is part of it.
pub fn effect_step_key(
    run: &oxplow_db::effect_run_store::EffectRunKey,
    index: usize,
    call: &str,
    input: &Value,
) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(call.as_bytes());
    hash.update([0]);
    hash.update(input.to_string().as_bytes());
    let digest = hash.finalize();
    let short: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!(
        "{}:{}:{index}:{short}",
        oxplow_domain::uri::build::effect_source(&run.effect),
        run.event_id
    )
}

#[derive(Clone)]
/// A run of an extension's command backed by one of its provider's
/// operations (`provider:` + `op:` in its manifest).
pub struct ProviderCall {
    pub extension: String,
    /// The provider's id in the extension.
    pub provider: String,
    /// The operation, as its declarations name it.
    pub op: String,
    /// The command's input: a `uri` to one of an instance's items, or an
    /// `instance`, says which instance runs it.
    pub input: Value,
    pub invocation: Invocation,
}

/// Runs a provider's operation on the instance a call names: the provider
/// registry (`providers::registry`).
pub trait ProviderRouter: Send + Sync {
    fn run(&self, call: ProviderCall) -> ExternalFuture;
}

#[derive(Clone)]
pub enum Handler {
    /// Runs inside the bus's transaction.
    Tx(Arc<TxHandler>),
    /// Runs against a system the bus doesn't own; the bus audits it after
    /// it returns.
    External(Arc<ExternalHandler>),
    /// A composite (`compose.rs`): says which calls an input runs; the bus
    /// runs them in its transaction when every one can, else as steps.
    Compose(Arc<compose::Compose>),
}

/// What the bus has once a run's admission is done: the outcome, or an
/// External handler still to run, outside the generation.
enum Flow {
    Done(CommandOutcome),
    External(Box<ExternalRun>),
}

/// An admitted External run, owned so it outlives the generation pin.
struct ExternalRun {
    command: Arc<Command>,
    resolved: Resolved,
    /// The run, without the pin its admission held.
    run: RunContext,
    input: Value,
    origin: RunOrigin,
    gates: Gates,
    /// A person confirmed it.
    confirmed: bool,
}

/// What settling a run's outcome needs of it.
struct Settling<'a> {
    run: &'a RunContext,
    command: &'a Arc<Command>,
    resolved: &'a Resolved,
    input: Value,
    origin: RunOrigin,
    gates: Gates,
}

/// A handler with its route decided: what step 5 runs.
#[derive(Clone)]
enum Resolved {
    Tx(Arc<TxHandler>),
    External(Arc<ExternalHandler>),
    /// A composite with a call outside the transaction: its routed calls,
    /// run as steps.
    Steps(Arc<steps::Composed>),
}

/// Step 1's routing: decided from the input alone, or a composite, routed
/// once the actor has been admitted (composing reads the database and may
/// run an extension's script).
enum Routing {
    Ready(Resolved),
    Composite(Arc<compose::Compose>),
}

/// A command with its handler resolved for the input being run.
#[derive(Clone, Copy)]
struct Prepared<'a> {
    command: &'a Arc<Command>,
    resolved: &'a Resolved,
}

pub type ConfirmFor = dyn Fn(&Value) -> oxplow_domain::Confirm + Send + Sync;

/// A command's own check of an input that needs what a transaction can't
/// do — async work, such as resolving a metric query through the metric
/// engine (tsk1010). Run once the actor is admitted, before the
/// transaction opens: for a direct call, and for each call a composite
/// composes (`steps.rs` collects them while routing). A refusal is the
/// command's `Invalid`, at the input's field.
pub type Precheck =
    dyn Fn(Value) -> futures::future::BoxFuture<'static, Result<(), CommandError>> + Send + Sync;

/// A registered command: its spec, compiled input schema and handler.
pub struct Command {
    pub spec: CommandSpec,
    pub handler: Handler,
    /// Compiled once, shared by [`Command::with_handler`]'s copies.
    validator: Arc<InputValidator>,
    /// Per-input confirmation, when the spec's `confirm` depends on the
    /// input (`oxplow.config.set` on a human-only key). Overrides `spec.confirm`.
    confirm_for: Option<Arc<ConfirmFor>>,
    /// Its check before the transaction, if it has one ([`Precheck`]).
    precheck: Option<Arc<Precheck>>,
    /// The extension contribution it is (an extension's `commands:`
    /// entry). The callback supplies its handler's version and generation:
    /// its runs are that contribution's, recorded as its.
    contribution: Option<Arc<ContributionOf>>,
}

/// A command's own run, nested in the run that called it: named for the
/// contribution that owns the handler, with what that contribution may do.
pub type ContributionOf = dyn Fn(&RunContext) -> RunContext + Send + Sync;

impl Command {
    /// This command, offered to a person as `ui` says (label, group, …).
    pub fn with_ui(mut self, ui: oxplow_domain::CommandUi) -> Self {
        self.spec.ui = Some(ui);
        self
    }

    /// This command run by `handler` instead — its spec and compiled
    /// schema shared (an effect's reaction: `oxplow.command.sequence` over what
    /// its script composed). The handler must be of the same atomicity.
    pub fn with_handler(&self, handler: Handler) -> Result<Self, CommandError> {
        let copy = Self {
            spec: self.spec.clone(),
            handler,
            validator: self.validator.clone(),
            confirm_for: self.confirm_for.clone(),
            precheck: self.precheck.clone(),
            contribution: self.contribution.clone(),
        };
        copy.check_atomicity()?;
        Ok(copy)
    }

    #[cfg(test)]
    pub(crate) fn shares_validator(&self, other: &Command) -> bool {
        Arc::ptr_eq(&self.validator, &other.validator)
    }

    pub fn new(mut spec: CommandSpec, handler: Handler) -> Result<Self, CommandError> {
        CommandSpec::validate_id(&spec.id)?;
        if let Some((field, message)) = spec.ui.as_ref().and_then(|ui| ui.problem()) {
            return Err(CommandError::Invalid {
                field: Some(field),
                message: format!("`{}`'s {message}", spec.id),
            });
        }
        if let Some(when) = spec.ui.as_ref().and_then(|ui| ui.when.as_deref()) {
            oxplow_domain::when::check(when).map_err(|message| CommandError::Invalid {
                field: Some("/ui/when".into()),
                message: format!("`{}`'s `when`: {message}", spec.id),
            })?;
        }
        if spec.confirm.required() {
            Self::may_ask(&spec)?;
        }
        // A secret is a kind of value: a top-level input field its schema
        // marks `writeOnly` is never in a record, whatever else declares.
        for field in secret_fields(&spec.input_schema) {
            if !spec.unrecorded.contains(&field) {
                spec.unrecorded.push(field);
            }
        }
        let validator = Arc::new(InputValidator::compile(&spec.input_schema)?);
        let command = Self {
            spec,
            handler,
            validator,
            confirm_for: None,
            precheck: None,
            contribution: None,
        };
        command.check_atomicity()?;
        Ok(command)
    }

    /// Its handler is of the atomicity its spec declares.
    fn check_atomicity(&self) -> Result<(), CommandError> {
        let declared = match self.handler {
            Handler::Tx(_) => Atomicity::Tx,
            Handler::External(_) => Atomicity::External,
            Handler::Compose(_) => Atomicity::Dispatch,
        };
        if declared != self.spec.atomicity {
            return Err(CommandError::Invalid {
                field: Some("/atomicity".into()),
                message: format!(
                    "`{}` declares {:?} but its handler is {:?}",
                    self.spec.id, self.spec.atomicity, declared
                ),
            });
        }
        Ok(())
    }

    /// Its confirmation decided per input. A command that only reads may not have
    /// one (see [`Self::may_ask`]).
    /// This command as `contribution`'s: its runs are recorded as that
    /// contribution's.
    pub fn of_contribution(mut self, contribution: Arc<ContributionOf>) -> Self {
        self.contribution = Some(contribution);
        self
    }

    /// This command with its check before the transaction ([`Precheck`]).
    pub fn with_precheck(mut self, f: Arc<Precheck>) -> Self {
        self.precheck = Some(f);
        self
    }

    pub fn with_confirm_for(mut self, f: Arc<ConfirmFor>) -> Result<Self, CommandError> {
        Self::may_ask(&self.spec)?;
        self.confirm_for = Some(f);
        Ok(self)
    }

    /// A command that only reads is never asked about: it runs without a
    /// record, so nothing would resolve its proposal or carry its
    /// confirmation.
    fn may_ask(spec: &CommandSpec) -> Result<(), CommandError> {
        if spec.access.reads_only() {
            return Err(CommandError::Invalid {
                field: Some("/confirm".into()),
                message: format!("`{}` only reads, so it can't need a confirmation", spec.id),
            });
        }
        Ok(())
    }

    fn confirm(&self, input: &Value) -> oxplow_domain::Confirm {
        match &self.confirm_for {
            Some(f) => f(input),
            None => self.spec.confirm,
        }
    }

    /// Its handler: a composite's routing deferred until the actor is
    /// admitted, the others as they are.
    fn resolve(&self) -> Routing {
        Routing::Ready(match &self.handler {
            Handler::Tx(h) => Resolved::Tx(h.clone()),
            Handler::External(h) => Resolved::External(h.clone()),
            Handler::Compose(c) => return Routing::Composite(c.clone()),
        })
    }
}

/// May this agent thread write? (`Thread::is_open()` in production: any
/// open thread may change its worktree.) The bus asks before an agent runs
/// a `Write` command.
pub type WriteGate =
    Arc<dyn Fn(oxplow_domain::ThreadId) -> futures::future::BoxFuture<'static, bool> + Send + Sync>;

/// What a run is, beyond the call itself: the undo of an audited run, or
/// the approval of a proposal. Either is marked in the run's own
/// transaction (a `Tx` handler) or claimed before it (an `External` one).
impl crate::extensions::RunningCommands for CommandBus {
    fn input_schema(&self, name: &str) -> Option<Value> {
        CommandBus::input_schema(self, name)
    }

    fn namespace_owner(&self, namespace: &str) -> Option<String> {
        CommandBus::namespace_owner(self, namespace)
    }
}

/// One source's replacement prepared by the owning host. An empty command
/// set removes that source. Each source may appear once in an atomic batch.
pub struct NamespaceCommands {
    pub namespace: String,
    pub source: String,
    pub commands: Vec<Command>,
}

/// Why a batch of namespace replacements was refused: the source whose
/// commands didn't fit, the source it collided with (if one did), and the
/// error.
#[derive(Debug)]
pub struct NamespaceRefusal {
    pub source: String,
    pub holder: Option<String>,
    pub error: CommandError,
}

/// Validated command registry candidate. It carries the registry identity it
/// was prepared against; publication cannot overwrite intervening changes or
/// another bus. No active handler changes while this value is held.
#[derive(Clone)]
pub struct PreparedNamespaces {
    base: Arc<()>,
    candidate: Registry,
}

pub(crate) struct NamespaceReservation<'a> {
    registry: parking_lot::RwLockWriteGuard<'a, Registry>,
    candidate: Option<Registry>,
}

impl NamespaceReservation<'_> {
    /// Commit the matching database candidate before this infallible swap.
    pub(crate) fn publish(mut self) {
        self.apply();
    }

    pub(crate) fn apply(&mut self) {
        *self.registry = self
            .candidate
            .take()
            .expect("namespace reservation applied once");
    }
}

/// Who registered a command: core itself.
pub const CORE_SOURCE: &str = "core";

/// May `source` register under the reserved `oxplow` namespace? Core and
/// the extensions that ship with oxplow — nothing else.
fn ships_with_oxplow(source: &str) -> bool {
    source == CORE_SOURCE
        || oxplow_domain::uri::build::id_of(source, oxplow_domain::uri::resources::EXTENSION)
            .is_ok_and(|name| crate::bundled_extensions::find(&name).is_some())
}

/// The registered commands by id, who registered each (its source:
/// `core`, `oxplow:oxplow/extension?name=<name>`, `provider:<name>`), and who holds each
/// namespace: `oxplow` for the reserved one — shared by core and oxplow's
/// own extensions, collisions checked per id — else the one source that
/// registered it.
#[derive(Clone, Default)]
struct Registry {
    /// An allocation identity avoids wraparound and ABA while candidates retain
    /// their base. It changes on every successful registry mutation.
    revision: Arc<()>,
    commands: BTreeMap<String, Arc<Command>>,
    sources: BTreeMap<String, String>,
    holders: BTreeMap<String, String>,
}

impl Registry {
    fn owner(&self, namespace: &str) -> Option<String> {
        self.holders.get(namespace).cloned()
    }

    /// Add `commands` from `source` under `namespace`, all or none.
    fn add(
        &mut self,
        namespace: &str,
        source: &str,
        commands: Vec<Command>,
    ) -> Result<(), CommandError> {
        let invalid = |field: Option<&str>, message: String| CommandError::Invalid {
            field: field.map(str::to_string),
            message,
        };
        let reserved = namespace == oxplow_domain::OXPLOW_NAMESPACE;
        if reserved && !ships_with_oxplow(source) {
            return Err(invalid(
                None,
                format!(
                    "the command namespace `{namespace}` is reserved for oxplow's own commands"
                ),
            ));
        }
        match self.holders.get(namespace) {
            Some(held) if !reserved && held != source => {
                return Err(invalid(
                    None,
                    format!("the command namespace `{namespace}` is already {held}'s"),
                ));
            }
            _ => {}
        }
        let mut batch_ids = std::collections::BTreeSet::new();
        for command in &commands {
            let id = &command.spec.id;
            if !batch_ids.insert(id) {
                return Err(invalid(
                    Some("/name"),
                    format!("command `{id}` is declared more than once"),
                ));
            }
            CommandSpec::validate_id(id).map_err(|e| invalid(Some("/name"), e.to_string()))?;
            if oxplow_domain::namespace_of(id) != namespace {
                return Err(invalid(
                    Some("/name"),
                    format!("`{id}` isn't under `{namespace}`"),
                ));
            }
            if self.commands.contains_key(id) {
                return Err(invalid(
                    Some("/name"),
                    format!("command `{id}` is already registered"),
                ));
            }
            // One key may run several commands told apart by `when`; two
            // under the same `when` always collide.
            if let Some(key) = shortcut_of(&command.spec) {
                let held = self
                    .commands
                    .values()
                    .map(|c| &c.spec)
                    .chain(commands.iter().map(|c| &c.spec).filter(|s| s.id != *id))
                    .find(|other| shortcut_of(other) == Some(key.clone()));
                if let Some(other) = held {
                    return Err(invalid(
                        Some("/ui/shortcut"),
                        format!(
                            "`{}` already runs on {} under the same `when`; give `{id}` another \
                             key, or a `when` that tells them apart",
                            other.id, key.0
                        ),
                    ));
                }
            }
        }
        let holder = if reserved {
            oxplow_domain::OXPLOW_NAMESPACE
        } else {
            source
        };
        self.holders
            .insert(namespace.to_string(), holder.to_string());
        for command in commands {
            self.sources
                .insert(command.spec.id.clone(), source.to_string());
            self.commands
                .insert(command.spec.id.clone(), Arc::new(command));
        }
        self.revision = Arc::new(());
        Ok(())
    }

    fn get(&self, name: &str) -> Option<&Arc<Command>> {
        self.commands.get(name)
    }

    fn remove_source(&mut self, source: &str) -> Vec<String> {
        let ids: Vec<String> = self
            .sources
            .iter()
            .filter(|(_, registered)| registered.as_str() == source)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &ids {
            self.commands.remove(id);
            self.sources.remove(id);
        }
        let held: std::collections::BTreeSet<_> = self
            .commands
            .keys()
            .map(|id| oxplow_domain::namespace_of(id).to_string())
            .collect();
        self.holders.retain(|namespace, _| held.contains(namespace));
        self.revision = Arc::new(());
        ids
    }

    fn replacing(&self, replacements: Vec<NamespaceCommands>) -> Result<Self, NamespaceRefusal> {
        let mut sources = std::collections::BTreeSet::new();
        for replacement in &replacements {
            if !sources.insert(&replacement.source) {
                return Err(NamespaceRefusal {
                    source: replacement.source.clone(),
                    holder: None,
                    error: CommandError::Invalid {
                        field: Some("/source".into()),
                        message: "a command source may be replaced only once in a batch".into(),
                    },
                });
            }
        }
        let mut candidate = self.clone();
        for replacement in &replacements {
            candidate.remove_source(&replacement.source);
        }
        for replacement in replacements {
            if replacement.commands.is_empty() {
                continue;
            }
            // Who it would collide with: the namespace's holder, else the
            // source of an id it repeats.
            let holder = candidate
                .holders
                .get(&replacement.namespace)
                .filter(|h| **h != replacement.source && *h != oxplow_domain::OXPLOW_NAMESPACE)
                .cloned()
                .or_else(|| {
                    replacement
                        .commands
                        .iter()
                        .find_map(|c| candidate.sources.get(&c.spec.id).cloned())
                });
            let source = replacement.source.clone();
            candidate
                .add(&replacement.namespace, &source, replacement.commands)
                .map_err(|error| NamespaceRefusal {
                    source,
                    holder,
                    error,
                })?;
        }
        // Even an empty publication receives a distinct identity.
        candidate.revision = Arc::new(());
        Ok(candidate)
    }
}

/// A command's shortcut with the `when` it holds under (`Ctrl/Cmd+K`,
/// case aside): two equal ones always collide.
fn shortcut_of(spec: &CommandSpec) -> Option<(String, Option<String>)> {
    let ui = spec.ui.as_ref()?;
    let key = ui.shortcut.as_ref()?;
    Some((
        key.to_lowercase(),
        ui.when
            .as_ref()
            .map(|w| w.split_whitespace().collect::<String>()),
    ))
}

/// Step 3's answers for a run, as its `TxCtx` carries them: may it claim
/// the worktree, and may the agent's thread write (`None` when the gate
/// doesn't apply).
#[derive(Debug, Clone, Copy)]
struct Gates {
    may_write: Option<bool>,
}

#[derive(Debug, Clone)]
enum RunOrigin {
    Call,
    Undo(i64),
    Approval(i64),
    /// An extension's effect reacting to an event (P8.D10): the run's
    /// `effect_run` row and `effect.result` land with it, and its
    /// `command.executed` is caused by the event. With what it composed
    /// when every step is safe to send again, kept with its claim
    /// (tsk954).
    Effect(
        Arc<oxplow_db::effect_run_store::EffectRunKey>,
        Option<Arc<str>>,
    ),
}

impl RunOrigin {
    /// What the run's `command.executed` is caused by: the event an
    /// effect reacted to, else what caused the run (an effort policy's
    /// event, a collector run's trigger).
    fn cause(&self, run: &RunContext) -> Option<oxplow_domain::EventId> {
        match self {
            RunOrigin::Effect(key, _) => Some(oxplow_domain::EventId(key.event_id.clone())),
            _ => run.cause().cloned(),
        }
    }
    /// How an `External` call at `index` of `run` is invoked: an
    /// effect's step carries its key.
    fn invocation(&self, run: &RunContext, index: usize, call: &str, input: &Value) -> Invocation {
        Invocation {
            run: run.child(None, Mode::External),
            idempotency_key: match self {
                RunOrigin::Effect(key, _) => Some(effect_step_key(key, index, call, input)),
                _ => None,
            },
            confirmed: false,
        }
    }
}

pub struct CommandBus {
    db: Database,
    log: SqliteEventLogStore,
    audit: SqliteCommandAuditStore,
    proposals: SqliteProposalStore,
    policy: Arc<AgentPolicy>,
    pump: Arc<EventPump>,
    commands: RwLock<Registry>,
    /// The scopes' operations: what a command declared in a
    /// manifest is backed by (`ops.rs`).
    ops: RwLock<ops::Ops>,
    /// Where an extension's provider command runs (`ProviderRouter`):
    /// the provider registry, set once it exists.
    providers: std::sync::OnceLock<std::sync::Weak<dyn ProviderRouter>>,
    write_gate: Option<WriteGate>,
    /// What a composer reads through (`compose::Composing`): the
    /// project's gateway once `with_sql_gateway` gives it; a bare one over
    /// `db` until then.
    sql: crate::sql_gateway::SqlGateway,
    /// What's active, for what a command needs or which implementation
    /// owns it (`capabilities::Active::refusal`); `None` offers everything.
    capabilities: Option<(
        Arc<crate::capabilities::CapabilityRegistry>,
        Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
    )>,
    /// The project's activation boundary; a bus built alone has its own,
    /// which nothing has activated.
    activation: Arc<crate::extension_activation::ExtensionActivation>,
}

impl CommandBus {
    pub fn new(
        db: Database,
        log: SqliteEventLogStore,
        policy: Arc<AgentPolicy>,
        pump: Arc<EventPump>,
    ) -> Self {
        let sql_db = db.clone();
        Self {
            audit: SqliteCommandAuditStore::new(db.clone()),
            proposals: SqliteProposalStore::new(db.clone()),
            db,
            log,
            policy,
            pump,
            commands: RwLock::new(Registry::default()),
            ops: RwLock::new(ops::Ops::default()),
            providers: std::sync::OnceLock::new(),
            write_gate: None,
            sql: crate::sql_gateway::SqlGateway::new(sql_db),
            capabilities: None,
            activation: Arc::new(crate::extension_activation::ExtensionActivation::new()),
        }
    }

    pub fn with_activation(
        mut self,
        activation: Arc<crate::extension_activation::ExtensionActivation>,
    ) -> Self {
        self.activation = activation;
        self
    }

    /// Offer and run a command only while what it needs is active and,
    /// when an implementation owns it, while that one is the active one.
    pub fn with_capabilities(
        mut self,
        registry: Arc<crate::capabilities::CapabilityRegistry>,
        config: Arc<std::sync::RwLock<oxplow_config::OxplowConfig>>,
    ) -> Self {
        self.capabilities = Some((registry, config));
        self
    }

    /// What's active now, when the bus knows.
    pub(super) fn active(&self) -> Option<crate::capabilities::Active> {
        self.capabilities.as_ref().map(|(registry, config)| {
            registry.snapshot(&crate::config_service::read_config(config))
        })
    }

    /// Consult `gate` before an agent thread runs a `Write` command: a
    /// closed thread can read but not change anything.
    pub fn with_write_gate(mut self, gate: WriteGate) -> Self {
        self.write_gate = Some(gate);
        self
    }

    /// Compose through `sql`: the project's gateway, whose metric
    /// functions, extension state, configuration and jobs a composer's
    /// script reads.
    pub fn with_sql_gateway(mut self, sql: crate::sql_gateway::SqlGateway) -> Self {
        self.sql = sql;
        self
    }

    /// Register one of core's commands (under `oxplow`). A second command
    /// with the same id is refused: two handlers for one id is a bug, not
    /// an override.
    pub fn register(&self, command: Command) -> Result<(), CommandError> {
        let namespace = oxplow_domain::namespace_of(&command.spec.id).to_string();
        self.commands
            .write()
            .add(&namespace, CORE_SOURCE, vec![command])
    }

    /// Register `commands` from `source` (`oxplow:oxplow/extension?name=<name>`,
    /// `provider:<name>`) under `namespace` — all of them or none, under
    /// one lock. Refused when the namespace is another source's, when it is
    /// the reserved `oxplow` and `source` doesn't ship with oxplow, or when
    /// an id is taken or isn't under it.
    pub fn register_namespace(
        &self,
        namespace: &str,
        source: &str,
        commands: Vec<Command>,
    ) -> Result<(), CommandError> {
        self.commands.write().add(namespace, source, commands)
    }

    /// Replace this source's commands through one registry swap. Rejection
    /// preserves the prior commands and namespace ownership. Empty removes
    /// the source. Already resolved commands retain their Arc handlers.
    pub fn replace_namespace(
        &self,
        namespace: &str,
        source: &str,
        commands: Vec<Command>,
    ) -> Result<(), CommandError> {
        self.replace_namespaces(vec![NamespaceCommands {
            namespace: namespace.to_owned(),
            source: source.to_owned(),
            commands,
        }])
    }

    /// Validate and swap several sources together. All prior source holdings
    /// are removed from the candidate before additions, so sources can exchange
    /// namespaces, IDs and shortcuts without a transient collision. A rejection
    /// preserves the entire active registry; already resolved handlers survive.
    pub fn replace_namespaces(
        &self,
        replacements: Vec<NamespaceCommands>,
    ) -> Result<(), CommandError> {
        let mut registry = self.commands.write();
        *registry = registry.replacing(replacements).map_err(|r| r.error)?;
        Ok(())
    }

    /// Validate a complete replacement off the active registry's lock. All
    /// retained and prepared handlers stay available if validation fails.
    pub fn prepare_namespaces(
        &self,
        replacements: Vec<NamespaceCommands>,
    ) -> Result<PreparedNamespaces, NamespaceRefusal> {
        let registry = self.commands.read().clone();
        Ok(PreparedNamespaces {
            base: registry.revision.clone(),
            candidate: registry.replacing(replacements)?,
        })
    }

    /// Publish only on the exact registry used for validation. A host must
    /// restage after any intervening registration, replacement or removal.
    /// This is only an in-memory publication, never a DB commit boundary.
    pub fn publish_namespaces(&self, prepared: PreparedNamespaces) -> Result<(), CommandError> {
        self.reserve_namespaces(&prepared)?.publish();
        Ok(())
    }

    pub(crate) fn reserve_namespaces(
        &self,
        prepared: &PreparedNamespaces,
    ) -> Result<NamespaceReservation<'_>, CommandError> {
        let registry = self.commands.write();
        if !Arc::ptr_eq(&registry.revision, &prepared.base) {
            return Err(CommandError::Unavailable {
                message: "the command registry changed while its replacement was staged".into(),
                retry_after_ms: None,
            });
        }
        Ok(NamespaceReservation {
            registry,
            candidate: Some(prepared.candidate.clone()),
        })
    }

    /// Add a scope's operation (`ops.rs`): refused when its scope
    /// isn't in the catalog or the op is already there.
    pub fn add_op(&self, op: ops::Op) -> Result<(), CommandError> {
        self.ops.write().add(op)
    }

    /// Run extension commands backed by a provider's operation through
    /// `router` (the provider registry).
    pub fn set_provider_router(&self, router: std::sync::Weak<dyn ProviderRouter>) {
        let _ = self.providers.set(router);
    }

    /// The provider router, while it's there.
    pub fn provider_router(&self) -> Option<Arc<dyn ProviderRouter>> {
        self.providers.get().and_then(std::sync::Weak::upgrade)
    }

    /// The command declared over a provider's operation `op` (its
    /// `CommandSpec::op`): what a provider's inverse naming an operation is
    /// a call of. There is one — an extension declares one command per
    /// provider operation (`extension_commands::parse_commands`) — where a
    /// scope's operation may back several (`project.open`,
    /// `project.open_in_new_window`), so it isn't asked for those.
    pub fn command_for_op(&self, op: &oxplow_domain::OpRef) -> Option<String> {
        self.commands
            .read()
            .commands
            .values()
            .find(|c| c.spec.op.as_ref() == Some(op))
            .map(|c| c.spec.id.clone())
    }

    /// The operation `op` of scope `scope`.
    pub fn op(&self, scope: &str, op: &str) -> Option<Arc<ops::Op>> {
        self.ops.read().get(scope, op)
    }

    /// Remove every command `source` registered (an extension disabled, a
    /// provider instance stopped); returns their ids. A namespace left
    /// with no commands is free again.
    pub fn unregister_source(&self, source: &str) -> Vec<String> {
        self.commands.write().remove_source(source)
    }

    /// Who registered command `id`: `core`, `oxplow:oxplow/extension?name=<name>`,
    /// `provider:<name>`.
    pub fn source_of(&self, id: &str) -> Option<String> {
        self.commands.read().sources.get(id).cloned()
    }

    /// Who holds `namespace`: `oxplow` for the reserved one, else the
    /// source that registered it; `None` when nothing is under it.
    pub fn namespace_owner(&self, namespace: &str) -> Option<String> {
        self.commands.read().owner(namespace)
    }

    /// The event types the log accepts.
    pub fn vocabulary(&self) -> &oxplow_domain::vocabulary::VocabularyHandle {
        self.log.vocabulary()
    }

    pub fn audit_store(&self) -> &SqliteCommandAuditStore {
        &self.audit
    }

    pub fn proposal_store(&self) -> &SqliteProposalStore {
        &self.proposals
    }

    /// The specs `actor` may invoke and that are offered now, by name.
    pub fn list(&self, actor: &Actor) -> Vec<CommandSpec> {
        let active = self.active();
        self.commands
            .read()
            .commands
            .values()
            .filter(|c| job_refusal(&c.spec, actor).is_none())
            .filter(|c| active.as_ref().is_none_or(|a| a.refusal(&c.spec).is_none()))
            .map(|c| c.spec.clone())
            .collect()
    }

    pub fn spec(&self, name: &str) -> Option<CommandSpec> {
        self.commands.read().get(name).map(|c| c.spec.clone())
    }

    /// A registered command, by name.
    pub(crate) fn command(&self, name: &str) -> Option<Arc<Command>> {
        self.commands.read().get(name).cloned()
    }

    /// A registered command's input schema: what an extension's launcher
    /// command entry is checked against (`extensions::RunningCommands`).
    pub fn input_schema(&self, name: &str) -> Option<Value> {
        self.commands
            .read()
            .get(name)
            .map(|c| c.spec.input_schema.clone())
    }

    /// The log the bus records into (tests read it back).
    pub fn log_for_tests(&self) -> &SqliteEventLogStore {
        &self.log
    }

    /// The `External` commands, sorted — each runs outside the bus's
    /// transaction against a system it doesn't own, so the list is pinned
    /// by a test and grows only on purpose.
    pub fn external_commands(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .commands
            .read()
            .commands
            .values()
            .filter(|c| matches!(c.handler, Handler::External(_)))
            // The window's and the shell's own (`client_host`) run there,
            // not against a system that owns state: reviewed as a class.
            .filter(|c| {
                !c.spec.op.as_ref().is_some_and(|op| {
                    oxplow_domain::scope::scope(&op.scope)
                        .is_some_and(|h| h.host != oxplow_domain::scope::Host::Daemon)
                })
            })
            .map(|c| c.spec.id.clone())
            .collect();
        names.sort();
        names
    }

    /// The composites, sorted — each decides per input whether it runs in
    /// the transaction or as steps, so the list is pinned like
    /// [`Self::external_commands`].
    pub fn composite_commands(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .commands
            .read()
            .commands
            .values()
            .filter(|c| matches!(c.handler, Handler::Compose(_)))
            .map(|c| c.spec.id.clone())
            .collect();
        names.sort();
        names
    }

    /// Run `name` with `input` as `actor`. `confirmed` says a person has
    /// already confirmed this exact call; an agent's `confirmed` is
    /// ignored — an agent can never confirm.
    pub async fn run(
        &self,
        actor: &Actor,
        name: &str,
        input: Value,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        let run = RunContext::core(actor.clone(), Mode::External);
        self.run_inner(&run, name, input, confirmed, RunOrigin::Call)
            .await
    }

    /// [`Self::run`] as part of `run`: its actor, caused by what caused
    /// it (an effort policy's event), recorded as its.
    pub async fn run_in(
        &self,
        run: &RunContext,
        name: &str,
        input: Value,
        confirmed: bool,
    ) -> Result<CommandOutcome, CommandError> {
        // An extension's run (a policy's reaction, a job's tool) may call
        // its own extension's commands; any other one its
        // `permissions.commands` must name. Core's runs have no authority.
        if let oxplow_domain::grant::Decision::Denied(why) =
            run.decide(oxplow_domain::grant::Need::Command(name.to_string()))
        {
            return Err(CommandError::Denied {
                reason: format!("`{name}`: {why}"),
            });
        }
        self.run_inner(run, name, input, confirmed, RunOrigin::Call)
            .await
    }

    /// After a run lands: what it wrote reads back once it returns — its
    /// projections (a work list's records in `work_item`, the page refs)
    /// are delivered first — and the pump's loops are woken for the rest.
    async fn landed(&self) {
        if let Err(error) = self.pump.deliver_projections().await {
            tracing::warn!(%error, "delivering a run's projections failed");
        }
        self.pump.wake();
    }

    /// Run an extension effect's reaction to one event (P8.D10): `command`
    /// (a composite of what its script composed) as `Actor::Effect` for the
    /// event's thread and stream (`anchors`), with the reaction's
    /// `effect_run` row and
    /// `effect.result` landing with the run — in its transaction, after a
    /// `started` claim when a step leaves it, or with its proposal when a
    /// command asks — and its `command.executed` caused by the event.
    /// `resend`: what it composed, when every step is safe to send again,
    /// kept with its claim (tsk954).
    pub(crate) async fn run_effect(
        &self,
        run: &RunContext,
        key: oxplow_db::effect_run_store::EffectRunKey,
        resend: Option<String>,
        command: Command,
        input: Value,
    ) -> Result<CommandOutcome, CommandError> {
        self.run_prepared(
            run,
            Arc::new(command),
            input,
            false,
            RunOrigin::Effect(Arc::new(key), resend.map(Arc::from)),
        )
        .await
    }

    /// [`Self::run`] as `origin`: the undo of an audit row, or a person's
    /// approval of a proposal. The row is marked (undone; approved) in the
    /// same transaction as the run (a `Tx` handler) or claimed before it
    /// (an `External` one, released if the run fails), so two undos of one
    /// row — or two approvals of one proposal — can't both run.
    async fn run_inner(
        &self,
        run: &RunContext,
        name: &str,
        input: Value,
        confirmed: bool,
        origin: RunOrigin,
    ) -> Result<CommandOutcome, CommandError> {
        // The lookup reads the published command map: it holds the
        // generation for that alone. The run pins for itself.
        let command =
            {
                let run = self
                    .activation
                    .pin_run(run)
                    .await
                    .map_err(CommandError::from)?;
                let pin = run
                    .host_generation::<crate::extension_activation::GenerationPin>()
                    .expect("pinned command context")
                    .clone();
                self.activation
                    .read_pinned(&pin, async {
                        self.commands.read().get(name).cloned().ok_or_else(|| {
                            CommandError::Unknown {
                                name: name.to_string(),
                            }
                        })
                    })
                    .await
                    .map_err(CommandError::from)??
            };
        self.run_prepared(run, command, input, confirmed, origin)
            .await
    }

    /// The pipeline (`.context/commands.md` "Running a command") for a
    /// command already looked up — or, for an effect, made for the run.
    async fn run_prepared(
        &self,
        run: &RunContext,
        command: Arc<Command>,
        input: Value,
        confirmed: bool,
        origin: RunOrigin,
    ) -> Result<CommandOutcome, CommandError> {
        // Admission, policy, confirmation and a transaction's run hold the
        // generation. An External handler runs after it is released: it
        // makes its own host calls, each pinning for itself.
        let flow = {
            let run = self
                .activation
                .pin_run(run)
                .await
                .map_err(CommandError::from)?;
            let pin = run
                .host_generation::<crate::extension_activation::GenerationPin>()
                .expect("pinned command context")
                .clone();
            self.activation
                .read_pinned(
                    &pin,
                    self.run_pipeline(&run, command, input, confirmed, origin),
                )
                .await
                .map_err(CommandError::from)??
        };
        match flow {
            Flow::Done(done) => Ok(done),
            Flow::External(external) => self.run_external(*external).await,
        }
    }

    async fn run_pipeline(
        &self,
        run: &RunContext,
        command: Arc<Command>,
        input: Value,
        confirmed: bool,
        origin: RunOrigin,
    ) -> Result<Flow, CommandError> {
        let spec = &command.spec;
        let run = &match self.resolved(run).await {
            Ok(run) => run,
            Err(err) => {
                self.audit_only(run, spec, &input, Outcome::Denied, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        };
        // An extension's command runs as that contribution: a child of the
        // run that called it.
        let run = &match &command.contribution {
            Some(named) => named(run),
            None => run.clone(),
        };
        let actor = run.actor();

        // 0. It must be offered: what it needs is active, and so is the
        // implementation that owns it.
        if let Some(message) = self.active().and_then(|a| a.refusal(spec)) {
            let err = CommandError::Invalid {
                field: None,
                message,
            };
            self.audit_only(run, spec, &input, Outcome::Invalid, Some(err.to_string()))
                .await;
            return Err(err);
        }

        // 1. The input must match the schema.
        let routing = match command.validator.check(&input).map(|()| command.resolve()) {
            Ok(routing) => routing,
            Err(err) => {
                self.audit_only(run, spec, &input, Outcome::Invalid, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        };
        // 2. A job runs only what its extension declared for jobs; anyone
        // else may run every command…
        if let Some(err) = job_refusal(spec, actor) {
            self.audit_only(run, spec, &input, Outcome::Denied, Some(err.to_string()))
                .await;
            return Err(err);
        }
        // 3. …and an agent (or a lens acting for one) must also pass the
        // agent policy: its own stream, an open thread to write.
        // A `Record` command isn't refused here: any thread may change
        // oxplow's own records.
        let mut gates = Gates { may_write: None };
        if let Some(thread_id) = actor.gated_thread() {
            if spec.access.needs_thread() && thread_id.is_none() && !actor.is_anchorless_effect() {
                let err = CommandError::Denied {
                    reason: "a command that changes the workspace requires the actor's thread"
                        .into(),
                };
                self.audit_only(run, spec, &input, Outcome::Denied, Some(err.to_string()))
                    .await;
                return Err(err);
            }
            let may_write = match (spec.access.records(), &thread_id, &self.write_gate) {
                (true, Some(t), Some(gate)) => Some(gate(*t).await),
                _ => None,
            };
            gates.may_write = may_write;
            let gated = may_write.filter(|_| spec.access.needs_thread());
            if let PolicyDecision::Deny { reason, .. } =
                self.policy.check_command(thread_id.as_ref(), spec, gated)
            {
                let err = CommandError::Denied { reason };
                self.audit_only(run, spec, &input, Outcome::Denied, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        }
        // A composite is routed now that the actor is admitted: composed
        // through the SQL gateway, each call checked and routed (`steps.rs`).
        let (resolved, prechecks, private_children) = match routing {
            Routing::Ready(resolved) => {
                let own = command
                    .precheck
                    .clone()
                    .map(|p| steps::Pending::own(p, input.clone()));
                (resolved, own.into_iter().collect(), false)
            }
            Routing::Composite(compose) => {
                match self.route_composite(run, spec, &compose, &input).await {
                    Ok((resolved, mut prechecks, private_children)) => {
                        // Its own check, then each composed call's.
                        if let Some(own) = command.precheck.clone() {
                            prechecks.insert(0, steps::Pending::own(own, input.clone()));
                        }
                        (resolved, prechecks, private_children)
                    }
                    Err(err) => {
                        let recorded = match err {
                            CommandError::Denied { .. } => Outcome::Denied,
                            _ => Outcome::Error,
                        };
                        self.audit_only(run, spec, &input, recorded, Some(err.to_string()))
                            .await;
                        return Err(err);
                    }
                }
            }
        };
        // Recording and proposals must honor private inputs through arbitrary
        // composers. Use an invocation-local spec; handler/schema/authority
        // and the published command registry remain the captured originals.
        let command = if private_children {
            let mut private = command.with_handler(command.handler.clone())?;
            private.spec = steps::private_composite_spec(&command.spec);
            Arc::new(private)
        } else {
            command
        };
        let spec = &command.spec;
        // Each check that needs more than a transaction, before it opens
        // (tsk1010): the command's own, or each composed call's.
        for pending in prechecks {
            if let Err(err) = pending.run().await {
                self.audit_only(run, spec, &input, Outcome::Invalid, Some(err.to_string()))
                    .await;
                return Err(err);
            }
        }
        // 4. A person confirms what asks: nothing is written, they are
        // asked. An agent's run doesn't ask (what it may call is its
        // harness's to decide); an effect's is kept as a proposal.
        // `confirmed` stays a person's word: a handler's own consent (a
        // deploy that adds permissions) still asks for it.
        let confirmed = confirmed && actor.may_confirm();
        let confirm = asks(&command, actor, &input);
        if confirm.required() && !confirmed {
            let preview = Preview {
                command: spec.id.clone(),
                summary: spec.summary.clone(),
                input: input.clone(),
                destructive: matches!(confirm, oxplow_domain::Confirm::Destructive),
                details: Vec::new(),
            };
            let prepared = Prepared {
                command: &command,
                resolved: &resolved,
            };
            return Err(self
                .unconfirmed(run, origin, prepared, input, preview, gates)
                .await);
        }
        // 5a. A read runs without a record: no audit row, no event.
        if spec.access.reads_only() {
            let read = self.run_read(&resolved, run, input).await;
            self.ran_unaudited(run, read.as_ref().err()).await;
            return read.map(Flow::Done);
        }
        // 5'. A composite's steps run outside one transaction: each lands
        // as it runs, the run recorded once at the end (`steps.rs`). A step
        // that asks is step 4's answer, as below.
        if let Resolved::Steps(plan) = &resolved {
            let admitted = steps::Admitted {
                confirmed,
                gates,
                origin: origin.clone(),
            };
            return match self
                .run_steps(run, spec, &input, plan.clone(), admitted)
                .await
                .map(Flow::Done)
            {
                Err(CommandError::NeedsConfirmation { preview }) => {
                    let prepared = Prepared {
                        command: &command,
                        resolved: &resolved,
                    };
                    Err(self
                        .unconfirmed(run, origin, prepared, input, *preview, gates)
                        .await)
                }
                Ok(done) => {
                    self.landed().await;
                    Ok(done)
                }
                other => other,
            };
        }
        // 5. Run, and record the run with its event in one transaction.
        let outcome = match &resolved {
            Resolved::Tx(handler) => {
                let handler = handler.clone();
                let actor_c = actor.clone();
                let run_c = run.clone();
                let spec_c = spec.clone();
                let input_c = input.clone();
                let vocabulary = self.log.vocabulary().clone();
                let origin_tx = origin.clone();
                let ran = self
                    .db
                    .transaction_or(move |tx| {
                        let executed_id = oxplow_domain::EventId::generate();
                        // Fresh per attempt: a retried run counts its calls once.
                        let trace = oxplow_domain::scope::ScopeTrace::default();
                        let ctx = TxCtx {
                            conn: tx,
                            run: &run_c,
                            events: oxplow_db::EventCtx {
                                vocabulary: &vocabulary.current(),
                                source: actor_c.source(),
                                cause: Some(executed_id.clone()),
                            },
                            confirmed,
                            may_write: gates.may_write,
                            depth: 0,
                            trace: &trace,
                        };
                        let out = match handler(&ctx, input_c.clone()) {
                            // A call that changed nothing: its answer, its
                            // transaction rolled back (tsk901).
                            Ok(out) if out.unchanged && matches!(origin_tx, RunOrigin::Call) => {
                                if !out.events.is_empty() || out.inverse.is_some() {
                                    return Err(TxError::Aborted(Abort::Failed(
                                        CommandError::Failed {
                                            message: format!(
                                                "`{}` said it changed nothing but returned \
                                                 events or an inverse",
                                                spec_c.id
                                            ),
                                        },
                                    )));
                                }
                                return Err(TxError::Aborted(Abort::Unchanged(Box::new(out))));
                            }
                            Ok(out) => out,
                            // A lock blip retries the whole run.
                            Err(CommandError::Busy { message }) => {
                                return Err(TxError::Storage(oxplow_domain::DomainError::Busy(
                                    message,
                                )));
                            }
                            // A handler error rolls the transaction back.
                            Err(err) => return Err(TxError::Aborted(Abort::Failed(err))),
                        };
                        let recorded = record_tx(
                            tx,
                            &vocabulary.current(),
                            &spec_c,
                            &input_c,
                            &out,
                            Executed {
                                scopes: trace.summary(),
                                ..Executed::ok(executed_id, &origin_tx, &run_c)
                            },
                        )?;
                        // Fails (and rolls the whole run back) when the row
                        // was undone, the proposal decided, or the effect's
                        // reaction recorded, meanwhile.
                        let marked = match &origin_tx {
                            RunOrigin::Call => Ok(()),
                            RunOrigin::Undo(original) => {
                                mark_undone_tx(tx, *original, recorded.audit_id)
                            }
                            RunOrigin::Approval(id) => {
                                proposal_store::approve_tx(tx, *id, recorded.audit_id).map(|_| ())
                            }
                            RunOrigin::Effect(key, _) => crate::effects::finished_tx(
                                tx,
                                &vocabulary.current(),
                                key,
                                &crate::effects::ran(recorded.audit_id, None),
                                Some(recorded.event_id.clone()),
                            ),
                        };
                        if let Err(e) = marked {
                            // An undo or approval that lost a race: not a
                            // failed run, so it leaves no audit row.
                            return Err(TxError::Aborted(Abort::Lost(lost_race(&origin_tx, e)?)));
                        }
                        if let RunOrigin::Approval(id) = origin_tx {
                            log_approved_tx(
                                tx,
                                &vocabulary.current(),
                                &actor_c,
                                &spec_c,
                                id,
                                &recorded,
                            )?;
                        }
                        Ok((out, recorded))
                    })
                    .await;
                match ran {
                    Ok((out, recorded)) => Ok(finish(out, recorded)),
                    Err(TxError::Aborted(Abort::Unchanged(out))) => {
                        self.ran_unaudited(run, None).await;
                        return Ok(Flow::Done(unrecorded(*out)));
                    }
                    Err(TxError::Aborted(Abort::Lost(e))) => return Err(e),
                    Err(TxError::Aborted(Abort::Failed(e))) => Err(e),
                    Err(TxError::Storage(e)) => Err(CommandError::from(e)),
                }
            }
            Resolved::Steps(_) => unreachable!("composite steps ran above"),
            Resolved::External(_) => {
                // Run after the generation is released (`run_external`).
                return Ok(Flow::External(Box::new(ExternalRun {
                    command,
                    resolved,
                    run: run.clone().without_host_generation(),
                    input,
                    origin,
                    gates,
                    confirmed,
                })));
            }
        };
        self.settle(
            outcome,
            Settling {
                run,
                command: &command,
                resolved: &resolved,
                input,
                origin,
                gates,
            },
        )
        .await
        .map(Flow::Done)
    }

    /// An External handler's run: after the bus released the generation,
    /// then audited and settled like any other.
    async fn run_external(&self, external: ExternalRun) -> Result<CommandOutcome, CommandError> {
        let ExternalRun {
            command,
            resolved,
            run,
            input,
            origin,
            gates,
            confirmed,
        } = external;
        let spec = &command.spec;
        let Resolved::External(handler) = &resolved else {
            unreachable!("an external run holds an external handler")
        };
        let outcome = async {
            self.claim(&origin).await?;
            // The command's own run: what it starts (a collector run,
            // a provider's `host/call`s) is part of the run audited.
            let invocation = Invocation {
                run: run.clone(),
                confirmed,
                ..origin.invocation(&run, 0, &spec.id, &input)
            };
            let called = invocation.run.clone();
            match handler(invocation, input.clone()).await {
                Ok(out) => Ok(self
                    .record_external(
                        &run,
                        spec,
                        &input,
                        out,
                        origin.clone(),
                        called.trace().summary(),
                    )
                    .await),
                Err(err) => {
                    self.release(&origin).await;
                    Err(err)
                }
            }
        }
        .await;
        self.settle(
            outcome,
            Settling {
                run: &run,
                command: &command,
                resolved: &resolved,
                input,
                origin,
                gates,
            },
        )
        .await
    }

    /// How a run ended, as the caller sees it: landed, asked, or recorded
    /// as refused or failed.
    async fn settle(
        &self,
        outcome: Result<CommandOutcome, CommandError>,
        settling: Settling<'_>,
    ) -> Result<CommandOutcome, CommandError> {
        let Settling {
            run,
            command,
            resolved,
            input,
            origin,
            gates,
        } = settling;
        let spec = &command.spec;
        match outcome {
            Ok(done) => {
                self.landed().await;
                Ok(done)
            }
            // A confirmation a handler raised while running (a composite
            // whose child asks) is step 4's answer, late: rolled back,
            // nothing audited — a person is asked, an agent's run proposed.
            Err(CommandError::NeedsConfirmation { preview }) => {
                let prepared = Prepared { command, resolved };
                Err(self
                    .unconfirmed(run, origin, prepared, input, *preview, gates)
                    .await)
            }
            Err(err) => {
                // A handler's refusal (a claim the actor may not take) is a
                // denial, not an error.
                let recorded = match err {
                    CommandError::Denied { .. } => Outcome::Denied,
                    _ => Outcome::Error,
                };
                self.audit_only(run, spec, &input, recorded, Some(err.to_string()))
                    .await;
                Err(err)
            }
        }
    }

    /// A run that leaves no audit row — a read, or one that changed
    /// nothing — is still an extension's run when it is one: recorded as
    /// such ([`oxplow_db::extension_run_store`]).
    async fn ran_unaudited(&self, run: &RunContext, failed: Option<&CommandError>) {
        if run.contribution_ref().is_none() {
            return;
        }
        let (ended, error) = match failed {
            None => (oxplow_domain::run::Outcome::Ok, None),
            Some(e) => (oxplow_domain::run::Outcome::Failed, Some(e.to_string())),
        };
        let run = run.clone();
        if let Err(e) = self
            .db
            .transaction(move |tx| {
                oxplow_db::extension_run_store::record_tx(tx, &run, ended, error.as_deref())
            })
            .await
        {
            tracing::warn!(error = %e, "recording an unaudited run failed");
        }
    }

    /// The actor a run is made as. An agent's — or a lens's acting for one —
    /// whose transport carried its thread but not its stream gets its
    /// thread's stream, so every handler's "the caller's stream" is
    /// `actor.stream_id()`, one answer; an agent claiming a thread that
    /// doesn't exist is refused.
    async fn resolved(&self, run: &RunContext) -> Result<RunContext, CommandError> {
        let actor = run.actor();
        let (Some(Some(thread)), None) = (actor.agent_thread(), actor.stream_id()) else {
            return Ok(run.clone());
        };
        use rusqlite::OptionalExtension as _;
        let stream: Option<i64> = self
            .db
            .read(move |c| {
                c.query_row(
                    "SELECT stream_id FROM threads WHERE id = ?1",
                    [thread.value()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(oxplow_db::map_sql_err)
            })
            .await?;
        match stream {
            Some(s) => Ok(run.clone().placed_in(oxplow_domain::StreamId::new(s))),
            None => Err(CommandError::Denied {
                reason: format!("the agent's thread `{thread}` doesn't exist"),
            }),
        }
    }

    /// Step 5 for a command that only reads (`View` / `Read`): the handler
    /// on a plain connection, nothing recorded. It must not write (it isn't
    /// audited).
    async fn run_read(
        &self,
        resolved: &Resolved,
        run: &RunContext,
        input: Value,
    ) -> Result<CommandOutcome, CommandError> {
        let out = match resolved {
            Resolved::Tx(handler) => {
                let handler = handler.clone();
                let run = run.clone();
                let vocabulary = self.log.vocabulary().clone();
                // A read snapshot, always rolled back: a reading handler's
                // stray write can't land (it isn't audited).
                self.db
                    .read_or(move |tx| {
                        let ctx = TxCtx {
                            conn: tx,
                            run: &run,
                            events: oxplow_db::EventCtx {
                                vocabulary: &vocabulary.current(),
                                source: run.actor().source(),
                                cause: None,
                            },
                            confirmed: false,
                            may_write: None,
                            depth: 0,
                            trace: &oxplow_domain::scope::ScopeTrace::default(),
                        };
                        handler(&ctx, input.clone()).map_err(TxError::Aborted)
                    })
                    .await
                    .map_err(command_error)?
            }
            Resolved::External(handler) => {
                let invocation = Invocation {
                    run: run.child(None, Mode::External),
                    idempotency_key: None,
                    confirmed: false,
                };
                handler(invocation, input).await?
            }
            Resolved::Steps(_) => {
                return Err(CommandError::Invalid {
                    field: None,
                    message: "a command that only reads can't compose steps that write".into(),
                });
            }
        };
        Ok(CommandOutcome {
            result: out.result,
            audit_id: None,
            event_id: None,
            inverse: None,
        })
    }
}

/// Step 0 for a composed call (at `field`): what it needs is active (and the
/// implementation that owns it is), as a direct call's must be — else
/// it's refused at its place in the calls.
pub(super) fn offered(
    active: Option<&crate::capabilities::Active>,
    spec: &CommandSpec,
    field: &str,
) -> Result<(), CommandError> {
    match active.and_then(|a| a.refusal(spec)) {
        Some(message) => Err(CommandError::Invalid {
            field: Some(field.to_string()),
            message: format!("`{}`: {message}", spec.id),
        }),
        None => Ok(()),
    }
}

/// The top-level input fields `schema` marks secret (`writeOnly: true`): a
/// key, a token, a credential. Written, never read back: kept out of every
/// record (`CommandSpec::unrecorded`), masked in every form, never in a
/// draft. See `.context/commands.md` "Secrets".
pub fn secret_fields(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, p)| p.get("writeOnly").and_then(Value::as_bool) == Some(true))
        .map(|(k, _)| k.clone())
        .collect()
}

/// Why `actor` may not run `spec`, if it may not: it is a durable job's
/// model (or a lens it pressed) and `spec` isn't declared for jobs
/// (`job: true`). A person, an agent and a lens acting for either may run
/// every command.
pub(crate) fn job_refusal(spec: &CommandSpec, actor: &Actor) -> Option<CommandError> {
    (actor.invoker() == oxplow_domain::Invoker::Job && !spec.job).then(|| CommandError::Denied {
        reason: format!(
            "`{}` isn't a job's tool: its command says `job: true`",
            spec.id
        ),
    })
}

/// What a run of `command` by `actor` asks: the command's confirmation,
/// except an agent's (or a lens it pressed), which never asks.
pub(crate) fn asks(command: &Command, actor: &Actor, input: &Value) -> oxplow_domain::Confirm {
    if actor.is_agent_driven() {
        oxplow_domain::Confirm::Never
    } else {
        command.confirm(input)
    }
}

#[cfg(test)]
mod tests;
