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
    // `file:line` as a vocabulary declaration records it.
    let declared_at = |at: &str| {
        let (file, line) = at.rsplit_once(':').unwrap_or((at, ""));
        DeclaredAt {
            file: file.to_string(),
            line: line.parse().ok(),
        }
    };
    for m in ext.datasets.iter().filter(|m| m.twin.is_none()) {
        let view = oxplow_db::datasets::extension_view(&ext.name, &m.decl.name);
        let mut d = new(ContributionKind::Dataset, &m.decl.name, RunsOn::Query);
        d.contract_version = Some(m.decl.version);
        d.global = Some(view.clone());
        d.source.file = format!("{}/{}", ext.path.trim_end_matches('/'), m.file);
        out.push(d);
    }
    // One descriptor per type, whatever versions of it are declared.
    let mut typed: Vec<&str> = Vec::new();
    for t in &ext.event_types.types {
        if typed.contains(&t.event_type.as_str()) {
            continue;
        }
        typed.push(&t.event_type);
        let mut d = new(ContributionKind::EventType, &t.event_type, RunsOn::Declared);
        d.contract_version = ext
            .event_types
            .types
            .iter()
            .filter(|o| o.event_type == t.event_type)
            .map(|o| o.v)
            .max();
        d.global = Some(t.event_type.clone());
        d.source = declared_at(&t.declared_at);
        out.push(d);
    }
    for r in &ext.resources {
        let mut d = new(
            ContributionKind::Resource,
            r.decl.name.name(),
            RunsOn::Declared,
        );
        d.global = Some(r.decl.name.to_string());
        d.source = declared_at(&r.declared_at);
        out.push(d);
    }
    for p in &ext.page_files {
        let mut d = new(ContributionKind::Page, &p.slug, RunsOn::Mount);
        d.needs = p.needs.clone();
        d.source = DeclaredAt {
            file: p.origin.running.clone(),
            line: None,
        };
        out.push(d);
    }
    for p in &ext.panels {
        let local =
            p.id.strip_prefix(&format!("{}/", ext.name))
                .unwrap_or(&p.id);
        let d = new(ContributionKind::Panel, local, RunsOn::Mount);
        out.push(d);
    }
    for (n, slot) in ext.ui.page_slots.iter().enumerate() {
        let d = new(ContributionKind::Slot, &slot.local_id(n), RunsOn::Mount);
        out.push(d);
    }
    for dec in &ext.ui.decorators {
        let n = dec.id.rsplit('/').next().unwrap_or_default();
        let d = new(
            ContributionKind::Decorator,
            &format!("decorator-{n}"),
            RunsOn::Mount,
        );
        out.push(d);
    }
    for r in &ext.ui.replacements {
        let d = new(ContributionKind::Replacement, &r.target, RunsOn::Mount);
        out.push(d);
    }
    for c in &ext.custom_components {
        let d = new(ContributionKind::CustomComponent, &c.id, RunsOn::Mount);
        out.push(d);
    }
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
        d.needs = c.service_needs();
        // A fact collector's id is flat across owners (the fact engine's
        // key); an entity collector's is `<owner>/<id>`.
        if !c.facts.is_empty() {
            d.global = Some(c.id.clone());
        }
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
        out.push(d);
    }
    // A command is named by its bus id (`<namespace>.<area>.<verb>`), its
    // ref `command:<bus id>`.
    for c in &ext.commands {
        let mut d = new(ContributionKind::Command, &c.name, RunsOn::Call);
        d.needs = c.needs.clone();
        d.global = Some(c.name.clone());
        out.push(d);
    }
    for job in &ext.jobs {
        let mut d = new(ContributionKind::Job, &job.id, RunsOn::Call);
        d.contract_version = Some(job.version);
        d.needs = job.needs.clone();
        out.push(d);
    }
    for profile in &ext.ai_profiles {
        out.push(new(
            ContributionKind::AiProfile,
            &profile.id,
            RunsOn::Declared,
        ));
    }
    for a in &ext.advisories {
        let mut d = new(ContributionKind::Advisory, &a.id, RunsOn::Agent);
        d.needs = a.needs.clone();
        out.push(d);
    }
    // A skill's name is what an agent's runtime files it under: one per
    // project.
    for k in &ext.skills {
        let mut d = new(ContributionKind::Skill, &k.name, RunsOn::Agent);
        d.needs = k.needs.clone();
        d.global = Some(k.name.clone());
        out.push(d);
    }
    // An implementation is chosen by `<capability>/<id>`.
    for i in &ext.implementations {
        let mut d = new(
            ContributionKind::Implementation,
            &oxplow_domain::contribution::implementation_local(&i.capability, &i.id),
            RunsOn::Call,
        );
        d.needs = i.needs.clone();
        d.global = Some(format!("{}/{}", i.capability, i.id));
        out.push(d);
    }
    // A provider's id is its default instance's id, its refs' segment and
    // its commands' namespace: one per project.
    for p in &ext.providers {
        let mut d = new(ContributionKind::Provider, &p.id, RunsOn::Call);
        d.global = Some(p.id.clone());
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
    let mut refused: Vec<Vec<Collision>> = vec![Vec::new(); exts.len()];
    for (i, ext) in exts
        .iter()
        .enumerate()
        .filter(|(_, e)| e.enabled && !e.public_contracts.blocked)
    {
        for d in &ext.contributions {
            if let Err(collision) = registry.add(d.clone()) {
                refused[i].push(*collision);
            }
        }
    }
    for (ext, refused) in exts.iter_mut().zip(refused) {
        for c in refused {
            ext.errors.push(format!(
                "{}/extension.yaml: {c}",
                ext.path.trim_end_matches('/')
            ));
            remove(ext, &c);
        }
    }
    order_catalog(exts, crate::event_pump::CORE_CONSUMERS);
    registry
}

