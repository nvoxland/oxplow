//! The app's constructors of a run's context (`oxplow_domain::run`,
//! `.context/extensions.md` "The run context"): each host that runs
//! extension code builds its run here, so every run names its
//! contribution, the extension's version and the catalog generation the
//! same way, and is caused by and anchored to what triggered it.

use std::path::Path;

use oxplow_domain::contribution::{ContributionId, ContributionKind};
use oxplow_domain::run::{ContributionRef, Mode, RunContext};
use oxplow_domain::{Actor, StoredEvent};

use crate::extension_catalog::ExtensionCatalog;
use crate::extensions::Extension;

/// `ext`'s contribution `kind`/`local`, at its version and the catalog
/// generation `root` is served at now.
pub fn contribution(
    catalog: &ExtensionCatalog,
    root: &Path,
    ext: &Extension,
    kind: ContributionKind,
    local: &str,
) -> ContributionRef {
    ContributionRef {
        id: ContributionId::new(&ext.name, kind, local),
        version: ext.version.clone(),
        generation: Some(catalog.runtime_snapshot(root).generation().to_string()),
    }
}

/// The catalog generation `root` is served at now: what a run checks it
/// was built against.
pub fn current_generation(catalog: &ExtensionCatalog, root: &Path) -> String {
    catalog.runtime_snapshot(root).generation().to_string()
}

/// A run reacting to `event`: caused by it, anchored where it happened,
/// about what it's about.
fn reacting(run: RunContext, event: &StoredEvent) -> RunContext {
    let env = &event.envelope;
    run.caused_by(env.id.clone())
        .anchored(env.anchors.clone())
        .about(env.subject.clone())
}

/// An effect's reaction to `event`: as the effect, for the event's thread
/// and stream (`Actor::Effect`).
pub fn for_effect(
    catalog: &ExtensionCatalog,
    root: &Path,
    ext: &Extension,
    decl: &crate::effects::EffectDecl,
    event: &StoredEvent,
) -> RunContext {
    let anchors = &event.envelope.anchors;
    let actor = Actor::Effect {
        effect: decl.name(),
        thread_id: anchors.thread_id,
        stream_id: anchors.stream_id,
    };
    reacting(
        RunContext::contribution(
            contribution(catalog, root, ext, ContributionKind::Effect, &decl.id),
            actor,
            Mode::External,
        ),
        event,
    )
}

/// The effort policy `id` reacting to `event`: as the policy's effect
/// actor, for the event's thread and stream. `ext` is the extension that
/// implements it, when one does (core's built-in has none).
pub fn for_policy(
    catalog: &ExtensionCatalog,
    root: &Path,
    id: &str,
    ext: Option<&Extension>,
    event: &StoredEvent,
) -> RunContext {
    let anchors = &event.envelope.anchors;
    let actor = Actor::Effect {
        effect: format!("effort_policy:{id}"),
        thread_id: anchors.thread_id,
        stream_id: anchors.stream_id,
    };
    let run = match ext {
        Some(ext) => RunContext::contribution(
            contribution(
                catalog,
                root,
                ext,
                ContributionKind::Implementation,
                &oxplow_domain::contribution::implementation_local("effort_policy", id),
            ),
            actor,
            Mode::External,
        ),
        None => RunContext::core(actor, Mode::External),
    };
    reacting(run, event)
}

/// Collector `id` of `ext`, run as `actor` — the system on its schedule
/// or an event, whoever ran it by hand — caused by its trigger event when
/// it has one.
pub fn for_collector(
    catalog: &ExtensionCatalog,
    root: &Path,
    ext: &Extension,
    id: &str,
    actor: Actor,
    event: Option<&StoredEvent>,
) -> RunContext {
    let run = RunContext::contribution(
        contribution(catalog, root, ext, ContributionKind::Collector, id),
        actor,
        Mode::External,
    );
    match event {
        Some(e) => reacting(run, e),
        None => run,
    }
}

/// An extension command's run of `decl`, as `actor`.
pub fn for_command(
    catalog: &ExtensionCatalog,
    root: &Path,
    ext: &Extension,
    command: &str,
    actor: Actor,
) -> RunContext {
    RunContext::contribution(
        contribution(catalog, root, ext, ContributionKind::Command, command),
        actor,
        Mode::External,
    )
}

/// A dry run — `extension test`, a review — of something that would run
/// as the system: pure, nothing behind it to cause or anchor it.
pub fn for_dry_run() -> RunContext {
    RunContext::core(Actor::System, Mode::Pure)
}
