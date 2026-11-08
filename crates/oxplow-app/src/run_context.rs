//! The app's constructors of a run's context (`oxplow_domain::run`,
//! `.context/extensions-permissions.md` "The run context"): each host that runs
//! extension code builds its run here, so every run names its
//! contribution, the extension's version and the catalog generation the
//! same way, and is caused by and anchored to what triggered it.

use std::path::Path;

use oxplow_domain::contribution::{ContributionId, ContributionKind};
use oxplow_domain::run::{ContributionRef, Mode, RunContext};
use oxplow_domain::{Actor, StoredEvent};

use crate::extension::extension_catalog::ExtensionCatalog;
use crate::extension::extensions::Extension;

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
        id: ContributionId::new(&ext.namespace, kind, local),
        extension: ext.name.clone(),
        version: ext.version.clone(),
        generation: Some(catalog.live_snapshot(root).generation().to_string()),
        deployment: ext.deployment.hash().map(str::to_owned),
    }
}

/// `ext`'s contribution `kind`/`local`, at its deployment, for a run that
/// holds no catalog generation while it goes on (a collector's, an effect's):
/// it belongs to its extension's deployment, which [`live_owner`] rechecks.
pub fn unpinned_contribution(
    ext: &Extension,
    kind: ContributionKind,
    local: &str,
) -> ContributionRef {
    ContributionRef {
        id: ContributionId::new(&ext.namespace, kind, local),
        extension: ext.name.clone(),
        version: ext.version.clone(),
        generation: None,
        deployment: ext.deployment.hash().map(str::to_owned),
    }
}

/// A check a running contribution must keep passing (its extension still
/// live at its version): `Err` says why it must stop.
pub type LiveCheck = std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// The live check of a run that holds no generation: its own extension is
/// still enabled, still the deployment the run started with (a redeploy is
/// another one whatever its version says), and still declares the
/// contribution. Run at each host call and at publication.
pub fn live_owner(
    catalog: std::sync::Arc<ExtensionCatalog>,
    root: &Path,
    owner: ContributionRef,
) -> std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync> {
    let root = root.to_path_buf();
    std::sync::Arc::new(move || {
        let snapshot = catalog.live_snapshot(&root);
        let current = crate::extension::extensions::owner(snapshot.extensions(), &owner.extension);
        match current {
            Some(ext)
                if ext.enabled
                    && ext.version == owner.version
                    && ext.deployment.hash() == owner.deployment.as_deref()
                    && ext.contributions.iter().any(|item| item.id == owner.id) =>
            {
                Ok(())
            }
            _ => Err(format!(
                "`{}`'s extension changed or was revoked during its run",
                owner.id
            )),
        }
    })
}

/// Dropping it stops the watch.
pub struct AuthorityWatch(tokio::task::JoinHandle<()>);

impl Drop for AuthorityWatch {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Check `run`'s live authority every 100 ms until the returned watch is
/// dropped: a guard that fails (an approval revoked, its extension disabled
/// or changed) sets the run's cancel flag, which a script worker polls. A
/// script computing with no host call would otherwise keep running until its
/// timeout. Hold it across any hosted script run given the run's cancel flag.
pub fn watch_authority(run: &RunContext) -> AuthorityWatch {
    let run = run.clone();
    AuthorityWatch(tokio::spawn(async move {
        loop {
            let check = run.clone();
            // A guard can hash files; keep that off the async workers. A
            // check that cannot run fails closed.
            if !matches!(
                tokio::task::spawn_blocking(move || check.check_authority()).await,
                Ok(Ok(()))
            ) {
                run.cancel().cancel();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }))
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
    catalog: &std::sync::Arc<ExtensionCatalog>,
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
    // It holds no catalog generation while it runs (a script may take
    // minutes): it belongs to its extension's version, rechecked at each
    // host call and when its commands land.
    let owner = unpinned_contribution(ext, ContributionKind::Effect, &decl.id);
    reacting(
        RunContext::contribution(owner.clone(), actor, Mode::External).guarded(live_owner(
            catalog.clone(),
            root,
            owner,
        )),
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

/// A dry run — `extension test`, a review — of something that would run
/// as the system: pure, nothing behind it to cause or anchor it.
pub fn for_dry_run() -> RunContext {
    RunContext::core(Actor::System, Mode::Pure)
}