/// Whether contribution `d` is ordered: effects and collectors, the two
/// kinds one event runs several of.
fn ordered(d: &Descriptor) -> bool {
    matches!(
        d.id.kind,
        ContributionKind::Effect | ContributionKind::Collector
    )
}

/// Where `ext`'s effect or collector `local` runs among what one event
/// reaches: its level and what it waits on, as the catalog compiled them.
pub fn placement(
    ext: &Extension,
    kind: ContributionKind,
    local: &str,
) -> (u32, Vec<oxplow_domain::ordering::After>) {
    ext.contributions
        .iter()
        .find(|d| d.id.kind == kind && d.id.local == local)
        .map(|d| {
            (
                d.order_level.unwrap_or(0),
                d.after
                    .iter()
                    .filter_map(|a| oxplow_domain::ordering::After::parse(a).ok())
                    .collect(),
            )
        })
        .unwrap_or_default()
}

/// The pump consumers a contribution of `kind` waiting on `after` makes
/// its consumer wait on: a core consumer by name, the other kind's
/// consumer for a contribution of the other kind. A target of its own kind
/// runs earlier in the same delivery.
pub fn gates(kind: ContributionKind, after: &[oxplow_domain::ordering::After]) -> Vec<String> {
    use oxplow_domain::ordering::After;
    let mut out: Vec<String> = Vec::new();
    for a in after {
        let gate = match a {
            After::Consumer(name) => Some(name.as_str()),
            After::Contribution(target) if target.kind() != kind => consumer_of(target.kind()),
            After::Contribution(_) => None,
        };
        if let Some(gate) = gate {
            if !out.iter().any(|g| g == gate) {
                out.push(gate.to_string());
            }
        }
    }
    out
}

/// The consumer that runs contributions of `kind`.
pub fn consumer_of(kind: ContributionKind) -> Option<&'static str> {
    match kind {
        ContributionKind::Effect => Some(crate::effect_triggers::NAME),
        ContributionKind::Collector => Some(crate::collector_triggers::NAME),
        _ => None,
    }
}

