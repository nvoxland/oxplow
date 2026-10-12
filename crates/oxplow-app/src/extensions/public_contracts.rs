//! Manifest declarations and host bindings for public extension contracts.

use std::collections::{BTreeMap, BTreeSet};

use oxplow_domain::contribution::{ContributionId, ContributionKind, DeclaredAt};
use serde::{Deserialize, Serialize};

use super::{imports, manifest_v2, Extension, Sharing};

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
    #[schemars(range(min = 1))]
    pub version: u32,
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
    pub version: u32,
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
    pub version: u32,
    pub available: bool,
    pub unavailable_reason: Option<imports::UnavailableReason>,
    /// Published references of the resolved contribution; never lookup keys.
    pub references: Vec<String>,
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
    #[serde(default)]
    pub withheld: BTreeSet<ContributionId>,
    #[serde(skip)]
    slots: Vec<(ContributionId, super::LensSlot)>,
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
        if let Some(contribution) = parse_id(ext, &raw.contribution, &at) {
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
        if let Some(contribution) = parse_id(ext, &raw.contribution, &at) {
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
        if let Some(contribution) = parse_id(ext, &raw.contribution, &at) {
            ext.public_contracts.guards.push(ImportGuard {
                contribution,
                imports: raw.imports.clone(),
                source: at,
            });
        }
    }
}

