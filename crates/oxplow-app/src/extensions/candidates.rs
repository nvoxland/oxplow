//! Candidate catalogs for authoring checks.

use std::collections::BTreeSet;
use std::path::Path;

use super::Extension;

pub(crate) struct CandidateCatalog {
    pub extensions: Vec<Extension>,
    target: usize,
    pub dependents: BTreeSet<String>,
}

impl CandidateCatalog {
    pub fn candidate(&self) -> &Extension {
        &self.extensions[self.target]
    }
}

pub(crate) fn build(root: &Path, project: &Path, mut candidate: Extension) -> CandidateCatalog {
    let settings = oxplow_config::extension_settings(project);
    let mut sources = super::load_sources(root, &settings);
    let settings = super::sources_on(settings, &sources);
    let baseline = super::resolve_catalog(sources.clone(), &settings);
    let previously_active: BTreeSet<_> = baseline
        .iter()
        .filter(|ext| ext.enabled)
        .map(|ext| ext.name.clone())
        .collect();
    let name = candidate.name.clone();
    let origin = candidate.origin.clone();
    // Authoring previews inspect the candidate, including a disabled package,
    // without changing the project's enabling policy or publishing authority.
    candidate.enabled = true;
    if let Some(index) = sources
        .iter()
        .position(|ext| ext.name == name && ext.origin == origin)
    {
        sources[index] = candidate;
    } else {
        sources.push(candidate);
    }
    let mut preview = settings;
    preview
        .personal
        .disable
        .retain(|disabled| disabled != &name);
    preview.personal.enable.push(name.clone());
    let mut extensions = super::resolve_catalog(sources, &preview);
    let target = extensions
        .iter()
        .position(|ext| ext.name == name && ext.origin == origin)
        .expect("candidate remains listed even when blocked");
    let mut regressions = Vec::new();
    for ext in extensions.iter().filter(|ext| {
        ext.name != name && previously_active.contains(&ext.name) && ext.public_contracts.blocked
    }) {
        regressions.extend(
            ext.errors
                .iter()
                .map(|error| format!("candidate `{name}` blocks consumer `{}`: {error}", ext.name)),
        );
    }
    for error in regressions {
        if !extensions[target].errors.contains(&error) {
            extensions[target].errors.push(error);
        }
    }
    let mut dependents = BTreeSet::from([name]);
    loop {
        let consumers: Vec<_> = extensions
            .iter()
            .filter(|ext| {
                ext.public_contracts
                    .imports
                    .iter()
                    .any(|import| dependents.contains(&import.contribution.extension))
            })
            .map(|ext| ext.name.clone())
            .collect();
        let before = dependents.len();
        dependents.extend(consumers);
        if before == dependents.len() {
            break;
        }
    }
    CandidateCatalog {
        extensions,
        target,
        dependents,
    }
}
