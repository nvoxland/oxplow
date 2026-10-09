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
        let mut d = new(ContributionKind::Implementation, &i.id, RunsOn::Call);
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
        ContributionKind::Command => {
            let global = c.global.as_str();
            ext.commands.retain(|x| x.name != global)
        }
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
    use super::*;

    /// The reference's "What it can declare" table, from the kinds' table:
    /// the two can't drift. `OXPLOW_BLESS=1` rewrites it.
    #[test]
    fn the_reference_lists_every_kind() {
        use oxplow_domain::contribution::{Stability, KINDS};
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
        let mut table =
            String::from("| Key | What it adds | Stability | More |\n|---|---|---|---|\n");
        for k in KINDS {
            let key = match k.kind {
                ContributionKind::Lens => "`lenses/*.yaml` (files, not a key)".to_string(),
                _ => format!("`{}`", k.key),
            };
            let stability = match k.stability {
                Stability::Stable => "stable",
                Stability::Experimental => "experimental",
            };
            table.push_str(&format!(
                "| {key} | {} | {stability} | {} |\n",
                k.summary,
                more(k.kind)
            ));
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/reference/extensions.md");
        let doc = std::fs::read_to_string(&path).unwrap();
        let begin = doc.find("<!-- kinds:begin").expect("the begin marker");
        let begin = begin + doc[begin..].find("-->").unwrap() + 3;
        let end = doc.find("<!-- kinds:end -->").expect("the end marker");
        let generated = format!("\n{table}");
        if std::env::var_os("OXPLOW_BLESS").is_some() {
            let blessed = format!("{}{generated}{}", &doc[..begin], &doc[end..]);
            std::fs::write(&path, blessed).unwrap();
            return;
        }
        assert_eq!(
            &doc[begin..end],
            generated,
            "docs/reference/extensions.md's kinds table is stale; bless it with OXPLOW_BLESS=1"
        );
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
