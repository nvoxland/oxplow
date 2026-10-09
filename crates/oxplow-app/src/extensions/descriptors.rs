//! Each loaded contribution as a [`Descriptor`]
//! (`oxplow_domain::contribution`): built from an extension's typed
//! declarations ([`describe`]) and checked across the catalog
//! ([`check_catalog`]), so the rules every kind shares — id grammar,
//! global-name collisions, ownership — are written once.
//!
//! A kind gains an adapter here when it moves onto the descriptor; the
//! rest keep their own checks until then.

use oxplow_domain::contribution::{
    Collision, ContributionId, ContributionKind, DeclaredAt, Descriptor, Owner, Registry, RunsOn,
};

use super::Extension;

/// The descriptors of `ext`'s contributions, in manifest order per kind.
pub fn describe(ext: &Extension) -> Vec<Descriptor> {
    let owner = Owner {
        extension: ext.name.clone(),
        origin: ext.origin.clone(),
        version: ext.version.clone(),
    };
    let source = DeclaredAt {
        file: format!("{}/extension.yaml", ext.path.trim_end_matches('/')),
        line: None,
    };
    let new = |kind: ContributionKind, local: &str, trigger: RunsOn| {
        Descriptor::new(
            ContributionId::new(&ext.name, kind, local),
            owner.clone(),
            trigger,
            source.clone(),
        )
    };
    let mut out = Vec::new();
    for c in &ext.collectors {
        let trigger = match &c.trigger {
            oxplow_config::collectors::Trigger::Manual => RunsOn::Call,
            oxplow_config::collectors::Trigger::Every { minutes } => RunsOn::Schedule {
                every: format!("{minutes}m"),
            },
            oxplow_config::collectors::Trigger::On { events, .. } => RunsOn::Events {
                types: events.clone(),
            },
            oxplow_config::collectors::Trigger::OnRun { .. } => RunsOn::Run,
        };
        let mut d = new(ContributionKind::Collector, &c.id, trigger);
        d.after = c.after.clone();
        // A fact collector's id is flat across owners (the fact engine's
        // key); an entity collector's is `<owner>/<id>`.
        if !c.facts.is_empty() {
            d.global = Some(c.id.clone());
        }
        d.aliases = vec![format!("{}/{}", ext.name, c.id)];
        out.push(d);
    }
    for e in &ext.effects {
        let mut d = new(
            ContributionKind::Effect,
            &e.id,
            RunsOn::Events {
                types: e.on.clone(),
            },
        );
        d.needs = e.needs.clone();
        d.after = e.after.clone();
        d.aliases = vec![e.name()];
        out.push(d);
    }
    for m in &ext.measures {
        if let Some(key) = &m.key {
            let mut d = new(ContributionKind::Measure, key, RunsOn::Query);
            d.global = Some(key.clone());
            out.push(d);
        }
    }
    // A `use:` entry turns on a metric someone else defines: it claims no
    // name.
    for m in &ext.metrics {
        if let Some(key) = &m.key {
            let mut d = new(ContributionKind::Metric, key, RunsOn::Query);
            d.global = Some(key.clone());
            out.push(d);
        }
    }
    for dim in &ext.dimensions {
        if let Some(key) = &dim.key {
            let mut d = new(ContributionKind::Dimension, key, RunsOn::Query);
            d.global = Some(key.clone());
            out.push(d);
        }
    }
    out
}

/// Each descriptor's id problem, as `ext`'s load errors.
pub fn check_ids(ext: &mut Extension) {
    let problems: Vec<String> = ext
        .contributions
        .iter()
        .filter_map(|d| d.id.problem().map(|p| format!("{}: {p}", d.source.file)))
        .collect();
    ext.errors.extend(problems);
}

/// Check the enabled extensions' contributions against each other, in
/// catalog order: a global name a kind doesn't share is held by the first
/// to declare it; a later one is a load error of its extension, naming
/// both, and is taken out of it so nothing downstream sees two.
pub fn check_catalog(exts: &mut [Extension]) -> Registry {
    let mut registry = Registry::default();
    for ext in exts.iter_mut().filter(|e| e.enabled) {
        let mut refused = Vec::new();
        for d in &ext.contributions {
            if let Err(collision) = registry.add(d.clone()) {
                refused.push(*collision);
            }
        }
        for c in refused {
            ext.errors.push(format!(
                "{}/extension.yaml: {c}",
                ext.path.trim_end_matches('/')
            ));
            remove(ext, &c);
        }
    }
    registry
}

/// Take the refused contribution out of its extension.
fn remove(ext: &mut Extension, c: &Collision) {
    let local = c.refused.local.as_str();
    match c.kind {
        ContributionKind::Collector => ext.collectors.retain(|x| x.id != local),
        ContributionKind::Measure => ext.measures.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Metric => ext.metrics.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Dimension => ext.dimensions.retain(|x| x.key.as_deref() != Some(local)),
        _ => {}
    }
    ext.contributions.retain(|d| d.id != c.refused);
}
