//! One run of extension code, and what every host hands it
//! (`.context/extensions.md` "The run context"): who it runs as, which
//! contribution at which version and catalog generation, what caused it
//! and where it sits (anchors, subject), whether it may call out, its
//! budget and the flag that cancels it. A host builds one at the start of
//! a run with a named constructor; nested work gets a [`RunContext::child`],
//! which can narrow the budget but never change who runs or what was
//! granted. What the run doesn't have is `None`, never made up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::commands::Actor;
use crate::contribution::ContributionId;
use crate::events::{Anchors, EventId};
use crate::scope::ScopeTrace;
use crate::DomainError;

/// A run's identity: a UUIDv7, so ids sort by when the run began.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub String);

impl RunId {
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Asks a run to stop: a script's worker is killed at its next tick, a
/// provider call is cancelled, and nested work sharing the flag stops too.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A run's time limits. Past either, it is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// The script's own time: what it spends outside waiting on its host
    /// (a command's `scope(…)`, a collector's `ai_*`).
    pub timeout: Duration,
    /// Wall-clock time, waiting included: what stops a per-row loop of
    /// model calls from running for hours.
    pub ceiling: Duration,
}

/// The default [`Budget::ceiling`]. A run past it keeps the model answers
/// it recorded (`ai_result`), so its next run gets further.
const CEILING: Duration = Duration::from_secs(600);

impl Default for Budget {
    fn default() -> Self {
        Budget {
            // A runaway catch, not a latency target: a real whole-workspace
            // report runs several seconds in the debug build the app ships
            // to developers, and nothing waits on a collector's run.
            timeout: Duration::from_secs(120),
            ceiling: CEILING,
        }
    }
}

impl Budget {
    /// A budget with a custom timeout (and a ceiling no lower than it).
    pub const fn with_timeout(timeout: Duration) -> Self {
        let ceiling = if timeout.as_nanos() > CEILING.as_nanos() {
            timeout
        } else {
            CEILING
        };
        Budget { timeout, ceiling }
    }

    /// This budget with a custom ceiling.
    pub fn with_ceiling(self, ceiling: Duration) -> Self {
        Budget { ceiling, ..self }
    }

    /// The tighter of the two, field by field.
    pub fn min(self, other: Budget) -> Budget {
        Budget {
            timeout: self.timeout.min(other.timeout),
            ceiling: self.ceiling.min(other.ceiling),
        }
    }
}

/// How a run may act.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Deterministic, no I/O but its scope reads: a command's composer, an
    /// effect's or policy's script, an upcast. It may be run again.
    Pure,
    /// Inside the run's database transaction: a command's `Tx` handler.
    Transactional,
    /// May call out — a program, a provider, a model — outside any
    /// transaction.
    External,
}

/// Which contribution a run belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionRef {
    pub id: ContributionId,
    /// The extension's declared `version`, when it has one.
    pub version: Option<String>,
    /// The extension catalog's generation the run was built against.
    pub generation: Option<String>,
}

/// Why a run can't go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// Built against a catalog generation that's no longer served: the
    /// extension changed under it.
    Stale {
        run: RunId,
        generation: String,
    },
    Cancelled,
    /// A pure run asked to call out.
    NotExternal {
        what: String,
    },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Stale { run, generation } => write!(
                f,
                "run {run} was started against extension generation {generation}, which is no \
                 longer loaded; it stops here"
            ),
            RunError::Cancelled => f.write_str("the run was cancelled"),
            RunError::NotExternal { what } => {
                write!(f, "{what} needs to call out, which a pure run may not")
            }
        }
    }
}

impl From<RunError> for DomainError {
    fn from(e: RunError) -> Self {
        DomainError::Invalid(e.to_string())
    }
}

/// One run of extension (or core) code. Built by a named constructor;
/// cheap to clone (clones share the cancel flag and the trace).
#[derive(Debug, Clone)]
pub struct RunContext {
    run: RunId,
    parent: Option<RunId>,
    contribution: Option<ContributionRef>,
    actor: Actor,
    anchors: Anchors,
    subject: Vec<String>,
    cause: Option<EventId>,
    mode: Mode,
    budget: Budget,
    started: Instant,
    cancel: Cancel,
    trace: Arc<ScopeTrace>,
}

