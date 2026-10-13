//! One run of extension code, and what every host hands it
//! (`.context/extensions.md` "The run context"): who it runs as, which
//! contribution at which version and catalog generation, what caused it
//! and where it sits (anchors, subject), whether it may call out, its
//! budget and the flag that cancels it. A host builds one at the start of
//! a run with a named constructor; nested work gets a [`RunContext::child`],
//! which can narrow the budget but never change who runs or what was
//! granted. What the run doesn't have is `None`, never made up.

use std::any::Any;
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

/// What a run used, as it went: the scopes it called, what its scripts
/// cost, the model calls it made. Nested work of the same contribution
/// adds to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub scopes: std::collections::BTreeMap<String, u32>,
    /// Its scripts' CPU time, summed.
    pub cpu_ms: u64,
    /// The most any one of its scripts held live at once.
    pub peak_bytes: u64,
    pub ai_calls: u32,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Skipped,
    Failed,
    Denied,
    Cancelled,
    Stale,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Ok => "ok",
            Outcome::Skipped => "skipped",
            Outcome::Failed => "failed",
            Outcome::Denied => "denied",
            Outcome::Cancelled => "cancelled",
            Outcome::Stale => "stale",
        }
    }
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Pure => "pure",
            Mode::Transactional => "transactional",
            Mode::External => "external",
        }
    }
}

/// What a run may do: what its contribution declares, the grant it runs
/// on when a person's approval is what lets it run, and where this
/// machine can hold it to that (`crate::grant`).
#[derive(Debug, Clone)]
pub struct Authority {
    pub declared: crate::grant::GrantRequest,
    pub grant: Option<crate::grant::Grant>,
    pub enforcement: crate::grant::Enforcement,
}

/// A host-owned live check, inherited by nested work without exposing it
/// to extension code. It rechecks consent/version at sensitive calls.
#[derive(Clone)]
pub struct AuthorityGuard(Arc<dyn Fn() -> Result<(), String> + Send + Sync>);

impl std::fmt::Debug for AuthorityGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorityGuard")
    }
}

/// Opaque host-owned lifetime pin. Possessing this value grants no authority;
/// the host must validate its concrete type and owning boundary before use.
#[derive(Clone)]
struct HostGenerationPin(Arc<dyn Any + Send + Sync>);