/// Resolve the enabled extensions' effects' and collectors' `after:` to
/// refs and compile one order across them (`oxplow_domain::ordering`). A
/// target that resolves to nothing, or a cycle, is a load error of the
/// declaring extension, which loses the contribution; a target in a
/// disabled extension is dropped with a warning, since it can't run. An
/// order the pump couldn't honor — effects waiting on collectors and
/// collectors on effects at once, the two consumers then each waiting on
/// the other — is a load error of every contribution that crosses.
fn order_catalog(exts: &mut [Extension], consumers: &[&str]) {
    use oxplow_domain::ordering::{resolve, After, Node, Order, OrderError};
    let live = |e: &Extension| e.enabled && !e.public_contracts.blocked;
    // Extensions the catalog has but doesn't run: a target in one can't run.
    let idle: std::collections::BTreeSet<String> = exts
        .iter()
        .filter(|e| !live(e))
        .map(|e| e.name.clone())
        .collect();
    // Resolve each entry as its extension wrote it.
    for ext in exts.iter_mut().filter(|e| live(e)) {
        let own_effects: Vec<String> = ext.effects.iter().map(|e| e.id.clone()).collect();
        let own_collectors: Vec<String> = ext.collectors.iter().map(|c| c.id.clone()).collect();
        let own = |kind: ContributionKind, id: &str| match kind {
            ContributionKind::Effect => own_effects.iter().any(|e| e == id),
            ContributionKind::Collector => own_collectors.iter().any(|c| c == id),
            _ => false,
        };
        let mut refused = Vec::new();
        let mut warnings = Vec::new();
        for d in ext.contributions.iter_mut().filter(|d| ordered(d)) {
            let mut resolved = Vec::new();
            for entry in &d.after {
                match resolve(&ext.name, entry, &own, consumers) {
                    Ok(After::Contribution(target))
                        if target.extension().is_some_and(|x| idle.contains(x)) =>
                    {
                        warnings.push(format!(
                            "{}: runs after {target}, whose extension is disabled: it runs \
                             without waiting",
                            d.id
                        ))
                    }
                    Ok(after) => resolved.push(after.to_ref()),
                    Err(e) => refused.push((d.id.clone(), format!("{}: `after`: {e}", d.id))),
                }
            }
            d.after = resolved;
        }
        let file = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
        ext.warnings
            .extend(warnings.into_iter().map(|w| format!("{file}: {w}")));
        for (id, message) in refused {
            ext.errors.push(format!("{file}: {message}"));
            remove_id(ext, &id);
        }
    }
    // Compile, taking out what can't be ordered until what's left compiles.
    loop {
        let nodes: Vec<Node> = exts
            .iter()
            .filter(|e| live(e))
            .flat_map(|e| e.contributions.iter().filter(|d| ordered(d)))
            .map(|d| Node {
                id: d.id.reference(),
                after: d
                    .after
                    .iter()
                    .filter_map(|a| After::parse(a).ok())
                    .collect(),
            })
            .collect();
        let mut problems: Vec<(oxplow_domain::contribution::ContributionRef, String)> =
            match Order::compile(&nodes, consumers) {
                Ok(order) => {
                    let crossing = crossings(&nodes);
                    if crossing.is_empty() {
                        for ext in exts.iter_mut().filter(|e| live(e)) {
                            for d in ext.contributions.iter_mut().filter(|d| ordered(d)) {
                                d.order_level = order.level(&d.id.reference()).map(|l| l as u32);
                            }
                        }
                        return;
                    }
                    crossing
                }
                Err(errors) => errors
                    .into_iter()
                    .flat_map(|e| match &e {
                        OrderError::Unknown { by, .. } => vec![(by.clone(), e.to_string())],
                        OrderError::Cycle { cycle } => {
                            cycle.iter().map(|c| (c.clone(), e.to_string())).collect()
                        }
                    })
                    .collect(),
            };
        problems.dedup();
        for (id, message) in problems {
            let Some(ext) = exts
                .iter_mut()
                .find(|e| Some(e.name.as_str()) == id.extension())
            else {
                continue;
            };
            let file = format!("{}/extension.yaml", ext.path.trim_end_matches('/'));
            let error = format!("{file}: {message}");
            if !ext.errors.contains(&error) {
                ext.errors.push(error);
            }
            if let Some(id) = id.id() {
                remove_id(ext, &id);
            }
        }
    }
}

/// The contributions whose `after:` crosses between the two consumers, when
/// it crosses both ways: each consumer would wait on the other.
fn crossings(
    nodes: &[oxplow_domain::ordering::Node],
) -> Vec<(oxplow_domain::contribution::ContributionRef, String)> {
    use oxplow_domain::ordering::After;
    let cross: Vec<(&oxplow_domain::ordering::Node, &After)> = nodes
        .iter()
        .flat_map(|n| n.after.iter().map(move |a| (n, a)))
        .filter(|(n, a)| matches!(a, After::Contribution(t) if t.kind() != n.id.kind()))
        .collect();
    let ways: std::collections::BTreeSet<ContributionKind> =
        cross.iter().map(|(n, _)| n.id.kind()).collect();
    if ways.len() < 2 {
        return Vec::new();
    }
    cross
        .into_iter()
        .map(|(n, a)| {
            (
                n.id.clone(),
                format!(
                    "{} runs after {}, while elsewhere effects and collectors wait on each                      other the other way: on one event the two would each wait for the other.                      Order effects after collectors or collectors after effects, not both",
                    n.id,
                    a.to_ref()
                ),
            )
        })
        .collect()
}

/// Take contribution `id` (an effect or a collector) out of `ext`.
fn remove_id(ext: &mut Extension, id: &ContributionId) {
    match id.kind {
        ContributionKind::Effect => ext.effects.retain(|e| e.id != id.local),
        ContributionKind::Collector => ext.collectors.retain(|c| c.id != id.local),
        _ => {}
    }
    ext.contributions.retain(|d| &d.id != id);
}