impl RunContext {
    fn new(actor: Actor, contribution: Option<ContributionRef>, mode: Mode) -> Self {
        RunContext {
            run: RunId::generate(),
            parent: None,
            contribution,
            anchors: actor.anchors(),
            actor,
            subject: Vec::new(),
            cause: None,
            mode,
            budget: Budget::default(),
            started: Instant::now(),
            cancel: Cancel::default(),
            trace: Arc::new(ScopeTrace::default()),
        }
    }

    /// Core's own work, for `actor`: no extension behind it.
    pub fn core(actor: Actor, mode: Mode) -> Self {
        Self::new(actor, None, mode)
    }

    /// A contribution's run, for `actor`.
    pub fn contribution(contribution: ContributionRef, actor: Actor, mode: Mode) -> Self {
        Self::new(actor, Some(contribution), mode)
    }

    /// Caused by `event` (its trigger).
    pub fn caused_by(mut self, event: EventId) -> Self {
        self.cause = Some(event);
        self
    }

    /// Where it sits: these anchors, the actor's filling what they lack.
    pub fn anchored(mut self, anchors: Anchors) -> Self {
        let own = self.actor.anchors();
        self.anchors = Anchors {
            stream_id: anchors.stream_id.or(own.stream_id),
            thread_id: anchors.thread_id.or(own.thread_id),
            agent_session_id: anchors.agent_session_id.or(own.agent_session_id),
            ..anchors
        };
        self
    }

    /// What it's about.
    pub fn about(mut self, subject: Vec<String>) -> Self {
        self.subject = subject;
        self
    }