impl std::fmt::Debug for HostGenerationPin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostGenerationPin")
    }
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
    /// When it began, as a timestamp (the record's `started_at`).
    began: crate::time::Timestamp,
    cancel: Cancel,
    trace: Arc<ScopeTrace>,
    usage: Arc<std::sync::Mutex<Usage>>,
    authority: Option<Arc<Authority>>,
    authority_guard: Option<AuthorityGuard>,
    host_generation: Option<HostGenerationPin>,
    decided: Arc<std::sync::Mutex<Vec<crate::grant::Decided>>>,
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
            began: crate::time::Timestamp::now(),
            cancel: Cancel::default(),
            trace: Arc::new(ScopeTrace::default()),
            usage: Arc::default(),
            authority: None,
            authority_guard: None,
            host_generation: None,
            decided: Arc::default(),
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

    /// Fresh host job execution. Persisted scheduling context supplies
    /// attribution only: actor, cancellation, authority and budgets are new.
    /// No service is authorized until the dispatcher attaches live authority.
    pub fn job(
        contribution: ContributionRef,
        identity: &crate::extension_job_context::JobExecutionIdentity,
        provenance: &crate::extension_job_context::JobInvocationContext,
    ) -> Result<Self, crate::extension_job_context::JobContextError> {
        provenance.validate()?;
        let actor = Actor::job(
            identity,
            provenance.anchors().thread_id,
            provenance.anchors().stream_id,
        );
        let mut run = Self::contribution(contribution, actor, Mode::External)
            .anchored(provenance.anchors().clone())
            .about(provenance.subject().to_vec())
            .authorized(Authority {
                declared: crate::grant::GrantRequest::default(),
                grant: None,
                enforcement: crate::grant::Enforcement { hosts: false },
            });
        run.parent = Some(provenance.originating_run_id().clone());
        run.cause = provenance.cause().cloned();
        Ok(run)
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

    /// May do what `authority` says (its declaration, and its grant when
    /// it runs on one).
    pub fn authorized(mut self, authority: Authority) -> Self {
        self.authority = Some(Arc::new(authority));
        self
    }

    pub fn guarded(mut self, check: Arc<dyn Fn() -> Result<(), String> + Send + Sync>) -> Self {
        self.authority_guard = Some(match self.authority_guard.take() {
            None => AuthorityGuard(check),
            Some(previous) => AuthorityGuard(Arc::new(move || {
                (previous.0)()?;
                check()
            })),
        });
        self
    }

    /// Attach an opaque host-owned generation pin to this live run.
    ///
    /// The pin may keep a host reader alive while this context, a child, or
    /// spawned work retains it. It grants no authority; the host must
    /// downcast and validate the pin at its owning boundary. Fresh contexts,
    /// including jobs, do not inherit pins.
    pub fn with_host_generation<T: Any + Send + Sync>(mut self, pin: Arc<T>) -> Self {
        self.host_generation = Some(HostGenerationPin(pin));
        self
    }

    /// Read a host-owned generation pin of `T`. The caller must validate
    /// that the pin belongs to its own host boundary; this is not authority.
    pub fn host_generation<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.host_generation
            .as_ref()?
            .0
            .as_ref()
            .downcast_ref::<T>()
    }

    /// A bounded SQL grant needs a policy at preparation, before any rows
    /// are evaluated. Bare `sql.read` explicitly grants the whole layer.
    pub fn has_model_bounds(&self) -> bool {
        self.authority.as_ref().is_some_and(|a| {
            !a.grant
                .as_ref()
                .map_or(&a.declared, |g| &g.request)
                .scopes
                .contains("sql.read")
        })
    }

    pub fn has_path_bounds(&self) -> bool {
        self.authority.as_ref().is_some_and(|a| {
            !a.grant
                .as_ref()
                .map_or(&a.declared, |g| &g.request)
                .scopes
                .contains("tree.read")
        })
    }

    /// Filtering a captured tree is not an attempted read of every
    /// excluded file. Record service use, without a denial per hidden path.
    pub fn permits_path(&self, path: &str) -> bool {
        crate::service::valid_path(path)
            && self.authority.as_ref().is_none_or(|a| {
                matches!(
                    crate::grant::decide(
                        &crate::grant::Need::Path(path.into()),
                        a.grant.as_ref(),
                        &a.declared,
                        a.enforcement
                    ),
                    crate::grant::Decision::Allowed
                )
            })
    }

    pub fn check_authority(&self) -> Result<(), String> {
        if self.cancel.is_cancelled() {
            return Err("the run was cancelled".into());
        }
        if let Some(guard) = &self.authority_guard {
            if let Err(reason) = (guard.0)() {
                self.cancel.cancel();
                return Err(reason);
            }
        }
        Ok(())
    }

    /// Within `budget`.
    pub fn within(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    /// This run, started by `parent`'s (a person's `oxplow.effect.retry`
    /// of an effect's reaction): linked to it as its parent and stopped
    /// with it, still run as its own actor.
    pub fn under(mut self, parent: &RunContext) -> Self {
        self.parent = Some(parent.run.clone());
        self.cancel = parent.cancel.clone();
        if let Some(guard) = &parent.authority_guard {
            self = self.guarded(guard.0.clone());
        }
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
    /// and a budget no larger than what this run has left. Work of the
    /// same contribution adds to this run's usage; another's keeps its own.
    pub fn child(&self, contribution: Option<ContributionRef>, mode: Mode) -> Self {
        let left = self.budget.ceiling.saturating_sub(self.started.elapsed());
        let (usage, authority, decided) = match &contribution {
            None => (
                self.usage.clone(),
                self.authority.clone(),
                self.decided.clone(),
            ),
            Some(_) => (Arc::default(), None, Arc::default()),
        };
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
            began: crate::time::Timestamp::now(),
            cancel: self.cancel.clone(),
            trace: Arc::new(ScopeTrace::default()),
            usage,
            authority,
            authority_guard: self.authority_guard.clone(),
            host_generation: self.host_generation.clone(),
            decided,
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

    /// Whether a host attached an explicit declaration for this scope.
    /// Core's unrestricted context is not an extension service declaration.
    pub fn declares_scope(&self, scope: &str) -> bool {
        self.authority.as_ref().is_some_and(|authority| {
            authority
                .declared
                .scopes
                .iter()
                .any(|need| crate::service::need_scope(need) == scope)
        })
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

    /// When it began.
    pub fn began(&self) -> crate::time::Timestamp {
        self.began
    }

    /// It called scope `id`.
    pub fn used_scope(&self, id: &str) {
        *self.usage_lock().scopes.entry(id.to_string()).or_insert(0) += 1;
    }

    /// One of its scripts ran, costing `cpu_ms` and holding `peak_bytes`.
    pub fn used_cpu(&self, cpu_ms: u64, peak_bytes: u64) {
        let mut u = self.usage_lock();
        u.cpu_ms += cpu_ms;
        u.peak_bytes = u.peak_bytes.max(peak_bytes);
    }

    /// It called a model.
    pub fn used_ai(&self) {
        self.usage_lock().ai_calls += 1;
    }

    /// Decide `need` against its authority and record it. A run with no
    /// authority (core's own work) is allowed what it does.
    pub fn decide(&self, need: crate::grant::Need) -> crate::grant::Decision {
        let decision = match self.check_authority() {
            Err(reason) => crate::grant::Decision::Denied(reason),
            Ok(()) => match &self.authority {
                Some(a) => {
                    crate::grant::decide(&need, a.grant.as_ref(), &a.declared, a.enforcement)
                }
                None => crate::grant::Decision::Allowed,
            },
        };
        if self.authority.is_some() {
            self.decided
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(crate::grant::Decided {
                    need,
                    decision: decision.clone(),
                });
        }
        decision
    }

    /// Decide its use of service `scope` (`crate::service`): refused when
    /// this run's mode may not use it (a pure run asking a model), else
    /// decided by its authority — recorded either way.
    pub fn decide_service(&self, scope: &str) -> crate::grant::Decision {
        let need = crate::grant::Need::Scope(scope.to_string());
        if let Some(service) = crate::service::service(scope) {
            if !service.modes.contains(&self.mode) {
                let decision = crate::grant::Decision::Denied(format!(
                    "a {} run can't use `{scope}` ({}): only a run that may call out can",
                    self.mode.as_str(),
                    service.title.to_lowercase()
                ));
                self.decided.lock().unwrap_or_else(|e| e.into_inner()).push(
                    crate::grant::Decided {
                        need,
                        decision: decision.clone(),
                    },
                );
                return decision;
            }
        }
        self.decide(need)
    }

    /// What its authority decided, in order.
    pub fn decisions(&self) -> Vec<crate::grant::Decided> {
        self.decided
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Whether anything it did was denied.
    pub fn was_denied(&self) -> bool {
        self.decisions()
            .iter()
            .any(|d| matches!(d.decision, crate::grant::Decision::Denied(_)))
    }

    /// What it has used so far.
    pub fn usage(&self) -> Usage {
        self.usage_lock().clone()
    }

    fn usage_lock(&self) -> std::sync::MutexGuard<'_, Usage> {
        self.usage.lock().unwrap_or_else(|e| e.into_inner())
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
    /// its ref (`<kind>:<extension>/<id>`, `command:<bus id>`).
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

    /// Same-contribution nested work adds to the run's usage; another
    /// contribution's keeps its own.
    #[test]
    fn usage_follows_the_contribution() {
        let run = RunContext::contribution(effect("g1"), Actor::System, Mode::External);
        run.child(None, Mode::Pure).used_scope("sql.read");
        run.used_cpu(5, 100);
        run.child(None, Mode::Pure).used_cpu(3, 50);
        let other = run.child(Some(effect("g1")), Mode::External);
        other.used_ai();
        let u = run.usage();
        assert_eq!(u.scopes.get("sql.read"), Some(&1));
        assert_eq!((u.cpu_ms, u.peak_bytes, u.ai_calls), (8, 100, 0));
        assert_eq!(other.usage().ai_calls, 1);
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

    /// A pure run may not ask a model, whatever it declares; it may read.
    #[test]
    fn a_pure_run_may_not_use_a_service_that_calls_out() {
        let declared = crate::grant::GrantRequest {
            scopes: ["ai.compute".to_string(), "tree.read".to_string()].into(),
            ..Default::default()
        };
        let authority = |mode| {
            RunContext::contribution(effect("g1"), Actor::System, mode).authorized(Authority {
                declared: declared.clone(),
                grant: None,
                enforcement: crate::grant::Enforcement { hosts: true },
            })
        };
        let pure = authority(Mode::Pure);
        assert!(matches!(
            pure.decide_service("ai.compute"),
            crate::grant::Decision::Denied(why) if why.contains("a pure run can't use `ai.compute`")
        ));
        assert_eq!(
            pure.decide_service("tree.read"),
            crate::grant::Decision::Allowed
        );
        assert_eq!(pure.decisions().len(), 2, "both recorded");
        assert_eq!(
            authority(Mode::External).decide_service("ai.compute"),
            crate::grant::Decision::Allowed
        );
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
