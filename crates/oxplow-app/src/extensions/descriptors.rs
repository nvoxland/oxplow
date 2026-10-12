//! Each loaded contribution as a [`Descriptor`]
//! (`oxplow_domain::contribution`): built from an extension's typed
//! declarations ([`describe`]) and checked across the catalog
//! ([`check_catalog`]), so the rules every kind shares — id grammar,
//! global-name collisions, ownership — are written once.
//!
//! A kind gains an adapter here when it moves onto the descriptor; the
//! rest keep their own checks until then.

use oxplow_domain::contribution::{
    extension_namespace, Collision, ContributionId, ContributionKind, DeclaredAt, Descriptor,
    Owner, Registry, RunsOn,
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
    for m in ext.models.iter().filter(|m| m.twin.is_none()) {
        let view = oxplow_db::models::extension_view(&ext.name, &m.decl.name);
        let mut d = new(ContributionKind::Model, &m.decl.name, RunsOn::Query);
        d.contract_version = Some(m.decl.version);
        d.global = Some(view.clone());
        d.aliases = vec![view];
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
        d.aliases = vec![t.event_type.clone()];
        d.source = declared_at(&t.declared_at);
        out.push(d);
    }
    for k in &ext.ref_kinds {
        let mut d = new(ContributionKind::RefKind, &k.kind, RunsOn::Declared);
        d.global = Some(k.kind.clone());
        d.aliases = vec![k.kind.clone()];
        d.source = declared_at(&k.declared_at);
        out.push(d);
    }
    for l in &ext.lenses {
        let mut d = new(ContributionKind::Lens, &l.slug, RunsOn::Mount);
        d.needs = l.needs.clone();
        d.aliases = vec![format!("lens:{}", l.id)];
        d.source = DeclaredAt {
            file: l.path.clone(),
            line: None,
        };
        out.push(d);
    }
    for p in &ext.pages {
        let mut d = new(ContributionKind::Page, &p.id, RunsOn::Mount);
        d.aliases = vec![p.page_ref.clone()];
        out.push(d);
    }
    for p in &ext.panels {
        let local =
            p.id.strip_prefix(&format!("{}/", ext.name))
                .unwrap_or(&p.id);
        let mut d = new(ContributionKind::Panel, local, RunsOn::Mount);
        d.aliases = vec![p.id.clone()];
        out.push(d);
    }
    for (n, slot) in ext.ui.slots.iter().enumerate() {
        let mut d = new(
            ContributionKind::Slot,
            &format!("{}-{n}", slot.slot),
            RunsOn::Mount,
        );
        d.aliases = vec![slot.slot.clone()];
        out.push(d);
    }
    for dec in &ext.ui.decorators {
        let n = dec.id.rsplit('/').next().unwrap_or_default();
        let mut d = new(
            ContributionKind::Decorator,
            &format!("decorator-{n}"),
            RunsOn::Mount,
        );
        d.aliases = vec![dec.id.clone()];
        out.push(d);
    }
    for r in &ext.ui.replacements {
        let mut d = new(ContributionKind::Replacement, &r.target, RunsOn::Mount);
        d.aliases = vec![r.id.clone()];
        out.push(d);
    }
    for c in &ext.custom_components {
        let mut d = new(ContributionKind::CustomComponent, &c.id, RunsOn::Mount);
        d.aliases = vec![format!("{}/{}", ext.name, c.id)];
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
    // A command's bus id is `<namespace>.<name>`: its local id is the
    // manifest's `<area>.<verb>`, its global the bus id.
    for c in &ext.commands {
        let local = c
            .name
            .strip_prefix(&format!("{}.", ext.namespace))
            .unwrap_or(&c.name);
        let mut d = new(ContributionKind::Command, local, RunsOn::Call);
        d.needs = c.needs.clone();
        d.global = Some(c.name.clone());
        d.aliases = vec![c.name.clone()];
        out.push(d);
    }
    for job in &ext.jobs {
        let mut d = new(ContributionKind::Job, &job.id, RunsOn::Call);
        d.contract_version = Some(job.version);
        d.needs = job.needs.clone();
        out.push(d);
    }
    for a in &ext.advisories {
        let mut d = new(ContributionKind::Advisory, &a.id, RunsOn::Agent);
        d.needs = a.needs.clone();
        d.aliases = vec![format!("{}/{}", ext.name, a.id)];
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
        d.aliases = vec![format!("{}/{}", i.capability, i.id)];
        out.push(d);
    }
    // A provider's id is its default instance's id, its refs' segment and
    // its commands' namespace: one per project.
    for p in &ext.providers {
        let mut d = new(ContributionKind::Provider, &p.id, RunsOn::Call);
        d.global = Some(p.id.clone());
        d.aliases = vec![format!("{}/{}", ext.name, p.id)];
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
    // A ref kind in two extensions' namespaces is the more specific
    // namespace's (`acme-pr`'s `acme_pr_` holds inside `acme`'s `acme_`),
    // so those are registered most specific first; equal namespaces, and
    // every other kind, go in catalog order.
    let mut by_specificity: Vec<usize> = (0..exts.len())
        .filter(|&i| exts[i].enabled && !exts[i].public_contracts.blocked)
        .collect();
    by_specificity.sort_by_key(|&i| std::cmp::Reverse(extension_namespace(&exts[i].name).len()));
    for i in by_specificity {
        for d in exts[i]
            .contributions
            .iter()
            .filter(|d| d.id.kind == ContributionKind::RefKind)
        {
            if let Err(collision) = registry.add(d.clone()) {
                refused[i].push(*collision);
            }
        }
    }
    for (i, ext) in exts
        .iter()
        .enumerate()
        .filter(|(_, e)| e.enabled && !e.public_contracts.blocked)
    {
        for d in ext
            .contributions
            .iter()
            .filter(|d| d.id.kind != ContributionKind::RefKind)
        {
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
    registry
}

/// Take the refused contribution out of its extension.
fn remove(ext: &mut Extension, c: &Collision) {
    let local = c.refused.local.as_str();
    match c.kind {
        ContributionKind::Model => ext
            .models
            .retain(|x| x.decl.name != local && x.twin.as_ref().is_none_or(|t| t.of != local)),
        ContributionKind::EventType => ext.event_types.types.retain(|x| x.event_type != local),
        ContributionKind::RefKind => ext.ref_kinds.retain(|x| x.kind != local),
        ContributionKind::Collector => ext.collectors.retain(|x| x.id != local),
        ContributionKind::Measure => ext.measures.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Metric => ext.metrics.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Dimension => ext.dimensions.retain(|x| x.key.as_deref() != Some(local)),
        ContributionKind::Command => {
            let global = c.global.as_str();
            ext.commands.retain(|x| x.name != global)
        }
        ContributionKind::Job => ext.jobs.retain(|job| job.id != local),
        ContributionKind::Skill => ext.skills.retain(|x| x.name != local),
        ContributionKind::Implementation => {
            let global = c.global.as_str();
            ext.implementations
                .retain(|x| format!("{}/{}", x.capability, x.id) != global)
        }
        ContributionKind::Provider => ext.providers.retain(|x| x.id != local),
        _ => {}
    }
    ext.contributions.retain(|d| d.id != c.refused);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use oxplow_domain::contribution::{ContributionKind as K, Descriptor, RunsOn};

    use super::*;
    use crate::extensions::load_extensions;

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
            "manifest: 2\nname: {name}\nsharing: private\nintent: {{ purpose: p, origin: null, examples: [] }}\n\
             models:\n  - name: prs\n    version: 3\n    description: Pull requests.\n    columns:\n      - {{ name: ref, type: TEXT, doc: The ref. }}\n      - {{ name: title, type: TEXT, doc: Its title. }}\n\
             event_types:\n  types:\n    - {{ type: {ns}.merged, v: 1, schema: merged.json, summary: Merged. }}\n\
             pages:\n  - {{ id: pr, title: Pull request, category: Work, lens: open }}\n\
             panels:\n  - {{ id: inbox, title: Inbox, scope: project, body: open }}\n\
             ref_kinds:\n  - {{ kind: {ns}_pr, label: Pull request, id: '^\\\\d+$', resolve: prs, page: pr, icon: git-pull-request }}\n\
             custom_components:\n  - {{ id: chart }}\n\
             ui:\n  slots:\n    - {{ slot: effort.review.details, lens: open }}\n    - {{ slot: vcs.commit.details, lens: open }}\n  decorators:\n    - {{ model: prs, kind: work_item, placement: ref-chip, label: title }}\n"
        )
    }

    fn write_acme(root: &Path, name: &str) {
        let dir = format!("oxplow/extensions/{name}");
        write(root, &format!("{dir}/extension.yaml"), &acme_manifest(name));
        write(
            root,
            &format!("{dir}/models/prs.sql"),
            "SELECT 'work_item:oxplow:tsk1' AS ref, 'One' AS title",
        );
        write(
            root,
            &format!("{dir}/lenses/open.yaml"),
            "title: Open\nquery: \"SELECT 1 AS n WHERE :change_id = :change_id\"\nparams:\n  - { name: change_id, label: Change, default: \"\" }\n",
        );
        write(root, &format!("{dir}/merged.json"), NUMBER_SCHEMA);
        write(
            root,
            &format!("{dir}/components/chart/index.html"),
            "<html></html>",
        );
    }

    fn acme() -> crate::extensions::Extension {
        let dir = tempfile::tempdir().unwrap();
        write_acme(dir.path(), "acme-pr");
        load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "acme-pr")
            .unwrap()
    }

    fn of(ext: &crate::extensions::Extension, kind: K) -> Vec<&Descriptor> {
        ext.contributions
            .iter()
            .filter(|d| d.id.kind == kind)
            .collect()
    }

    fn one(ext: &crate::extensions::Extension, kind: K) -> &Descriptor {
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
            "oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nsharing: private\nintent: { purpose: p, origin: null, examples: [] }\n\
             implementations:\n\
             \x20 - { capability: effort_policy, id: tidy, entry: policies/tidy.star }\n\
             \x20 - { capability: ai_provider, id: tidy, entry: providers/tidy.star }\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/acme/policies/tidy.star",
            "def transform(x):\n    return []\n",
        );
        write(
            dir.path(),
            "oxplow/extensions/acme/providers/tidy.star",
            "def request(x):\n    return {}\n\ndef response(x):\n    return {}\n",
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
                "acme/implementation/effort_policy.tidy",
                "acme/implementation/ai_provider.tidy"
            ]
        );
    }

    #[test]
    fn a_model_is_a_query_named_by_its_view() {
        let ext = acme();
        let d = one(&ext, K::Model);
        assert_eq!(d.id.to_string(), "acme-pr/model/prs");
        assert_eq!(d.trigger, RunsOn::Query);
        assert_eq!(d.contract_version, Some(3));
        assert_eq!(d.global.as_deref(), Some("v_acme_pr_prs"));
        assert_eq!(d.aliases, vec!["v_acme_pr_prs"]);
    }

    #[test]
    fn an_event_type_is_declared_and_named_by_its_type() {
        let ext = acme();
        let d = one(&ext, K::EventType);
        assert_eq!(d.id.to_string(), "acme-pr/event_type/acme_pr.merged");
        assert_eq!(d.trigger, RunsOn::Declared);
        assert_eq!(d.contract_version, Some(1));
        assert_eq!(d.global.as_deref(), Some("acme_pr.merged"));
        assert_eq!(d.aliases, vec!["acme_pr.merged"]);
        assert!(d.source.line.is_some(), "{:?}", d.source);
    }

    #[test]
    fn a_ref_kind_is_declared_and_named_by_its_kind() {
        let ext = acme();
        let d = one(&ext, K::RefKind);
        assert_eq!(d.id.to_string(), "acme-pr/ref_kind/acme_pr_pr");
        assert_eq!(d.trigger, RunsOn::Declared);
        assert_eq!(d.global.as_deref(), Some("acme_pr_pr"));
        assert_eq!(d.aliases, vec!["acme_pr_pr"]);
    }

    #[test]
    fn the_ui_kinds_are_mounted_and_claim_no_global_name() {
        let ext = acme();
        let lens = one(&ext, K::Lens);
        assert_eq!(lens.id.to_string(), "acme-pr/lens/open");
        assert_eq!(lens.aliases, vec!["lens:acme-pr/open"]);
        let page = one(&ext, K::Page);
        assert_eq!(page.id.to_string(), "acme-pr/page/pr");
        assert_eq!(page.aliases, vec!["page:ext.acme-pr.pr"]);
        let panel = one(&ext, K::Panel);
        assert_eq!(panel.id.to_string(), "acme-pr/panel/inbox");
        assert_eq!(panel.aliases, vec!["acme-pr/inbox"]);
        let component = one(&ext, K::CustomComponent);
        assert_eq!(component.id.to_string(), "acme-pr/custom_component/chart");
        assert_eq!(component.aliases, vec!["acme-pr/chart"]);
        for d in [lens, page, panel, component] {
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
                "acme-pr/slot/effort.review.details-0",
                "acme-pr/slot/vcs.commit.details-1"
            ]
        );
        assert_eq!(slots[0].aliases, vec!["effort.review.details"]);
        for d in slots {
            assert_eq!((d.trigger.clone(), d.global.clone()), (RunsOn::Mount, None));
        }
    }

    #[test]
    fn a_decorator_is_named_by_its_position() {
        let ext = acme();
        let d = one(&ext, K::Decorator);
        assert_eq!(d.id.to_string(), "acme-pr/decorator/decorator-0");
        assert_eq!(d.aliases, vec!["acme-pr/0"]);
        assert_eq!((d.trigger.clone(), d.global.clone()), (RunsOn::Mount, None));
    }

    #[test]
    fn a_replacement_is_mounted_by_its_target() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "oxplow/extensions/tracker/extension.yaml",
            "manifest: 2\nname: tracker\nintent: { purpose: p, origin: null, examples: [] }\n",
        );
        let mut ext = load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "tracker")
            .unwrap();
        ext.ui
            .replacements
            .push(crate::extensions::replacements::UiReplacement {
                id: "tracker/work_item.board".into(),
                extension: "tracker".into(),
                target: "work_item.board".into(),
                capability: "work_items".into(),
                bound_to_provider: false,
                lens_id: "tracker/board".into(),
                version: 1,
                label: "board".into(),
            });
        let d = describe(&ext)
            .into_iter()
            .find(|d| d.id.kind == K::Replacement)
            .expect("described");
        assert_eq!(d.id.to_string(), "tracker/replacement/work_item.board");
        assert_eq!(d.id.problem(), None);
        assert_eq!(d.aliases, vec!["tracker/work_item.board"]);
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
        second.path = "oxplow/extensions/acme-qr".into();
        // The model's view is named for its extension, so a second one's
        // never matches; claim it outright.
        second.contributions = describe(&second);
        for d in &mut second.contributions {
            if d.id.kind == K::Model {
                d.global = Some("v_acme_pr_prs".into());
            }
        }
        let mut exts = vec![first, second];
        check_catalog(&mut exts);
        let (first, second) = (&exts[0], &exts[1]);
        assert_eq!(first.errors, Vec::<String>::new());
        for (what, name) in [
            ("model", "v_acme_pr_prs"),
            ("event type", "acme_pr.merged"),
            ("ref kind", "acme_pr_pr"),
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
        assert!(second.models.is_empty() && second.ref_kinds.is_empty());
        assert!(second.event_types.types.is_empty());
        assert_eq!(first.models.len(), 1);
        assert_eq!(first.ref_kinds.len(), 1);
        assert_eq!(first.event_types.types.len(), 1);
        for kind in [K::Model, K::EventType, K::RefKind] {
            assert!(of(second, kind).is_empty(), "{kind:?}");
            assert_eq!(of(first, kind).len(), 1, "{kind:?}");
        }
    }

    /// A kind in the namespaces of two extensions (`acme`'s `acme_` holds
    /// `acme-pr`'s `acme_pr_`) is the more specific one's, whichever loads
    /// first.
    #[test]
    fn a_ref_kind_in_two_namespaces_is_the_more_specific_ones() {
        let manifest = |name: &str| {
            acme_manifest("acme-pr")
                .replace("name: acme-pr", &format!("name: {name}"))
                .replace("acme_pr_pr", "acme_pr_x")
        };
        let dir = tempfile::tempdir().unwrap();
        for name in ["acme", "acme-pr"] {
            write_acme(dir.path(), name);
            write(
                dir.path(),
                &format!("oxplow/extensions/{name}/extension.yaml"),
                &manifest(name),
            );
        }
        let exts = load_extensions(dir.path());
        let acme = exts.iter().find(|e| e.name == "acme").unwrap();
        let specific = exts.iter().find(|e| e.name == "acme-pr").unwrap();
        assert!(acme.ref_kinds.is_empty(), "{:?}", acme.errors);
        assert!(
            acme.errors
                .iter()
                .any(|e| e.contains("ref kind `acme_pr_x`") && e.contains("`acme-pr`")),
            "{:?}",
            acme.errors
        );
        assert_eq!(specific.ref_kinds.len(), 1);
        assert!(!specific.errors.iter().any(|e| e.contains("acme_pr_x")));
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
            ContributionKind::Lens => "[Lenses](../guide/lenses.md)",
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
                ContributionKind::Lens => "`lenses/*.yaml` (files, not a key)".to_string(),
                _ => format!("`{}`", k.key),
            };
            table.push_str(&format!("| {key} | {} | {} |\n", k.summary, more(k.kind)));
        }
        crate::test_fixtures::generated_section("docs/reference/extensions.md", "kinds", &table);
    }

    fn write(root: &std::path::Path, ext: &str, body: &str) {
        let dir = root.join("oxplow/extensions").join(ext);
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
        let skill = "skills:\n  - { name: deploy, file: skills/deploy.md }\n";
        write(dir.path(), "alpha", skill);
        write(dir.path(), "beta", skill);
        let exts = crate::extensions::load_extensions(dir.path());
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
            "skills:\n  - { name: deploy, file: skills/deploy.md }\nmeasures:\n  - { key: acme.loc, title: Lines, unit: lines, subjectKind: file }\n",
        );
        let exts = crate::extensions::load_extensions(dir.path());
        let acme = exts.iter().find(|e| e.name == "acme").unwrap();
        assert_eq!(acme.errors, Vec::<String>::new());
        let ids: Vec<String> = acme
            .contributions
            .iter()
            .map(|d| d.id.to_string())
            .collect();
        assert_eq!(ids, ["acme/skill/deploy", "acme/measure/acme.loc"]);
        assert!(acme
            .contributions
            .iter()
            .all(|d| d.owner.extension == "acme" && d.owner.origin == "project"));
    }
}