    /// Within `budget`.
    pub fn within(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// This run with its agent placed in `stream` (its transport carried
    /// the thread but not the stream): the same actor, its stream filled.
    pub fn placed_in(mut self, stream: crate::ids::StreamId) -> Self {
        self.actor = self.actor.in_stream(stream);
        self.anchors.stream_id = self.anchors.stream_id.or(Some(stream));
        self
    }

    /// Nested work of this run: a new run id whose parent is this one,
    /// the same actor, cancel flag and generation, `contribution` when it
    /// is another one's (an effect's reaction a person's retry started),
    /// and a budget no larger than what this run has left.
    pub fn child(&self, contribution: Option<ContributionRef>, mode: Mode) -> Self {
        let left = self.budget.ceiling.saturating_sub(self.started.elapsed());
        RunContext {
            run: RunId::generate(),
            parent: Some(self.run.clone()),
            contribution: contribution.or_else(|| self.contribution.clone()),
            actor: self.actor.clone(),
            anchors: self.anchors.clone(),
            subject: self.subject.clone(),
            cause: self.cause.clone(),
            mode,
            budget: self.budget.min(Budget {
                timeout: self.budget.timeout,
                ceiling: left,
            }),
            started: Instant::now(),
            cancel: self.cancel.clone(),
            trace: Arc::new(ScopeTrace::default()),
        }
    }

    pub fn run(&self) -> &RunId {
        &self.run
    }

    pub fn parent(&self) -> Option<&RunId> {
        self.parent.as_ref()
    }

    pub fn contribution_ref(&self) -> Option<&ContributionRef> {
        self.contribution.as_ref()
    }

    /// The extension it belongs to.
    pub fn extension(&self) -> Option<&str> {
        self.contribution.as_ref().map(|c| c.id.extension.as_str())
    }

    pub fn actor(&self) -> &Actor {
        &self.actor
    }

    pub fn anchors(&self) -> &Anchors {
        &self.anchors
    }

    pub fn subject(&self) -> &[String] {
        &self.subject
    }

    pub fn cause(&self) -> Option<&EventId> {
        self.cause.as_ref()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn budget(&self) -> Budget {
        self.budget
    }

    pub fn cancel(&self) -> &Cancel {
        &self.cancel
    }

    /// The scopes it called, counted (the audit row's `scopes`).
    pub fn trace(&self) -> &ScopeTrace {
        &self.trace
    }

    /// How long it has run.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Refuse to go on if cancelled, or if it was built against a catalog
    /// generation other than `current` (the one now served).
    pub fn check(&self, current: Option<&str>) -> Result<(), RunError> {
        if self.cancel.is_cancelled() {
            return Err(RunError::Cancelled);
        }
        match (
            self.contribution
                .as_ref()
                .and_then(|c| c.generation.as_deref()),
            current,
        ) {
            (Some(ran), Some(now)) if ran != now => Err(RunError::Stale {
                run: self.run.clone(),
                generation: ran.to_string(),
            }),
            _ => Ok(()),
        }
    }

    /// Refuse `what` — a program, a provider, a model — unless this run
    /// may call out.
    pub fn may_call_out(&self, what: &str) -> Result<(), RunError> {
        match self.mode {
            Mode::External => Ok(()),
            Mode::Pure | Mode::Transactional => Err(RunError::NotExternal { what: what.into() }),
        }
    }

    /// What a record or a provider is told about it: everything but the
    /// handles, `null` where absent.
    pub fn describe(&self) -> RunDescription {
        RunDescription {
            run: self.run.clone(),
            parent: self.parent.clone(),
            contribution: self.contribution.as_ref().map(|c| c.id.to_string()),
            version: self.contribution.as_ref().and_then(|c| c.version.clone()),
            generation: self
                .contribution
                .as_ref()
                .and_then(|c| c.generation.clone()),
            actor: self.actor.clone(),
            anchors: self.anchors.clone(),
            subject: self.subject.clone(),
            cause: self.cause.clone(),
            mode: self.mode,
        }
    }
}

/// A run as data: what a usage record keeps and a provider is sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunDescription {
    pub run: RunId,
    pub parent: Option<RunId>,
    /// `<extension>/<kind>/<local>`.
    pub contribution: Option<String>,
    pub version: Option<String>,
    pub generation: Option<String>,
    pub actor: Actor,
    pub anchors: Anchors,
    pub subject: Vec<String>,
    pub cause: Option<EventId>,
    pub mode: Mode,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution::ContributionKind;
    use crate::ids::ThreadId;

    fn effect(generation: &str) -> ContributionRef {
        ContributionRef {
            id: ContributionId::new("acme", ContributionKind::Effect, "note"),
            version: Some("1.2.0".into()),
            generation: Some(generation.into()),
        }
    }

    /// A child is the same run's work: same actor, same cancel flag, a
    /// parent link, and no more budget than the parent has left.
    #[test]
    fn a_child_shares_cancel_and_cant_widen() {
        let parent = RunContext::contribution(effect("g1"), Actor::Human, Mode::External).within(
            Budget::with_timeout(Duration::from_secs(5)).with_ceiling(Duration::from_secs(5)),
        );
        let child = parent.child(None, Mode::Pure);
        assert_eq!(child.parent(), Some(parent.run()));
        assert_ne!(child.run(), parent.run());
        assert_eq!(child.actor(), &Actor::Human);
        assert!(child.budget().ceiling <= Duration::from_secs(5));
        assert_eq!(child.extension(), Some("acme"));
        parent.cancel().cancel();
        assert_eq!(child.check(None), Err(RunError::Cancelled));
    }

    /// A run built against another generation than the one served stops.
    #[test]
    fn a_run_from_an_older_generation_is_stale() {
        let ctx = RunContext::contribution(effect("g1"), Actor::System, Mode::Pure);
        assert_eq!(ctx.check(Some("g1")), Ok(()));
        assert!(matches!(ctx.check(Some("g2")), Err(RunError::Stale { .. })));
        assert_eq!(
            RunContext::core(Actor::System, Mode::Pure).check(Some("g2")),
            Ok(())
        );
    }

    /// Only an external run may call out.
    #[test]
    fn a_pure_run_may_not_call_out() {
        let pure = RunContext::core(Actor::System, Mode::Pure);
        assert!(pure.may_call_out("a model").is_err());
        assert!(RunContext::core(Actor::System, Mode::External)
            .may_call_out("a model")
            .is_ok());
    }

    /// The description has every field, `null` where the run has none, and
    /// anchors filled from the actor where the event gave none.
    #[test]
    fn a_description_fills_nothing_it_wasnt_given() {
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(ThreadId::new(3)),
            stream_id: None,
        };
        let ctx = RunContext::core(agent, Mode::External).anchored(Anchors {
            turn_id: Some(7),
            ..Anchors::default()
        });
        let json = serde_json::to_value(ctx.describe()).unwrap();
        for absent in ["parent", "contribution", "version", "generation", "cause"] {
            assert_eq!(json[absent], serde_json::Value::Null, "{absent}");
        }
        assert_eq!(json["anchors"]["thread_id"], "thr3");
        assert_eq!(json["anchors"]["turn_id"], 7);
        assert_eq!(json["mode"], "external");
    }
}