fn parse_id(ext: &mut Extension, text: &str, source: &DeclaredAt) -> Option<ContributionId> {
    match text.parse() {
        Ok(id) => Some(id),
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

pub(super) fn prepare(ext: &mut Extension) {
    ext.public_contracts.slots = ext
        .ui
        .slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            (
                ContributionId::new(
                    &ext.name,
                    ContributionKind::Slot,
                    &format!("{}-{index}", slot.slot),
                ),
                slot.clone(),
            )
        })
        .collect();
    let queries: Vec<_> = ext
        .models
        .iter()
        .map(|model| {
            (
                ContributionId::new(
                    &ext.name,
                    ContributionKind::Model,
                    model
                        .twin
                        .as_ref()
                        .map_or(model.decl.name.as_str(), |twin| twin.of.as_str()),
                ),
                model.file.clone(),
                model.sql.clone(),
            )
        })
        .chain(ext.lenses.iter().map(|lens| {
            (
                ContributionId::new(&ext.name, ContributionKind::Lens, &lens.slug),
                lens.path.clone(),
                lens.query.clone(),
            )
        }))
        .collect();
    for (contribution, file, sql) in queries {
        if !sql.to_ascii_lowercase().contains("import_ref") {
            continue;
        }
        match oxplow_db::models::import_references(&sql) {
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
                        .is_some_and(|binding| binding.contribution.kind != ContributionKind::Model)
                    {
                        invalid(
                            ext,
                            &source,
                            format!(
                                "import_ref('{}') must bind a model contribution",
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
            ContributionKind::Model
                | ContributionKind::Lens
                | ContributionKind::Collector
                | ContributionKind::Command
                | ContributionKind::Job
                | ContributionKind::Effect
                | ContributionKind::Advisory
                | ContributionKind::Skill
                | ContributionKind::Provider
        ) {
            invalid(ext, &guard.source, format!("import guard `{}` must name a model, lens, collector, command, job, effect, advisory, skill or provider", guard.contribution));
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

fn candidate(ext: &Extension, withdrawn: &BTreeSet<ContributionId>) -> imports::Candidate {
    imports::Candidate {
        name: ext.name.clone(),
        enabled: ext.enabled,
        shared: ext.sharing == Sharing::Shared,
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
            .filter(|export| !withdrawn.contains(&export.contribution))
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

fn record_diagnostics(extensions: &mut [Extension], resolution: &imports::Resolution) {
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
            let message = manifest_v2::at(
                &file,
                diagnostic
                    .source
                    .line
                    .and_then(|line| usize::try_from(line).ok()),
                format!("{}: {}", diagnostic.code, diagnostic.message),
            );
            let messages = match diagnostic.severity {
                imports::Severity::Error => &mut ext.errors,
                imports::Severity::Warning => &mut ext.warnings,
            };
            if !messages.contains(&message) {
                messages.push(message);
            }
        }
    }
}

fn resolve(extensions: &mut [Extension], publish: bool) {
    let mut blocked: BTreeSet<_> = extensions
        .iter()
        .filter(|ext| ext.public_contracts.invalid || ext.public_contracts.blocked)
        .map(|ext| ext.name.clone())
        .collect();
    let mut withdrawn: BTreeSet<_> = extensions
        .iter()
        .flat_map(|ext| ext.public_contracts.withheld.iter().cloned())
        .collect();
    let slots: BTreeMap<_, _> = extensions
        .iter()
        .map(|ext| (ext.name.clone(), ext.public_contracts.slots.clone()))
        .collect();
    let resolution = loop {
        let candidates: Vec<_> = extensions
            .iter()
            .filter(|ext| {
                ext.origin != "project" || !crate::bundled_extensions::is_reserved(&ext.name)
            })
            .map(|ext| candidate(ext, &withdrawn))
            .collect();
        let resolution = imports::validate_with_blocked(&candidates, &blocked);
        blocked.extend(resolution.blocked.iter().cloned());
        record_diagnostics(extensions, &resolution);
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
                    let message = manifest_v2::at(
                        &guard.source.file,
                        guard
                            .source
                            .line
                            .and_then(|line| usize::try_from(line).ok()),
                        format!(
                            "withheld `{}` because an import binding is unavailable",
                            guard.contribution
                        ),
                    );
                    if !ext.warnings.contains(&message) {
                        ext.warnings.push(message);
                    }
                }
            }
        }
        prune_mounts(extensions, &blocked, &slots);
        let after: BTreeSet<_> = extensions
            .iter()
            .flat_map(|ext| {
                ext.contributions
                    .iter()
                    .map(|descriptor| descriptor.id.clone())
            })
            .collect();
        withdrawn.extend(before.difference(&after).cloned());
        if before == after {
            break resolution;
        }
    };
    let descriptors: BTreeMap<_, _> = extensions
        .iter()
        .filter(|ext| ext.enabled && !blocked.contains(&ext.name))
        .flat_map(|ext| {
            ext.contributions
                .iter()
                .map(|descriptor| (descriptor.id.clone(), descriptor.aliases.clone()))
        })
        .collect();
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
                    references: if available {
                        descriptors
                            .get(&binding.target)
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect();
        ext.public_contracts.withheld = withdrawn
            .iter()
            .filter(|id| id.extension == ext.name)
            .cloned()
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
        ContributionKind::Model => ext.models.retain(|model| {
            model.decl.name != local && model.twin.as_ref().is_none_or(|twin| twin.of != local)
        }),
        ContributionKind::Lens => ext.lenses.retain(|lens| lens.slug != local),
        ContributionKind::Collector => ext.collectors.retain(|collector| collector.id != local),
        ContributionKind::Command => {
            let name = format!("{}.{local}", ext.namespace);
            ext.commands.retain(|command| command.name != name);
        }
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

fn prune_mounts(
    extensions: &mut [Extension],
    blocked: &BTreeSet<String>,
    slots: &BTreeMap<String, Vec<(ContributionId, super::LensSlot)>>,
) {
    loop {
        let lenses: BTreeSet<_> = extensions
            .iter()
            .filter(|ext| ext.enabled && !blocked.contains(&ext.name))
            .flat_map(|ext| ext.lenses.iter().map(|lens| lens.id.clone()))
            .collect();
        let mut changed = false;
        for ext in extensions
            .iter_mut()
            .filter(|ext| ext.enabled && !blocked.contains(&ext.name))
        {
            let grids: Vec<_> = ext
                .lenses
                .iter()
                .filter(|lens| lens.children.iter().any(|child| !lenses.contains(child)))
                .map(|lens| ContributionId::new(&ext.name, ContributionKind::Lens, &lens.slug))
                .collect();
            changed |= !grids.is_empty();
            for grid in grids {
                remove_guarded(ext, &grid);
            }
            let mut removed = BTreeSet::new();
            let providers: BTreeSet<_> = ext
                .providers
                .iter()
                .map(|provider| provider.id.clone())
                .collect();
            ext.commands.retain(|command| {
                if let crate::extension_commands::CommandHandler::Provider { provider, .. } =
                    &command.handler
                {
                    if !providers.contains(provider) {
                        removed.insert(ContributionId::new(
                            &ext.name,
                            ContributionKind::Command,
                            command
                                .name
                                .strip_prefix(&format!("{}.", ext.namespace))
                                .unwrap_or(&command.name),
                        ));
                        return false;
                    }
                }
                true
            });
            ext.pages.retain(|page| {
                if lenses.contains(&page.lens) {
                    true
                } else {
                    removed.insert(ContributionId::new(
                        &ext.name,
                        ContributionKind::Page,
                        &page.id,
                    ));
                    false
                }
            });
            ext.panels.retain(|panel| {
                if [&panel.body]
                    .into_iter()
                    .chain(
                        [&panel.badge, &panel.collapsed, &panel.count]
                            .into_iter()
                            .flatten(),
                    )
                    .all(|lens| lenses.contains(lens))
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
            for (id, slot) in slots.get(&ext.name).into_iter().flatten() {
                if !lenses.contains(&slot.lens_id) {
                    ext.ui.slots.retain(|current| current != slot);
                    removed.insert(id.clone());
                }
            }
            ext.ui.replacements.retain(|replacement| {
                if lenses.contains(&replacement.lens_id) {
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
            let models: BTreeSet<_> = ext
                .models
                .iter()
                .map(|model| oxplow_db::models::extension_view(&ext.name, &model.decl.name))
                .collect();
            let pages: BTreeSet<_> = ext.pages.iter().map(|page| page.page_ref.clone()).collect();
            ext.ref_kinds.retain(|kind| {
                if models.contains(&kind.resolve)
                    && pages.contains(&kind.page)
                    && kind
                        .searchable
                        .as_ref()
                        .is_none_or(|view| models.contains(view))
                {
                    true
                } else {
                    removed.insert(ContributionId::new(
                        &ext.name,
                        ContributionKind::RefKind,
                        &kind.kind,
                    ));
                    false
                }
            });
            ext.ui.decorators.retain(|decorator| {
                if models.contains(&decorator.view) {
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
        if !changed {
            break;
        }
    }
}

fn bind_sql(ext: &mut Extension) {
    let bindings = &ext.public_contracts.bindings;
    for model in &mut ext.models {
        model.sql = rewrite_sql(&model.sql, bindings, true);
    }
    for lens in &mut ext.lenses {
        lens.query = rewrite_sql(&lens.query, bindings, false);
    }
}

fn rewrite_sql(sql: &str, bindings: &[PublicBinding], model: bool) -> String {
    let Ok(references) = oxplow_db::models::import_references(sql) else {
        return sql.to_owned();
    };
    let mut result = sql.to_owned();
    for reference in references.into_iter().rev() {
        if let Some(binding) = bindings
            .iter()
            .find(|binding| binding.id == reference.binding && binding.available)
        {
            let replacement = if model {
                format!(
                    "ref('{}/{}')",
                    binding.contribution.extension, binding.contribution.local
                )
            } else {
                format!(
                    "\"{}\"",
                    oxplow_db::models::extension_view(
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