/// Take the refused contribution out of its extension.
fn remove(ext: &mut Extension, c: &Collision) {
    let local = c.refused.local.as_str();
    match c.kind {
        ContributionKind::Dataset => ext
            .datasets
            .retain(|x| x.decl.name != local && x.twin.as_ref().is_none_or(|t| t.of != local)),
        ContributionKind::EventType => ext.event_types.types.retain(|x| x.event_type != local),
        ContributionKind::Resource => ext.resources.retain(|x| x.decl.name.name() != local),
        ContributionKind::Collector => ext.collectors.retain(|x| x.id != local),
        ContributionKind::Measure => ext.measures.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Metric => ext.metrics.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Dimension => ext.dimensions.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Command => {
            let global = c.global.as_str();
            ext.commands.retain(|x| x.name != global)
        }
        ContributionKind::Job => ext.jobs.retain(|job| job.id != local),
        ContributionKind::AiProfile => ext.ai_profiles.retain(|x| x.id != local),
        ContributionKind::Skill => ext.skills.retain(|x| x.name != local),
        ContributionKind::Implementation => {
            let global = c.global.as_str();
            ext.implementations
                .retain(|x| format!("{}/{}", x.capability, x.id) != global)
        }
        ContributionKind::Provider => ext.providers.retain(|x| x.id != local),
        ContributionKind::Effect => ext.effects.retain(|x| x.id != local),
        _ => {}
    }
    ext.contributions.retain(|d| d.id != c.refused);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use oxplow_domain::contribution::{ContributionKind as K, Descriptor, RunsOn};

    use super::*;
    use crate::extension::extensions::load_extensions;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    const NUMBER_SCHEMA: &str =
        r#"{"type":"object","required":["n"],"properties":{"n":{"type":"integer"}}}"#;

    /// An extension with one of each data and UI kind (but a replacement,
    /// which needs a provider: that one is built by hand below).
    fn acme_manifest(name: &str) -> String {
        let ns = name.replace('-', "_");
        format!(
            "manifest: 2\nname: {name}\nintent: {{ purpose: p, origin: null, examples: [] }}\n\
             datasets:\n  - name: prs\n    version: 3\n    description: Pull requests.\n    fields:\n      - {{ name: uri, type: TEXT, doc: The ref. }}\n      - {{ name: title, type: TEXT, doc: Its title. }}\n\
             event_types:\n  types:\n    - {{ type: {ns}.merged, v: 1, schema: merged.json, summary: Merged. }}\n\
             panels:\n  - {{ id: inbox, title: Inbox, scope: project, body: open }}\n\
             resources:\n  - {{ name: pr, label: Pull request, params: {{ id: {{ required: true, pattern: '^[0-9]+$' }} }}, resolve: prs, icon: git-pull-request }}\n\
             custom_components:\n  - {{ id: chart }}\n\
             ui:\n  mounts:\n    - {{ page: open, at: oxplow-foundation/diff.file-header }}\n    - {{ page: open, at: oxplow-foundation/diff.file-header }}\n  decorators:\n    - {{ dataset: prs, resource: oxplow/work_item, placement: ref-chip, label: title }}\n"
        )
    }

    fn write_acme(root: &Path, name: &str) {
        let dir = format!(".oxplow/extensions/{name}");
        write(root, &format!("{dir}/extension.yaml"), &acme_manifest(name));
        write(
            root,
            &format!("{dir}/datasets/prs.sql"),
            "SELECT 'oxplow:oxplow/work_item?id=tsk1&provider=oxplow' AS uri, 'One' AS title",
        );
        write(
            root,
            &format!("{dir}/pages/open.yaml"),
            "title: Open\nparams:\n  - { name: path, label: Path, default: \"\" }\n\
             loaders:\n  n: { sql: \"SELECT 1 AS n WHERE :path = :path\" }\nview: { viz: number, from: n }\n",
        );
        write(root, &format!("{dir}/merged.json"), NUMBER_SCHEMA);
        write(
            root,
            &format!("{dir}/components/chart/index.html"),
            "<html></html>",
        );
    }

    fn acme() -> crate::extension::extensions::Extension {
        let dir = tempfile::tempdir().unwrap();
        write_acme(dir.path(), "acme-pr");
        load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "acme-pr")
            .unwrap()
    }

    fn of(ext: &crate::extension::extensions::Extension, kind: K) -> Vec<&Descriptor> {
        ext.contributions
            .iter()
            .filter(|d| d.id.kind == kind)
            .collect()
    }

    fn one(ext: &crate::extension::extensions::Extension, kind: K) -> &Descriptor {
        let all = of(ext, kind);
        assert_eq!(all.len(), 1, "{kind:?}: {all:?} (errors {:?})", ext.errors);
        all[0]
    }

    #[test]
    fn the_fixture_extension_loads_clean() {
        assert_eq!(acme().errors, Vec::<String>::new());
    }

    /// An implementation is chosen by its capability and id, so one id
    /// under two capabilities is two contributions.
    #[test]
    fn an_implementation_is_named_by_its_capability_and_id() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent: { purpose: p, origin: null, examples: [] }\n\
             permissions: [ai.compute]\n\
             implementations:\n\
             \x20 - { capability: effort_policy, id: tidy, entry: policies/tidy.star }\n\
             \x20 - { capability: ai_provider, id: tidy, provider: openai_compatible }\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/acme/policies/tidy.star",
            "def transform(x):\n    return []\n",
        );
        let ext = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "acme")
            .unwrap();
        let ids: Vec<String> = of(&ext, K::Implementation)
            .iter()
            .map(|d| d.id.to_string())
            .collect();
        assert_eq!(ext.errors, Vec::<String>::new());
        assert_eq!(
            ids,
            [
                "oxplow:oxplow/implementation?implementation=acme/effort_policy.tidy",
                "oxplow:oxplow/implementation?implementation=acme/ai_provider.tidy"
            ]
        );
    }

    #[test]
    fn a_dataset_is_a_query_named_by_its_view() {
        let ext = acme();
        let d = one(&ext, K::Dataset);
        assert_eq!(
            d.id.to_string(),
            "oxplow:oxplow/dataset?dataset=acme-pr/prs"
        );
        assert_eq!(d.trigger, RunsOn::Query);
        assert_eq!(d.contract_version, Some(3));
        assert_eq!(d.global.as_deref(), Some("v_acme_pr_prs"));
    }

    #[test]
    fn an_event_type_is_declared_and_named_by_its_type() {
        let ext = acme();
        let d = one(&ext, K::EventType);
        assert_eq!(
            d.id.to_string(),
            "oxplow:oxplow/event_type?event_type=acme_pr.merged"
        );
        assert_eq!(d.trigger, RunsOn::Declared);
        assert_eq!(d.contract_version, Some(1));
        assert_eq!(d.global.as_deref(), Some("acme_pr.merged"));
        assert!(d.source.line.is_some(), "{:?}", d.source);
    }

    #[test]
    fn a_resource_is_declared_and_named_by_its_extension_and_name() {
        let ext = acme();
        let d = one(&ext, K::Resource);
        assert_eq!(d.id.local, "pr");
        assert_eq!(d.trigger, RunsOn::Declared);
        assert_eq!(d.global.as_deref(), Some("acme-pr/pr"));
    }

    #[test]
    fn the_ui_kinds_are_mounted_and_claim_no_global_name() {
        let ext = acme();
        let page = one(&ext, K::Page);
        assert_eq!(page.id.to_string(), "oxplow:oxplow/page?page=acme-pr/open");
        let panel = one(&ext, K::Panel);
        assert_eq!(
            panel.id.to_string(),
            "oxplow:oxplow/panel?panel=acme-pr/inbox"
        );
        let component = one(&ext, K::CustomComponent);
        assert_eq!(
            component.id.to_string(),
            "oxplow:oxplow/custom_component?custom_component=acme-pr/chart"
        );
        for d in [page, panel, component] {
            assert_eq!(d.trigger, RunsOn::Mount, "{}", d.id);
            assert_eq!(d.global, None, "{}", d.id);
        }
    }

    #[test]
    fn slot_mounts_are_numbered_and_share_their_slot() {
        let ext = acme();
        let slots = of(&ext, K::Slot);
        let ids: Vec<String> = slots.iter().map(|d| d.id.to_string()).collect();
        assert_eq!(
            ids,
            vec![
                "oxplow:oxplow/slot?slot=acme-pr/oxplow-foundation.diff.file-header-0",
                "oxplow:oxplow/slot?slot=acme-pr/oxplow-foundation.diff.file-header-1"
            ]
        );
        for d in slots {
            assert_eq!((d.trigger.clone(), d.global.clone()), (RunsOn::Mount, None));
        }
    }

    #[test]
    fn a_decorator_is_named_by_its_position() {
        let ext = acme();
        let d = one(&ext, K::Decorator);
        assert_eq!(
            d.id.to_string(),
            "oxplow:oxplow/decorator?decorator=acme-pr/decorator-0"
        );
        assert_eq!((d.trigger.clone(), d.global.clone()), (RunsOn::Mount, None));
    }

    #[test]
    fn a_replacement_is_mounted_by_its_target() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".oxplow/extensions/tracker/extension.yaml",
            "manifest: 2\nname: tracker\nintent: { purpose: p, origin: null, examples: [] }\n",
        );
        let mut ext = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "tracker")
            .unwrap();
        ext.ui
            .replacements
            .push(crate::extension::extensions::replacements::UiReplacement {
                id: "tracker/work_item.board".into(),
                extension: "tracker".into(),
                target: "work_item.board".into(),
                capability: "work_items".into(),
                bound_to_provider: false,
                page: "tracker/board".into(),
                version: 1,
                view: vec![],
                label: "board".into(),
            });
        let d = describe(&ext)
            .into_iter()
            .find(|d| d.id.kind == K::Replacement)
            .expect("described");
        assert_eq!(
            d.id.to_string(),
            "oxplow:oxplow/replacement?replacement=tracker/work_item.board"
        );
        assert_eq!(d.id.problem(), None);
        assert_eq!((d.trigger.clone(), d.global.clone()), (RunsOn::Mount, None));
    }

    /// Two extensions claiming one event type and one ref kind: the first
    /// keeps each, one load error per kind names both, and the second's is
    /// gone. (Valid extension names make distinct namespaces, so a second
    /// extension claims the first's names only by declaring them outright;
    /// here its manifest is the first's.)
    #[test]
    fn two_extensions_claiming_one_name_keep_the_first_and_report_each_kind() {
        let first = acme();
        let mut second = acme();
        second.name = "acme-qr".into();
        second.path = ".oxplow/extensions/acme-qr".into();
        // The dataset's view is named for its extension, so a second one's
        // never matches; claim it outright.
        second.contributions = describe(&second);
        for d in &mut second.contributions {
            if d.id.kind == K::Dataset {
                d.global = Some("v_acme_pr_prs".into());
            }
        }
        let mut exts = vec![first, second];
        check_catalog(&mut exts);
        let (first, second) = (&exts[0], &exts[1]);
        assert_eq!(first.errors, Vec::<String>::new());
        for (what, name) in [
            ("dataset", "v_acme_pr_prs"),
            ("event type", "acme_pr.merged"),
            ("resource", "acme-pr/pr"),
        ] {
            let reports: Vec<&String> = second
                .errors
                .iter()
                .filter(|e| e.contains(&format!("{what} `{name}`")))
                .collect();
            assert_eq!(reports.len(), 1, "{what}: {:?}", second.errors);
            assert!(
                reports[0].contains("`acme-pr`") && reports[0].contains("`acme-qr`"),
                "{}",
                reports[0]
            );
        }
        assert!(second.datasets.is_empty() && second.resources.is_empty());
        assert!(second.event_types.types.is_empty());
        assert_eq!(first.datasets.len(), 1);
        assert_eq!(first.resources.len(), 1);
        assert_eq!(first.event_types.types.len(), 1);
        for kind in [K::Dataset, K::EventType, K::Resource] {
            assert!(of(second, kind).is_empty(), "{kind:?}");
            assert_eq!(of(first, kind).len(), 1, "{kind:?}");
        }
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;

    /// The reference's "What it can declare" table, from the kinds' table:
    /// the two can't drift. `OXPLOW_BLESS=1` rewrites it.
    #[test]
    fn the_reference_lists_every_kind() {
        use oxplow_domain::contribution::KINDS;
        let more = |k: ContributionKind| match k {
            ContributionKind::Page => "[Pages](../guide/pages.md)",
            ContributionKind::Measure | ContributionKind::Metric | ContributionKind::Dimension => {
                "[Metrics](../guide/metrics.md)"
            }
            ContributionKind::Collector
            | ContributionKind::Command
            | ContributionKind::Effect
            | ContributionKind::Implementation => "below",
            ContributionKind::Provider => "[Provider protocol](provider-protocol.md)",
            _ => "",
        };
        let mut table = String::from("| Key | What it adds | More |\n|---|---|---|\n");
        for k in KINDS {
            let key = match k.kind {
                ContributionKind::Page => "`pages/*.yaml` (files, not a key)".to_string(),
                _ => format!("`{}`", k.key),
            };
            table.push_str(&format!("| {key} | {} | {} |\n", k.summary, more(k.kind)));
        }
        crate::test_fixtures::generated_section("docs/reference/extensions.md", "kinds", &table);
    }

    fn write(root: &std::path::Path, ext: &str, body: &str) {
        let dir = root.join(".oxplow/extensions").join(ext);
        std::fs::create_dir_all(dir.join("skills")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!(
                "manifest: 2\nname: {ext}\nintent:\n  purpose: p\n  examples: [{{ name: a }}]\n{body}"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("skills/deploy.md"),
            "---\nname: deploy\ndescription: d\n---\nShip it.\n",
        )
        .unwrap();
    }

    /// Two extensions naming one skill: the first (alphabetically) keeps
    /// it, the other gets a load error naming both and loses its skill.
    #[test]
    fn a_skill_name_is_one_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let skill =
            "permissions: [agent.write]\nskills:\n  - { name: deploy, file: skills/deploy.md }\n";
        write(dir.path(), "alpha", skill);
        write(dir.path(), "beta", skill);
        let exts = crate::extension::extensions::load_extensions(dir.path());
        let alpha = exts.iter().find(|e| e.name == "alpha").unwrap();
        let beta = exts.iter().find(|e| e.name == "beta").unwrap();
        assert_eq!(alpha.errors, Vec::<String>::new());
        assert_eq!(alpha.skills.len(), 1);
        assert!(beta.skills.is_empty());
        let errors: Vec<_> = beta
            .errors
            .iter()
            .filter(|e| e.contains("`deploy`"))
            .collect();
        assert_eq!(errors.len(), 1, "{:?}", beta.errors);
        assert!(
            errors[0].contains("`alpha`") && errors[0].contains("`beta`"),
            "{}",
            errors[0]
        );
        assert!(beta
            .contributions
            .iter()
            .all(|d| d.id.kind != ContributionKind::Skill));
    }

    /// Every contribution an extension loads is described, with its owner.
    #[test]
    fn an_extensions_contributions_are_described() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "acme",
            "permissions: [agent.write]\nskills:\n  - { name: deploy, file: skills/deploy.md }\nmeasures:\n  - { key: acme.loc, title: Lines, unit: lines, subjectKind: file }\n",
        );
        let exts = crate::extension::extensions::load_extensions(dir.path());
        let acme = exts.iter().find(|e| e.name == "acme").unwrap();
        assert_eq!(acme.errors, Vec::<String>::new());
        let ids: Vec<String> = acme
            .contributions
            .iter()
            .map(|d| d.id.to_string())
            .collect();
        assert_eq!(
            ids,
            [
                "oxplow:oxplow/skill?skill=acme/deploy",
                "oxplow:oxplow/measure?measure=acme.loc"
            ]
        );
        assert!(acme
            .contributions
            .iter()
            .all(|d| d.owner.extension == "acme" && d.owner.origin == "project"));
    }
}

