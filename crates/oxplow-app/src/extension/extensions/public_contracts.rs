//! Manifest declarations and host bindings for public extension contracts.

use std::collections::{BTreeMap, BTreeSet};

use oxplow_domain::contribution::{ContributionId, ContributionKind, ContributionRef, DeclaredAt};
use serde::{Deserialize, Serialize};

use super::{imports, manifest_v2, Extension};

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    #[default]
    Private,
    Experimental,
    Stable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportFile {
    pub contribution: String,
    #[schemars(range(min = 1))]
    pub version: u32,
    #[serde(default)]
    pub visibility: Visibility,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportFile {
    pub id: String,
    pub contribution: String,
    /// The contract version it was written for; absent, whatever the
    /// producer exports binds.
    #[serde(default)]
    #[schemars(range(min = 1))]
    pub version: Option<u32>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportGuardFile {
    pub contribution: String,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportDeclaration {
    pub contribution: ContributionId,
    pub version: u32,
    pub visibility: Visibility,
    pub source: DeclaredAt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportDeclaration {
    pub id: String,
    pub contribution: ContributionId,
    pub version: Option<u32>,
    pub optional: bool,
    pub source: DeclaredAt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportGuard {
    pub contribution: ContributionId,
    pub imports: Vec<String>,
    pub source: DeclaredAt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PublicBinding {
    pub id: String,
    pub contribution: ContributionId,
    /// The version it bound: the producer's export when available.
    pub version: Option<u32>,
    pub available: bool,
    pub unavailable_reason: Option<imports::UnavailableReason>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Withheld {
    pub contribution: ContributionId,
    /// The bindings (`imports[*].id`) whose unavailability withheld it.
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PublicContracts {
    pub exports: Vec<ExportDeclaration>,
    pub imports: Vec<ImportDeclaration>,
    pub guards: Vec<ImportGuard>,
    pub bindings: Vec<PublicBinding>,
    pub invalid: bool,
    pub blocked: bool,
    pub activation_index: Option<u32>,
    /// What this load withheld, each with the unavailable imports that
    /// withheld it (empty for a mount withheld with what it mounts from
    /// another extension).
    #[serde(default)]
    pub withheld: Vec<Withheld>,
}

pub(super) fn load(
    ext: &mut Extension,
    manifest: &manifest_v2::ManifestV2,
    text: &str,
    file: &str,
) {
    let lines: BTreeMap<_, _> = ["exports", "imports", "import_guards"]
        .into_iter()
        .map(|key| (key, manifest_v2::item_lines(text, key)))
        .collect();
    let source = |key: &str, index: usize| DeclaredAt {
        file: file.to_owned(),
        line: lines[key]
            .get(index)
            .copied()
            .or_else(|| manifest_v2::key_line(text, key))
            .and_then(|line| u32::try_from(line).ok()),
    };
    for (index, raw) in manifest.exports.iter().enumerate() {
        let at = source("exports", index);
        if let Some(contribution) = parse_id(ext, &raw.contribution, &at, true) {
            ext.public_contracts.exports.push(ExportDeclaration {
                contribution,
                version: raw.version,
                visibility: raw.visibility,
                source: at,
            });
        }
    }
    for (index, raw) in manifest.imports.iter().enumerate() {
        let at = source("imports", index);
        if let Some(contribution) = parse_id(ext, &raw.contribution, &at, false) {
            ext.public_contracts.imports.push(ImportDeclaration {
                id: raw.id.clone(),
                contribution,
                version: raw.version,
                optional: raw.optional,
                source: at,
            });
        }
    }
    for (index, raw) in manifest.import_guards.iter().enumerate() {
        let at = source("import_guards", index);
        if let Some(contribution) = parse_id(ext, &raw.contribution, &at, true) {
            ext.public_contracts.guards.push(ImportGuard {
                contribution,
                imports: raw.imports.clone(),
                source: at,
            });
        }
    }
}

/// A manifest's contribution ref (`oxplow:oxplow/dataset?dataset=effort-review/reviews`,
/// `oxplow:oxplow/command?command=acme.pr.merge`). A global name says no owner: in an export or
/// guard it is the extension's own; in an import the owner is found when
/// imports resolve ([`resolve_global_imports`]), until then its extension
/// is empty.
fn parse_id(
    ext: &mut Extension,
    text: &str,
    source: &DeclaredAt,
    own: bool,
) -> Option<ContributionId> {
    match text.parse::<oxplow_domain::contribution::ContributionRef>() {
        Ok(r) => Some(r.id().unwrap_or_else(|| {
            let owner = if own { ext.name.as_str() } else { "" };
            ContributionId::new(owner, r.kind(), r.name())
        })),
        Err(error) => {
            ext.public_contracts.invalid = true;
            ext.errors.push(manifest_v2::at(
                &source.file,
                source.line.and_then(|line| usize::try_from(line).ok()),
                format!("invalid public contribution `{text}`: {error}"),
            ));
            None
        }
    }
}

/// An import of a global name (`oxplow:oxplow/command?command=acme.pr.merge`) is of whichever
/// extension declares that name; one nothing declares stays ownerless
/// and resolves as missing.
fn resolve_global_imports(extensions: &mut [Extension]) {
    let owners: BTreeMap<ContributionRef, ContributionId> = extensions
        .iter()
        .flat_map(|ext| ext.contributions.iter())
        .filter(|d| d.id.kind.global_name())
        .map(|d| (d.id.reference(), d.id.clone()))
        .collect();
    for ext in extensions.iter_mut() {
        for import in &mut ext.public_contracts.imports {
            if import.contribution.extension.is_empty() {
                if let Some(owner) = owners.get(&import.contribution.reference()) {
                    import.contribution = owner.clone();
                }
            }
        }
    }
}

pub(super) fn prepare(ext: &mut Extension) {
    let queries: Vec<_> = ext
        .datasets
        .iter()
        .map(|dataset| {
            (
                ContributionId::new(
                    &ext.name,
                    ContributionKind::Dataset,
                    dataset
                        .twin
                        .as_ref()
                        .map_or(dataset.decl.name.as_str(), |twin| twin.of.as_str()),
                ),
                dataset.file.clone(),
                dataset.sql.clone(),
            )
        })
        .chain(ext.page_files.iter().flat_map(|page| {
            page.loaders.iter().filter_map(|decl| match &decl.loader {
                crate::pages::schema::Loader::Sql { sql, .. } => Some((
                    ContributionId::new(&ext.name, ContributionKind::Page, &page.slug),
                    page.origin.running.clone(),
                    sql.clone(),
                )),
                _ => None,
            })
        }))
        .collect();
    for (contribution, file, sql) in queries {
        if !sql.to_ascii_lowercase().contains("import_ref") {
            continue;
        }
        match oxplow_db::datasets::import_references(&sql) {
            Ok(references) => {
                for reference in references {
                    let source = DeclaredAt {
                        file: file.clone(),
                        line: u32::try_from(reference.line).ok(),
                    };
                    if ext
                        .public_contracts
                        .imports
                        .iter()
                        .find(|binding| binding.id == reference.binding)
                        .is_some_and(|binding| {
                            binding.contribution.kind != ContributionKind::Dataset
                        })
                    {
                        invalid(
                            ext,
                            &source,
                            format!(
                                "import_ref('{}') must bind a dataset contribution",
                                reference.binding
                            ),
                        );
                    }
                    ext.public_contracts.guards.push(ImportGuard {
                        contribution: contribution.clone(),
                        imports: vec![reference.binding],
                        source,
                    });
                }
            }
            Err(error) => invalid(ext, &DeclaredAt { file, line: None }, error.to_string()),
        }
    }
    for guard in ext.public_contracts.guards.clone() {
        if guard.contribution.extension != ext.name
            || !ext
                .contributions
                .iter()
                .any(|descriptor| descriptor.id == guard.contribution)
        {
            invalid(
                ext,
                &guard.source,
                format!(
                    "import guard `{}` must name this extension's declared contribution",
                    guard.contribution
                ),
            );
        }
        if !matches!(
            guard.contribution.kind,
            ContributionKind::Dataset
                | ContributionKind::Page
                | ContributionKind::Collector
                | ContributionKind::Command
                | ContributionKind::Job
                | ContributionKind::Effect
                | ContributionKind::Advisory
                | ContributionKind::Skill
                | ContributionKind::Provider
        ) {
            invalid(ext, &guard.source, format!("import guard `{}` must name a dataset, page, collector, command, job, effect, advisory, skill or provider", guard.contribution));
        }
        if guard.imports.is_empty() {
            invalid(
                ext,
                &guard.source,
                "import guard must name at least one import binding".into(),
            );
        }
        let mut seen = BTreeSet::new();
        for binding in &guard.imports {
            if !seen.insert(binding)
                || !ext
                    .public_contracts
                    .imports
                    .iter()
                    .any(|import| &import.id == binding)
            {
                invalid(
                    ext,
                    &guard.source,
                    format!("import guard binding `{binding}` is duplicate or undeclared"),
                );
            }
        }
    }
}

fn invalid(ext: &mut Extension, source: &DeclaredAt, message: String) {
    ext.public_contracts.invalid = true;
    ext.errors.push(manifest_v2::at(
        &source.file,
        source.line.and_then(|line| usize::try_from(line).ok()),
        message,
    ));
}

fn candidate(
    ext: &Extension,
    withdrawn: &BTreeMap<ContributionId, BTreeSet<String>>,
) -> imports::Candidate {
    imports::Candidate {
        name: ext.name.clone(),
        enabled: ext.enabled,
        bundled: ext.origin == "bundled",
        contributions: ext
            .contributions
            .iter()
            .map(|descriptor| imports::ContractContribution {
                id: descriptor.id.clone(),
                version: descriptor.contract_version,
            })
            .collect(),
        exports: ext
            .public_contracts
            .exports
            .iter()
            .filter(|export| !withdrawn.contains_key(&export.contribution))
            .map(|export| imports::Export {
                id: export.contribution.clone(),
                version: export.version,
                source: export.source.clone(),
                visibility: match export.visibility {
                    Visibility::Private => imports::ExportVisibility::Private,
                    Visibility::Experimental => imports::ExportVisibility::Experimental,
                    Visibility::Stable => imports::ExportVisibility::Stable,
                },
            })
            .collect(),
        imports: ext
            .public_contracts
            .imports
            .iter()
            .map(|import| imports::Import {
                binding: import.id.clone(),
                target: import.contribution.clone(),
                version: import.version,
                optional: import.optional,
                source: import.source.clone(),
            })
            .collect(),
    }
}

pub(super) fn stage(extensions: &mut [Extension]) {
    resolve(extensions, false);
}

pub(super) fn activate(extensions: &mut [Extension]) {
    resolve(extensions, true);
}

/// Records the resolution's diagnostics on their extensions. An optional
/// import's warning names what its unavailability withheld (`causes`), and
/// replaces the warning an earlier pass recorded for the same import.
fn record_diagnostics(
    extensions: &mut [Extension],
    resolution: &imports::Resolution,
    causes: &BTreeMap<ContributionId, BTreeSet<String>>,
) {
    for diagnostic in &resolution.diagnostics {
        for ext in extensions
            .iter_mut()
            .filter(|ext| ext.name == diagnostic.extension)
        {
            let file = if diagnostic.source.file == format!("{}/extension.yaml", ext.name) {
                format!("{}/extension.yaml", ext.path.trim_end_matches('/'))
            } else {
                diagnostic.source.file.clone()
            };
            let at = |text: String| {
                manifest_v2::at(
                    &file,
                    diagnostic
                        .source
                        .line
                        .and_then(|line| usize::try_from(line).ok()),
                    text,
                )
            };
            let base = format!("{}: {}", diagnostic.code, diagnostic.message);
            let withheld: Vec<_> = match (&diagnostic.binding, diagnostic.severity) {
                (Some(binding), imports::Severity::Warning) => causes
                    .iter()
                    .filter(|(id, bindings)| id.extension == ext.name && bindings.contains(binding))
                    .map(|(id, _)| format!("`{id}`"))
                    .collect(),
                _ => Vec::new(),
            };
            let message = if withheld.is_empty() {
                at(base.clone())
            } else {
                at(format!("{base}; withheld: {}", withheld.join(", ")))
            };
            let messages = match diagnostic.severity {
                imports::Severity::Error => &mut ext.errors,
                imports::Severity::Warning => &mut ext.warnings,
            };
            let earlier = at(base);
            messages.retain(|m| m != &message && !m.starts_with(&earlier));
            messages.push(message);
        }
    }
}

fn resolve(extensions: &mut [Extension], publish: bool) {
    resolve_global_imports(extensions);
    let mut blocked: BTreeSet<_> = extensions
        .iter()
        .filter(|ext| ext.public_contracts.invalid || ext.public_contracts.blocked)
        .map(|ext| ext.name.clone())
        .collect();
    // Each withheld contribution, with the unavailable imports that
    // withheld it.
    let mut causes: BTreeMap<ContributionId, BTreeSet<String>> = extensions
        .iter()
        .flat_map(|ext| {
            ext.public_contracts.withheld.iter().map(|withheld| {
                (
                    withheld.contribution.clone(),
                    withheld.imports.iter().cloned().collect(),
                )
            })
        })
        .collect();
    let resolution = loop {
        let candidates: Vec<_> = extensions
            .iter()
            .filter(|ext| {
                ext.origin != "project" || !crate::bundled_extensions::is_reserved(&ext.name)
            })
            .map(|ext| candidate(ext, &causes))
            .collect();
        let resolution = imports::validate_with_blocked(&candidates, &blocked);
        blocked.extend(resolution.blocked.iter().cloned());
        let before: BTreeSet<_> = extensions
            .iter()
            .flat_map(|ext| {
                ext.contributions
                    .iter()
                    .map(|descriptor| descriptor.id.clone())
            })
            .collect();
        for ext in extensions
            .iter_mut()
            .filter(|ext| ext.enabled && !blocked.contains(&ext.name))
        {
            let unavailable: BTreeSet<_> = resolution
                .imports
                .iter()
                .filter(|binding| {
                    binding.consumer == ext.name
                        && !matches!(binding.status, imports::ImportStatus::Available)
                })
                .map(|binding| binding.binding.as_str())
                .collect();
            let guards = ext.public_contracts.guards.clone();
            for guard in guards {
                if guard
                    .imports
                    .iter()
                    .any(|binding| unavailable.contains(binding.as_str()))
                    && ext
                        .contributions
                        .iter()
                        .any(|descriptor| descriptor.id == guard.contribution)
                {
                    remove_guarded(ext, &guard.contribution);
                    causes.insert(
                        guard.contribution.clone(),
                        guard
                            .imports
                            .iter()
                            .filter(|binding| unavailable.contains(binding.as_str()))
                            .cloned()
                            .collect(),
                    );
                }
            }
        }
        prune_mounts(extensions, &blocked);
        let after: BTreeSet<_> = extensions
            .iter()
            .flat_map(|ext| {
                ext.contributions
                    .iter()
                    .map(|descriptor| descriptor.id.clone())
            })
            .collect();
        // A mount withheld with what it mounts goes with its extension's
        // unavailable imports.
        for id in before.difference(&after) {
            if !causes.contains_key(id) {
                let bindings = causes
                    .iter()
                    .filter(|(other, _)| other.extension == id.extension)
                    .flat_map(|(_, bindings)| bindings.iter().cloned())
                    .collect();
                causes.insert(id.clone(), bindings);
            }
        }
        if before == after {
            break resolution;
        }
    };
    record_diagnostics(extensions, &resolution, &causes);
    for ext in extensions.iter_mut() {
        for (id, _) in causes
            .iter()
            .filter(|(id, bindings)| id.extension == ext.name && bindings.is_empty())
        {
            let message = manifest_v2::at(
                &format!("{}/extension.yaml", ext.path.trim_end_matches('/')),
                None,
                format!("withheld `{id}`: what it mounts from another extension was withheld"),
            );
            if !ext.warnings.contains(&message) {
                ext.warnings.push(message);
            }
        }
    }
    for ext in extensions {
        ext.public_contracts.blocked = ext.enabled && blocked.contains(&ext.name);
        ext.public_contracts.activation_index = resolution
            .activation_order
            .iter()
            .position(|name| name == &ext.name)
            .and_then(|index| u32::try_from(index).ok());
        ext.public_contracts.bindings = resolution
            .imports
            .iter()
            .filter(|binding| binding.consumer == ext.name)
            .map(|binding| {
                let available = matches!(binding.status, imports::ImportStatus::Available)
                    && !ext.public_contracts.blocked;
                PublicBinding {
                    id: binding.binding.clone(),
                    contribution: binding.target.clone(),
                    version: binding.version,
                    available,
                    unavailable_reason: if available {
                        None
                    } else {
                        Some(match &binding.status {
                            imports::ImportStatus::Available => imports::UnavailableReason::Blocked,
                            imports::ImportStatus::Unavailable { reason } => reason.clone(),
                        })
                    },
                }
            })
            .collect();
        ext.public_contracts.withheld = causes
            .iter()
            .filter(|(id, _)| id.extension == ext.name)
            .map(|(id, bindings)| Withheld {
                contribution: id.clone(),
                imports: bindings.iter().cloned().collect(),
            })
            .collect();
        if publish {
            if ext.public_contracts.blocked {
                ext.enabled = false;
            }
            if ext.enabled {
                bind_sql(ext);
            }
        }
    }
}

fn remove_guarded(ext: &mut Extension, id: &ContributionId) {
    let local = id.local.as_str();
    match id.kind {
        ContributionKind::Dataset => ext.datasets.retain(|dataset| {
            dataset.decl.name != local && dataset.twin.as_ref().is_none_or(|twin| twin.of != local)
        }),
        ContributionKind::Page => ext.page_files.retain(|page| page.slug != local),
        ContributionKind::Collector => ext.collectors.retain(|collector| collector.id != local),
        ContributionKind::Command => ext.commands.retain(|command| command.name != local),
        ContributionKind::Job => ext.jobs.retain(|job| job.id != local),
        ContributionKind::Effect => ext.effects.retain(|effect| effect.id != local),
        ContributionKind::Advisory => ext.advisories.retain(|advisory| advisory.id != local),
        ContributionKind::Skill => ext.skills.retain(|skill| skill.name != local),
        ContributionKind::Provider => ext.providers.retain(|provider| provider.id != local),
        _ => unreachable!("only validated executable contributions have import guards"),
    }
    if matches!(
        id.kind,
        ContributionKind::Effect | ContributionKind::Collector
    ) {
        let by = format!("{} `{local}`", id.kind.segment());
        ext.subscriptions
            .retain(|subscription| subscription.by != by);
    }
    ext.contributions.retain(|descriptor| &descriptor.id != id);
}

/// What a withheld contribution leaves without a part: a provider command
/// without its provider, a panel, slot mount or replacement without its
/// page, a resource or decorator without its dataset.
fn prune_mounts(extensions: &mut [Extension], blocked: &BTreeSet<String>) {
    {
        for ext in extensions
            .iter_mut()
            .filter(|ext| ext.enabled && !blocked.contains(&ext.name))
        {
            let mut removed = BTreeSet::new();
            let providers: BTreeSet<_> = ext
                .providers
                .iter()
                .map(|provider| provider.id.clone())
                .collect();
            ext.commands.retain(|command| {
                if let crate::extension::extension_commands::CommandHandler::Provider {
                    provider,
                    ..
                } = &command.handler
                {
                    if !providers.contains(provider) {
                        removed.insert(ContributionId::new(
                            &ext.name,
                            ContributionKind::Command,
                            &command.name,
                        ));
                        return false;
                    }
                }
                true
            });
            let pages: BTreeSet<String> = ext.page_files.iter().map(|p| p.id.clone()).collect();
            ext.panels.retain(|panel| {
                let present = |id: &String| pages.contains(id);
                if [&panel.body]
                    .into_iter()
                    .chain(
                        [&panel.badge, &panel.collapsed, &panel.count]
                            .into_iter()
                            .flatten(),
                    )
                    .all(present)
                {
                    true
                } else {
                    removed.insert(ContributionId::new(
                        &ext.name,
                        ContributionKind::Panel,
                        panel
                            .id
                            .strip_prefix(&format!("{}/", ext.name))
                            .unwrap_or(&panel.id),
                    ));
                    false
                }
            });
            let mut n = 0;
            ext.ui.page_slots.retain(|slot| {
                let id = slot.local_id(n);
                n += 1;
                if pages.contains(&slot.page) {
                    true
                } else {
                    removed.insert(ContributionId::new(&ext.name, ContributionKind::Slot, &id));
                    false
                }
            });
            ext.ui.replacements.retain(|replacement| {
                if pages.contains(&replacement.page) {
                    true
                } else {
                    removed.insert(ContributionId::new(
                        &ext.name,
                        ContributionKind::Replacement,
                        &replacement.target,
                    ));
                    false
                }
            });
            let datasets: BTreeSet<_> = ext
                .datasets
                .iter()
                .map(|dataset| oxplow_db::datasets::extension_view(&ext.name, &dataset.decl.name))
                .collect();
            let core = ext.namespace == super::CORE_NAMESPACE;
            ext.resources.retain(|resource| {
                let decl = &resource.decl;
                if core
                    || [&decl.resolve, &decl.searchable]
                        .into_iter()
                        .flatten()
                        .all(|view| datasets.contains(view))
                {
                    true
                } else {
                    removed.insert(ContributionId::new(
                        &ext.name,
                        ContributionKind::Resource,
                        decl.name.name(),
                    ));
                    false
                }
            });
            ext.ui.decorators.retain(|decorator| {
                if datasets.contains(&decorator.view) {
                    true
                } else {
                    removed.insert(ContributionId::new(
                        &ext.name,
                        ContributionKind::Decorator,
                        &format!(
                            "decorator-{}",
                            decorator.id.rsplit('/').next().unwrap_or_default()
                        ),
                    ));
                    false
                }
            });
            ext.contributions
                .retain(|descriptor| !removed.contains(&descriptor.id));
        }
    }
}

fn bind_sql(ext: &mut Extension) {
    let bindings = &ext.public_contracts.bindings;
    for dataset in &mut ext.datasets {
        dataset.sql = rewrite_sql(&dataset.sql, bindings, true);
    }
    for page in &mut ext.page_files {
        for decl in &mut page.loaders {
            if let crate::pages::schema::Loader::Sql { sql, .. } = &mut decl.loader {
                *sql = rewrite_sql(sql, bindings, false);
            }
        }
    }
}

fn rewrite_sql(sql: &str, bindings: &[PublicBinding], dataset: bool) -> String {
    let Ok(references) = oxplow_db::datasets::import_references(sql) else {
        return sql.to_owned();
    };
    let mut result = sql.to_owned();
    for reference in references.into_iter().rev() {
        if let Some(binding) = bindings
            .iter()
            .find(|binding| binding.id == reference.binding && binding.available)
        {
            let replacement = if dataset {
                format!(
                    "ref('{}/{}')",
                    binding.contribution.extension, binding.contribution.local
                )
            } else {
                format!(
                    "\"{}\"",
                    oxplow_db::datasets::extension_view(
                        &binding.contribution.extension,
                        &binding.contribution.local
                    )
                    .replace('"', "\"\"")
                )
            };
            result.replace_range(reference.start..reference.end, &replacement);
        }
    }
    result
}
