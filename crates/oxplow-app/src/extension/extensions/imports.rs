//! Pure validation and ordering for normalized extension contract imports.
//!
//! The loader turns manifest declarations and contribution descriptors into
//! these values. This module deliberately knows nothing about manifests,
//! activation registries, files, services, or grants.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use oxplow_domain::contribution::{valid_extension_name, ContributionId, DeclaredAt};
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportVisibility {
    #[default]
    Private,
    Experimental,
    Stable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct ContractContribution {
    pub id: ContributionId,
    pub version: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Export {
    pub id: ContributionId,
    pub version: u32,
    pub visibility: ExportVisibility,
    pub source: DeclaredAt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Import {
    pub binding: String,
    pub target: ContributionId,
    /// Absent, whatever the producer exports binds.
    pub version: Option<u32>,
    pub optional: bool,
    pub source: DeclaredAt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Candidate {
    pub name: String,
    pub enabled: bool,
    /// It ships with oxplow: it imports only stable contracts and exports
    /// nothing as experimental.
    pub bundled: bool,
    pub contributions: Vec<ContractContribution>,
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Resolution {
    pub imports: Vec<ResolvedImport>,
    pub blocked: BTreeSet<String>,
    pub activation_order: Vec<String>,
    pub diagnostics: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct ResolvedImport {
    pub consumer: String,
    pub binding: String,
    pub target: ContributionId,
    /// The version bound: the producer's export when available, else the
    /// one asked for.
    pub version: Option<u32>,
    pub status: ImportStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ImportStatus {
    Available,
    Unavailable { reason: UnavailableReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case", tag = "reason")]
#[derive(serde::Deserialize)]
pub enum UnavailableReason {
    Missing,
    Disabled,
    Blocked,
    Private,
    Experimental,
    Incompatible { actual: u32 },
    Ambiguous,
    Invalid,
}

impl UnavailableReason {
    /// Why an import of `producer`'s contribution is unavailable, in the
    /// words every report uses (the load warning, `extension check`).
    pub fn describe(&self, producer: &str) -> String {
        match self {
            Self::Missing => format!("{producer} or its contribution isn't here"),
            Self::Disabled => format!("{producer} is disabled"),
            Self::Blocked => format!("{producer} is blocked by its own imports"),
            Self::Private => format!("{producer} doesn't export it"),
            Self::Experimental => format!(
                "{producer} exports it as experimental, which an extension that ships with oxplow can't import"
            ),
            Self::Incompatible { actual } => format!("{producer} exports v{actual}"),
            Self::Ambiguous => "more than one producer has that name".to_string(),
            Self::Invalid => format!("{producer}'s export is invalid"),
        }
    }
}

pub use oxplow_domain::diagnostic::Severity;

/// A problem the resolution found, on the extension it belongs to: what
/// `public_contracts` records as that extension's diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Finding {
    pub extension: String,
    pub binding: Option<String>,
    pub source: DeclaredAt,
    pub severity: Severity,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct Edge {
    consumer: usize,
    producer: usize,
    resolution: usize,
    optional: bool,
    source: DeclaredAt,
    binding: String,
}

/// Resolve public imports and return a deterministic dependency-first order.
///
/// Only explicit exports participate. This function neither grants access nor
/// rewrites references inside SQL or scripts.
pub fn validate(catalog: &[Candidate]) -> Resolution {
    validate_with_blocked(catalog, &BTreeSet::new())
}

/// Include candidates refused by host manifest checks before resolving their
/// consumers. Their metadata remains available for actionable diagnostics.
pub fn validate_with_blocked(
    catalog: &[Candidate],
    initially_blocked: &BTreeSet<String>,
) -> Resolution {
    let mut resolution = Resolution {
        imports: Vec::new(),
        blocked: BTreeSet::new(),
        activation_order: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut blocked_indices = catalog
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            (candidate.enabled && initially_blocked.contains(&candidate.name)).then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let root_source = |candidate: &Candidate| DeclaredAt {
        file: format!("{}/extension.yaml", candidate.name),
        line: None,
    };

    let mut by_name = BTreeMap::<String, Vec<usize>>::new();
    for (index, candidate) in catalog.iter().enumerate() {
        by_name
            .entry(candidate.name.clone())
            .or_default()
            .push(index);
    }
    for (name, indices) in &by_name {
        if indices.len() > 1 {
            for &index in indices {
                let candidate = &catalog[index];
                diagnostic(
                    &mut resolution,
                    candidate,
                    None,
                    root_source(candidate),
                    Severity::Error,
                    "duplicate_extension",
                    format!("extension `{name}` appears more than once in the candidate catalog"),
                );
                if candidate.enabled {
                    blocked_indices.insert(index);
                }
            }
        }
    }

    let mut contributions = Vec::<BTreeMap<ContributionId, Vec<Option<u32>>>>::new();
    let mut export_positions = BTreeMap::<ContributionId, Vec<(usize, usize)>>::new();
    let mut export_valid = catalog
        .iter()
        .map(|candidate| vec![true; candidate.exports.len()])
        .collect::<Vec<_>>();

    for (candidate_index, candidate) in catalog.iter().enumerate() {
        let source = root_source(candidate);
        if !valid_extension_name(&candidate.name) {
            diagnostic(
                &mut resolution,
                candidate,
                None,
                source.clone(),
                Severity::Error,
                "invalid_extension_name",
                format!("`{}` is not a valid extension name", candidate.name),
            );
            if candidate.enabled {
                blocked_indices.insert(candidate_index);
            }
        }

        let mut declared = BTreeMap::<ContributionId, Vec<Option<u32>>>::new();
        for contribution in &candidate.contributions {
            let mut invalid = false;
            if let Some(problem) = contribution.id.problem() {
                diagnostic(
                    &mut resolution,
                    candidate,
                    None,
                    source.clone(),
                    Severity::Error,
                    "invalid_contribution_id",
                    format!("contribution `{}` is invalid: {problem}", contribution.id),
                );
                invalid = true;
            }
            if contribution.id.extension != candidate.name {
                diagnostic(
                    &mut resolution,
                    candidate,
                    None,
                    source.clone(),
                    Severity::Error,
                    "contribution_owner_mismatch",
                    format!(
                        "contribution `{}` is owned by `{}`, not candidate `{}`",
                        contribution.id, contribution.id.extension, candidate.name
                    ),
                );
                invalid = true;
            }
            if contribution.version == Some(0) {
                diagnostic(
                    &mut resolution,
                    candidate,
                    None,
                    source.clone(),
                    Severity::Error,
                    "invalid_contract_version",
                    format!(
                        "contribution `{}` has contract version zero",
                        contribution.id
                    ),
                );
                invalid = true;
            }
            declared
                .entry(contribution.id.clone())
                .or_default()
                .push(contribution.version);
            if invalid && candidate.enabled {
                blocked_indices.insert(candidate_index);
            }
        }
        for (id, versions) in &declared {
            if versions.len() > 1 {
                diagnostic(
                    &mut resolution,
                    candidate,
                    None,
                    source.clone(),
                    Severity::Error,
                    "duplicate_contribution",
                    format!("contribution `{id}` is declared more than once"),
                );
                if candidate.enabled {
                    blocked_indices.insert(candidate_index);
                }
            }
        }
        contributions.push(declared.clone());

        for (export_index, export) in candidate.exports.iter().enumerate() {
            export_positions
                .entry(export.id.clone())
                .or_default()
                .push((candidate_index, export_index));
            let mut invalid = false;
            if let Some(problem) = export.id.problem() {
                invalid_export(
                    &mut resolution,
                    candidate,
                    export,
                    "invalid_export_id",
                    format!("export identity `{}` is invalid: {problem}", export.id),
                );
                invalid = true;
            }
            if export.id.extension != candidate.name {
                invalid_export(
                    &mut resolution,
                    candidate,
                    export,
                    "export_owner_mismatch",
                    format!(
                        "export `{}` is owned by `{}`, not candidate `{}`",
                        export.id, export.id.extension, candidate.name
                    ),
                );
                invalid = true;
            }
            if export.version == 0 {
                invalid_export(
                    &mut resolution,
                    candidate,
                    export,
                    "invalid_contract_version",
                    format!("export `{}` has contract version zero", export.id),
                );
                invalid = true;
            }
            match declared.get(&export.id) {
                None => {
                    invalid_export(
                        &mut resolution,
                        candidate,
                        export,
                        "export_not_declared",
                        format!(
                            "export `{}` does not name a declared contribution",
                            export.id
                        ),
                    );
                    invalid = true;
                }
                Some(versions) if versions.len() != 1 => {
                    invalid_export(
                        &mut resolution,
                        candidate,
                        export,
                        "export_ambiguous_contribution",
                        format!("export `{}` names a duplicate contribution", export.id),
                    );
                    invalid = true;
                }
                Some(versions) => {
                    if let Some(Some(actual)) = versions.first() {
                        if export.version != *actual {
                            invalid_export(
                                &mut resolution,
                                candidate,
                                export,
                                "export_version_mismatch",
                                format!(
                                    "export `{}` declares version {}, but its contribution defines version {actual}",
                                    export.id, export.version
                                ),
                            );
                            invalid = true;
                        }
                    }
                }
            }
            if candidate.bundled && export.visibility == ExportVisibility::Experimental {
                invalid_export(
                    &mut resolution,
                    candidate,
                    export,
                    "bundled_experimental_export",
                    format!(
                        "bundled extension `{}` cannot export `{}` as experimental",
                        candidate.name, export.id
                    ),
                );
                invalid = true;
            }
            if invalid {
                export_valid[candidate_index][export_index] = false;
                if candidate.enabled {
                    blocked_indices.insert(candidate_index);
                }
            }
        }
    }

    for (id, positions) in &export_positions {
        if positions.len() > 1 {
            for &(candidate_index, export_index) in positions {
                let candidate = &catalog[candidate_index];
                let export = &candidate.exports[export_index];
                invalid_export(
                    &mut resolution,
                    candidate,
                    export,
                    "duplicate_export",
                    format!("export identity `{id}` is declared more than once"),
                );
                export_valid[candidate_index][export_index] = false;
                if candidate.enabled {
                    blocked_indices.insert(candidate_index);
                }
            }
        }
    }

    let mut edges = Vec::<Edge>::new();
    for (candidate_index, candidate) in catalog.iter().enumerate() {
        let mut bindings = BTreeMap::<&str, Vec<usize>>::new();
        for (import_index, import) in candidate.imports.iter().enumerate() {
            bindings
                .entry(import.binding.as_str())
                .or_default()
                .push(import_index);
        }
        let mut import_invalid = vec![false; candidate.imports.len()];
        for (binding, indices) in &bindings {
            if indices.len() > 1 {
                for &index in indices {
                    let import = &candidate.imports[index];
                    diagnostic(
                        &mut resolution,
                        candidate,
                        Some(import.binding.clone()),
                        import.source.clone(),
                        Severity::Error,
                        "duplicate_import_binding",
                        format!("import binding `{binding}` is declared more than once"),
                    );
                    import_invalid[index] = true;
                }
                if candidate.enabled {
                    blocked_indices.insert(candidate_index);
                }
            }
        }

        for (import_index, import) in candidate.imports.iter().enumerate() {
            let mut invalid = import_invalid[import_index];
            if let Some(problem) = import.target.problem() {
                diagnostic(
                    &mut resolution,
                    candidate,
                    Some(import.binding.clone()),
                    import.source.clone(),
                    Severity::Error,
                    "invalid_import_target",
                    format!(
                        "import `{}` has an invalid target: {problem}",
                        import.binding
                    ),
                );
                invalid = true;
            }
            if !valid_binding(&import.binding) {
                diagnostic(
                    &mut resolution,
                    candidate,
                    Some(import.binding.clone()),
                    import.source.clone(),
                    Severity::Error,
                    "invalid_import_binding",
                    format!("binding `{}` must start with a lowercase letter and contain only lowercase letters, digits, dashes or underscores", import.binding),
                );
                invalid = true;
            }
            if import.version == Some(0) {
                diagnostic(
                    &mut resolution,
                    candidate,
                    Some(import.binding.clone()),
                    import.source.clone(),
                    Severity::Error,
                    "invalid_contract_version",
                    format!("import `{}` requests contract version zero", import.binding),
                );
                invalid = true;
            }
            if import.target.extension == candidate.name {
                diagnostic(
                    &mut resolution,
                    candidate,
                    Some(import.binding.clone()),
                    import.source.clone(),
                    Severity::Error,
                    "self_import",
                    format!(
                        "import `{}` cannot target its own extension",
                        import.binding
                    ),
                );
                invalid = true;
            }
            if invalid {
                if candidate.enabled {
                    blocked_indices.insert(candidate_index);
                }
                resolution.imports.push(ResolvedImport {
                    consumer: candidate.name.clone(),
                    binding: import.binding.clone(),
                    target: import.target.clone(),
                    version: import.version,
                    status: ImportStatus::Unavailable {
                        reason: UnavailableReason::Invalid,
                    },
                });
                continue;
            }
            // Disabled consumers have no active dependencies. Keep no row for
            // valid declarations; structural errors above are still reported.
            if !candidate.enabled {
                continue;
            }

            let (status, producer) = resolve_import(
                candidate_index,
                import,
                catalog,
                &by_name,
                &contributions,
                &export_positions,
                &export_valid,
            );
            let resolution_index = resolution.imports.len();
            if let ImportStatus::Unavailable { reason } = &status {
                let invalid_contract = matches!(
                    reason,
                    UnavailableReason::Invalid | UnavailableReason::Ambiguous
                );
                let (severity, code) = if invalid_contract {
                    (Severity::Error, "invalid_import_contract")
                } else if import.optional {
                    (Severity::Warning, "optional_import_unavailable")
                } else {
                    (Severity::Error, "required_import_unavailable")
                };
                diagnostic(
                    &mut resolution,
                    candidate,
                    Some(import.binding.clone()),
                    import.source.clone(),
                    severity,
                    code,
                    format!(
                        "{} import `{}` for `{}` {} is unavailable: {}",
                        if import.optional && !invalid_contract {
                            "optional"
                        } else {
                            "required"
                        },
                        import.binding,
                        import.target,
                        import
                            .version
                            .map_or("at any version".to_string(), |v| format!("v{v}")),
                        reason.describe(import.target.producer_label())
                    ),
                );
                if !import.optional || invalid_contract {
                    blocked_indices.insert(candidate_index);
                }
            }
            let bound = match status {
                ImportStatus::Available => export_positions
                    .get(&import.target)
                    .and_then(|e| e.first())
                    .map(|&(owner, index)| catalog[owner].exports[index].version),
                _ => import.version,
            };
            resolution.imports.push(ResolvedImport {
                consumer: candidate.name.clone(),
                binding: import.binding.clone(),
                target: import.target.clone(),
                version: bound,
                status: status.clone(),
            });
            if let Some(producer) = producer {
                if matches!(status, ImportStatus::Available) {
                    edges.push(Edge {
                        consumer: candidate_index,
                        producer,
                        resolution: resolution_index,
                        optional: import.optional,
                        source: import.source.clone(),
                        binding: import.binding.clone(),
                    });
                }
            }
        }
    }

    // Required imports to invalid, disabled, or missing contracts may have
    // blocked nodes above. Propagate those failures before cycle analysis.
    cascade_blocked(catalog, &edges, &mut resolution, &mut blocked_indices);
    let eligible = catalog
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            (candidate.enabled && !blocked_indices.contains(&index)).then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let graph = dependency_graph(&edges, &eligible, &resolution);
    let mut cyclic_edges = Vec::new();
    let mut cyclic_nodes = BTreeSet::new();
    for edge in &edges {
        if !eligible.contains(&edge.consumer) || !eligible.contains(&edge.producer) {
            continue;
        }
        let Some(path) = find_path(edge.producer, edge.consumer, &graph, catalog) else {
            continue;
        };
        let mut cycle = vec![catalog[edge.consumer].name.clone()];
        cycle.extend(path.into_iter().map(|node| catalog[node].name.clone()));
        cyclic_nodes.insert(edge.consumer);
        cyclic_nodes.insert(edge.producer);
        cyclic_edges.push((edge.clone(), cycle.join(" -> ")));
    }
    for index in &cyclic_nodes {
        blocked_indices.insert(*index);
    }
    for (edge, path) in cyclic_edges {
        set_unavailable(&mut resolution, edge.resolution, UnavailableReason::Blocked);
        diagnostic(
            &mut resolution,
            &catalog[edge.consumer],
            Some(edge.binding),
            edge.source,
            Severity::Error,
            "dependency_cycle",
            format!("extension dependency cycle: {path}"),
        );
    }
    cascade_blocked(catalog, &edges, &mut resolution, &mut blocked_indices);

    resolution.blocked = blocked_indices
        .iter()
        .filter(|&&index| catalog[index].enabled)
        .map(|&index| catalog[index].name.clone())
        .collect();
    resolution.activation_order = activation_order(catalog, &edges, &resolution, &blocked_indices);
    resolution.imports.sort_by(|a, b| {
        (
            &a.consumer,
            &a.binding,
            &a.target,
            a.version,
            format!("{:?}", a.status),
        )
            .cmp(&(
                &b.consumer,
                &b.binding,
                &b.target,
                b.version,
                format!("{:?}", b.status),
            ))
    });
    resolution.diagnostics.sort_by(|a, b| {
        (
            &a.extension,
            &a.source.file,
            a.source.line,
            &a.binding,
            &a.code,
            &a.message,
            a.severity,
        )
            .cmp(&(
                &b.extension,
                &b.source.file,
                b.source.line,
                &b.binding,
                &b.code,
                &b.message,
                b.severity,
            ))
    });
    resolution
}

fn resolve_import(
    consumer_index: usize,
    import: &Import,
    catalog: &[Candidate],
    by_name: &BTreeMap<String, Vec<usize>>,
    contributions: &[BTreeMap<ContributionId, Vec<Option<u32>>>],
    export_positions: &BTreeMap<ContributionId, Vec<(usize, usize)>>,
    export_valid: &[Vec<bool>],
) -> (ImportStatus, Option<usize>) {
    let Some(producers) = by_name.get(&import.target.extension) else {
        return (unavailable(UnavailableReason::Missing), None);
    };
    if producers.len() != 1 {
        return (unavailable(UnavailableReason::Ambiguous), None);
    }
    let producer_index = producers[0];
    let producer = &catalog[producer_index];
    if !producer.enabled {
        return (
            unavailable(UnavailableReason::Disabled),
            Some(producer_index),
        );
    }
    if !contributions[producer_index].contains_key(&import.target) {
        return (
            unavailable(UnavailableReason::Missing),
            Some(producer_index),
        );
    }
    if contributions[producer_index][&import.target].len() != 1 {
        return (
            unavailable(UnavailableReason::Invalid),
            Some(producer_index),
        );
    }
    let Some(exports) = export_positions.get(&import.target) else {
        return (
            unavailable(UnavailableReason::Private),
            Some(producer_index),
        );
    };
    if exports.len() != 1 {
        return (
            unavailable(UnavailableReason::Ambiguous),
            Some(producer_index),
        );
    }
    let (export_owner, export_index) = exports[0];
    if !export_valid[export_owner][export_index] {
        return (
            unavailable(UnavailableReason::Invalid),
            Some(producer_index),
        );
    }
    let export = &catalog[export_owner].exports[export_index];
    match export.visibility {
        ExportVisibility::Private => {
            return (
                unavailable(UnavailableReason::Private),
                Some(producer_index),
            );
        }
        ExportVisibility::Experimental if catalog[consumer_index].bundled => {
            return (
                unavailable(UnavailableReason::Experimental),
                Some(producer_index),
            );
        }
        ExportVisibility::Experimental | ExportVisibility::Stable => {}
    }
    if import.version.is_some_and(|v| v != export.version) {
        return (
            unavailable(UnavailableReason::Incompatible {
                actual: export.version,
            }),
            Some(producer_index),
        );
    }
    (ImportStatus::Available, Some(producer_index))
}

fn cascade_blocked(
    catalog: &[Candidate],
    edges: &[Edge],
    resolution: &mut Resolution,
    blocked: &mut BTreeSet<usize>,
) {
    loop {
        let mut changed = false;
        for edge in edges {
            if !catalog[edge.consumer].enabled
                || !blocked.contains(&edge.producer)
                || matches!(
                    resolution.imports[edge.resolution].status,
                    ImportStatus::Unavailable { .. }
                )
            {
                continue;
            }
            set_unavailable(resolution, edge.resolution, UnavailableReason::Blocked);
            let consumer = &catalog[edge.consumer];
            if edge.optional {
                diagnostic(
                    resolution,
                    consumer,
                    Some(edge.binding.clone()),
                    edge.source.clone(),
                    Severity::Warning,
                    "optional_import_blocked",
                    format!(
                        "optional import `{}` is unavailable because `{}` is blocked",
                        edge.binding, catalog[edge.producer].name
                    ),
                );
            } else {
                diagnostic(
                    resolution,
                    consumer,
                    Some(edge.binding.clone()),
                    edge.source.clone(),
                    Severity::Error,
                    "required_import_blocked",
                    format!(
                        "required import `{}` cannot activate because `{}` is blocked",
                        edge.binding, catalog[edge.producer].name
                    ),
                );
                changed |= blocked.insert(edge.consumer);
            }
        }
        if !changed {
            break;
        }
    }
}

fn dependency_graph(
    edges: &[Edge],
    eligible: &BTreeSet<usize>,
    resolution: &Resolution,
) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut graph = eligible
        .iter()
        .map(|&index| (index, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in edges {
        if eligible.contains(&edge.consumer)
            && eligible.contains(&edge.producer)
            && matches!(
                resolution.imports[edge.resolution].status,
                ImportStatus::Available
            )
        {
            graph
                .entry(edge.consumer)
                .or_default()
                .insert(edge.producer);
        }
    }
    graph
}

fn find_path(
    start: usize,
    goal: usize,
    graph: &BTreeMap<usize, BTreeSet<usize>>,
    catalog: &[Candidate],
) -> Option<Vec<usize>> {
    let mut queue = VecDeque::from([start]);
    let mut previous = BTreeMap::<usize, usize>::new();
    let mut seen = BTreeSet::from([start]);
    while let Some(node) = queue.pop_front() {
        if node == goal {
            let mut path = vec![goal];
            let mut current = goal;
            while current != start {
                current = *previous.get(&current)?;
                path.push(current);
            }
            path.reverse();
            return Some(path);
        }
        let mut next = graph
            .get(&node)
            .into_iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        next.sort_by(|a, b| catalog[*a].name.cmp(&catalog[*b].name));
        for neighbor in next {
            if seen.insert(neighbor) {
                previous.insert(neighbor, node);
                queue.push_back(neighbor);
            }
        }
    }
    None
}

fn activation_order(
    catalog: &[Candidate],
    edges: &[Edge],
    resolution: &Resolution,
    blocked: &BTreeSet<usize>,
) -> Vec<String> {
    let eligible = catalog
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            (candidate.enabled && !blocked.contains(&index)).then_some(index)
        })
        .collect::<BTreeSet<_>>();
    let mut dependencies = eligible
        .iter()
        .map(|&index| (index, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut consumers = BTreeMap::<usize, BTreeSet<usize>>::new();
    for edge in edges {
        if !eligible.contains(&edge.consumer)
            || !eligible.contains(&edge.producer)
            || !matches!(
                resolution.imports[edge.resolution].status,
                ImportStatus::Available
            )
        {
            continue;
        }
        dependencies
            .entry(edge.consumer)
            .or_default()
            .insert(edge.producer);
        consumers
            .entry(edge.producer)
            .or_default()
            .insert(edge.consumer);
    }
    let mut ready = eligible
        .iter()
        .filter(|index| dependencies[index].is_empty())
        .map(|&index| (catalog[index].name.clone(), index))
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(eligible.len());
    while let Some((name, node)) = ready.pop_first() {
        order.push(name);
        for consumer in consumers.get(&node).into_iter().flatten() {
            let deps = dependencies.get_mut(consumer).expect("eligible consumer");
            deps.remove(&node);
            if deps.is_empty() {
                ready.insert((catalog[*consumer].name.clone(), *consumer));
            }
        }
    }
    order
}

fn invalid_export(
    resolution: &mut Resolution,
    candidate: &Candidate,
    export: &Export,
    code: &str,
    message: String,
) {
    diagnostic(
        resolution,
        candidate,
        None,
        export.source.clone(),
        Severity::Error,
        code,
        message,
    );
}

fn diagnostic(
    resolution: &mut Resolution,
    candidate: &Candidate,
    binding: Option<String>,
    source: DeclaredAt,
    severity: Severity,
    code: &str,
    message: String,
) {
    resolution.diagnostics.push(Finding {
        extension: candidate.name.clone(),
        binding,
        source,
        severity,
        code: code.to_string(),
        message,
    });
}

fn unavailable(reason: UnavailableReason) -> ImportStatus {
    ImportStatus::Unavailable { reason }
}

fn set_unavailable(resolution: &mut Resolution, index: usize, reason: UnavailableReason) {
    resolution.imports[index].status = unavailable(reason);
}

fn valid_binding(binding: &str) -> bool {
    let mut chars = binding.chars();
    matches!(chars.next(), Some('a'..='z'))
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_'))
        && !binding.ends_with(['-', '_'])
}