#[cfg(test)]
mod ordering_tests {
    use std::path::Path;

    use crate::extension::extensions::{load_extensions, Extension};

    const SKIP: &str = "def transform(input):\n    return {\"skip\": \"x\"}\n";

    /// Write extension `name` with these `effects:` entries (each
    /// `id: after-list`) and `collectors:` entries, then load the catalog.
    fn write(root: &Path, name: &str, effects: &[(&str, &str)], collectors: &[(&str, &str)]) {
        let dir = root.join(format!(".oxplow/extensions/{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut yaml = format!(
            "manifest: 2\nname: {name}\nintent: {{ purpose: p, origin: null, examples: [] }}\n"
        );
        if !effects.is_empty() {
            yaml.push_str("effects:\n");
            for (id, after) in effects {
                yaml.push_str(&format!(
                    "  - {{ id: {id}, summary: s, on: [work_item.state_changed], entry: e.star, after: [{after}] }}\n"
                ));
            }
        }
        if !collectors.is_empty() {
            yaml.push_str("collectors:\n");
            for (id, after) in collectors {
                yaml.push_str(&format!(
                    "  - {{ id: {id}, runtime: starlark, entry: c.star, on: [work_item.state_changed], after: [{after}], entities: [{{ name: {id}, key: k, columns: {{ k: text }} }}] }}\n"
                ));
            }
        }
        std::fs::write(dir.join("extension.yaml"), yaml).unwrap();
        std::fs::write(dir.join("e.star"), SKIP).unwrap();
        std::fs::write(
            dir.join("c.star"),
            "def transform(input):\n    return {\"entities\": {}}\n",
        )
        .unwrap();
    }

    fn named<'a>(exts: &'a [Extension], name: &str) -> &'a Extension {
        exts.iter().find(|e| e.name == name).unwrap()
    }

    fn placed(ext: &Extension, kind: &str, id: &str) -> Option<(u32, Vec<String>)> {
        ext.contributions
            .iter()
            .find(|d| d.id.kind.segment() == kind && d.id.local == id)
            .map(|d| (d.order_level.unwrap(), d.after.clone()))
    }

    #[test]
    fn after_resolves_to_refs_and_levels_across_extensions() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "first", &[("seed", "")], &[]);
        write(
            dir.path(),
            "second",
            &[
                ("notify", "react"),
                (
                    "react",
                    "\"oxplow:oxplow/effect?effect=first/seed\", change.analyze",
                ),
            ],
            &[("gather", "")],
        );
        let exts = load_extensions(dir.path());
        let second = named(&exts, "second");
        assert_eq!(second.errors, Vec::<String>::new());
        assert_eq!(
            placed(named(&exts, "first"), "effect", "seed"),
            Some((0, vec![]))
        );
        assert_eq!(
            placed(second, "effect", "react"),
            Some((
                1,
                vec![
                    "oxplow:oxplow/effect?effect=first/seed".to_string(),
                    "oxplow:oxplow/consumer?name=change.analyze".to_string()
                ]
            ))
        );
        assert_eq!(
            placed(second, "effect", "notify"),
            Some((
                2,
                vec!["oxplow:oxplow/effect?effect=second/react".to_string()]
            ))
        );
        assert_eq!(placed(second, "collector", "gather"), Some((0, vec![])));
    }

    #[test]
    fn an_unknown_target_is_a_load_error_and_the_contribution_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "acme",
            &[
                ("bare", "no.such.thing"),
                ("elsewhere", "\"oxplow:oxplow/effect?effect=nobody/home\""),
                ("fine", ""),
            ],
            &[],
        );
        let exts = load_extensions(dir.path());
        let acme = named(&exts, "acme");
        let errors = acme.errors.join("\n");
        assert!(
            errors.contains(
                "oxplow:oxplow/effect?effect=acme/bare: `after`: `no.such.thing` is none of"
            ),
            "{errors}"
        );
        assert!(
            errors.contains(
                "oxplow:oxplow/effect?effect=acme/elsewhere runs after oxplow:oxplow/effect?effect=nobody/home, which nothing declares"
            ),
            "{errors}"
        );
        let ids: Vec<&str> = acme.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["fine"]);
    }

    #[test]
    fn a_cycle_is_a_load_error_of_every_member() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "acme",
            &[("a", "b"), ("b", "c"), ("c", "a"), ("me", "me"), ("ok", "")],
            &[],
        );
        let exts = load_extensions(dir.path());
        let acme = named(&exts, "acme");
        let errors = acme.errors.join("\n");
        assert!(
            errors.contains(
                "these contributions wait on each other: oxplow:oxplow/effect?effect=acme/a → oxplow:oxplow/effect?effect=acme/b → oxplow:oxplow/effect?effect=acme/c"
            ),
            "{errors}"
        );
        assert!(
            errors.contains(
                "these contributions wait on each other: oxplow:oxplow/effect?effect=acme/me"
            ),
            "{errors}"
        );
        assert_eq!(acme.errors.len(), 2, "{errors}");
        let ids: Vec<&str> = acme.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["ok"]);
    }

    #[test]
    fn a_target_in_a_disabled_extension_is_dropped_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "first", &[("seed", "")], &[]);
        write(
            dir.path(),
            "second",
            &[("react", "\"oxplow:oxplow/effect?effect=first/seed\"")],
            &[],
        );
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/personal.yaml"),
            "disableExtensions: [first]\n",
        )
        .unwrap();
        let exts = load_extensions(dir.path());
        let second = named(&exts, "second");
        assert_eq!(second.errors, Vec::<String>::new());
        assert!(
            second.warnings.iter().any(|w| w.contains(
                "runs after oxplow:oxplow/effect?effect=first/seed, whose extension is disabled"
            )),
            "{:?}",
            second.warnings
        );
        assert_eq!(placed(second, "effect", "react"), Some((0, vec![])));
    }

    #[test]
    fn effects_and_collectors_waiting_on_each_other_both_ways_is_a_load_error() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "acme",
            &[("react", "\"collector:acme/gather\""), ("seed", "")],
            &[("gather", ""), ("late", "\"effect:acme/seed\"")],
        );
        let exts = load_extensions(dir.path());
        let acme = named(&exts, "acme");
        let errors = acme.errors.join("\n");
        assert!(
            errors.contains("the two would each wait for the other"),
            "{errors}"
        );
        // One way alone is fine.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "acme",
            &[("react", "\"collector:acme/gather\"")],
            &[("gather", "")],
        );
        let exts = load_extensions(dir.path());
        assert_eq!(named(&exts, "acme").errors, Vec::<String>::new());
    }
}
