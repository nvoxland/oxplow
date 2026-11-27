//! The database portion of an extension generation candidate. Runtime registry
//! activation and compatible reader pinning belong to the activation owner.

use oxplow_db::datasets::{self, ExtensionDatasets};
use oxplow_db::TxError;
use oxplow_domain::diagnostic::Diagnostic;
use oxplow_domain::{vocabulary::Vocabulary, DomainError, Timestamp};

use crate::capabilities::PreparedCapabilities;
use crate::extension::extension_catalog::CatalogSnapshot;
use crate::vocabulary_reactor::{PreparedVocabulary, VocabularyErrors};

/// Database declarations copied from one catalog snapshot. No external code
/// runs during preparation, and nothing is activated by constructing this.
pub struct PreparedExtensionDatabase {
    /// Declared entities, created empty when absent so datasets over them
    /// compile before their collectors first run.
    entities: Vec<oxplow_db::collector_store::EntityTable>,
    /// The enabled extensions' shared models (`.context/models.md`): their
    /// tables made before the datasets that read them compile.
    models: crate::models::ModelRegistry,
    datasets: Vec<ExtensionDatasets>,
    vocabulary: PreparedVocabulary,
    contributions: Vec<oxplow_domain::contribution::Descriptor>,
    /// The enabled extensions' pages, every loaded extension, what a
    /// person can ask and what they can configure, listed.
    listing: oxplow_db::catalog_listing_store::CatalogListing,
    capabilities: Option<PreparedCapabilities>,
    /// The enabled extensions' `file_kinds:` as `v_file_kind_rule` rows:
    /// `(kind, glob, label, extension)`.
    file_kinds: Vec<(String, String, Option<String>, String)>,
    /// Every language every loaded extension declares, on or off
    /// (`v_language`).
    languages: Vec<oxplow_db::model_store::LanguageDeclRow>,
    /// The declarations of the extensions whose datasets read their own
    /// configuration (`config()`): their effective values are resolved in the
    /// activation transaction and compiled into the views.
    /// Each is `(extension, namespace, declaration)`: settings are stored
    /// under the namespace.
    configurations: Vec<(
        String,
        String,
        crate::extension::extensions::config_schema::ConfigurationFile,
    )>,
}

/// The effective configuration the active generation was made with, per
/// extension that reads its own ([`configuration_readers`]); a change in it
/// reactivates.
pub type ConfigSignature = std::collections::BTreeMap<String, String>;

/// A kind of contribution whose active form depends on the extension's
/// effective configuration: a change of its settings reactivates the
/// generation (`ExtensionActivation::request_config_refresh`), which
/// recompiles the datasets and rewrites the agents' skills
/// (`capabilities::refresh_agent_text`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigReader {
    /// A dataset's SQL calls `config()`: compiled with the values.
    Datasets,
    /// A skill is offered by a setting (`when:`).
    Skills,
    /// An implementation is offered by a setting (`when:`): the capability
    /// rows and the running adapters follow it.
    Implementations,
}

/// What of `extension` reads its own configuration into what is
/// activated: the one list of configuration readers. A new contribution
/// kind whose active form depends on a setting adds itself here. An
/// agent context is not one: its `when:` and `config()` are read at each
/// delivery, so a write needs no reactivation.
pub fn configuration_readers(
    extension: &crate::extension::extensions::Extension,
) -> Vec<ConfigReader> {
    if extension.configuration.is_none() {
        return Vec::new();
    }
    let datasets = extension
        .datasets
        .iter()
        .any(|dataset| datasets::config_reads(&dataset.sql).is_ok_and(|reads| !reads.is_empty()));
    let skills = extension.skills.iter().any(|skill| skill.when.is_some());
    let implementations = extension.implementations.iter().any(|i| i.when.is_some());
    [
        (datasets, ConfigReader::Datasets),
        (skills, ConfigReader::Skills),
        (implementations, ConfigReader::Implementations),
    ]
    .into_iter()
    .filter_map(|(reads, reader)| reads.then_some(reader))
    .collect()
}

/// Whether anything of `extension` reads its configuration
/// ([`configuration_readers`]).
pub fn reads_configuration(extension: &crate::extension::extensions::Extension) -> bool {
    !configuration_readers(extension).is_empty()
}

/// The effective configuration of each extension in `extensions` that reads
/// its own ([`reads_configuration`]), as it is stored now. A configuration that can't be resolved
/// signs as its failure, so repairing it reactivates too.
pub fn config_signature_tx(
    project: &std::path::Path,
    global: &std::path::Path,
    extensions: &[crate::extension::extensions::Extension],
) -> ConfigSignature {
    extensions
        .iter()
        .filter(|extension| extension.enabled && reads_configuration(extension))
        .filter_map(|extension| {
            let declaration = extension.configuration.as_ref()?;
            let signed = match crate::extension::extension_configuration::effective_values(
                project,
                global,
                &extension.namespace,
                declaration,
            ) {
                Ok(values) => values.to_string(),
                Err(error) => format!("unresolved: {error:?}"),
            };
            Some((extension.name.clone(), signed))
        })
        .collect()
}

impl PreparedExtensionDatabase {
    /// Include capability rows/switch events and derive vocabulary recognition
    /// from that candidate's planned selection, rather than the active list.
    pub fn prepare_with_capabilities(
        snapshot: &CatalogSnapshot,
        capabilities: &PreparedCapabilities,
    ) -> Result<Self, VocabularyErrors> {
        if snapshot.generation() != capabilities.generation() {
            return Err(VocabularyErrors::from([(
                "<host>".to_string(),
                vec![Diagnostic::error(
                    "activation.catalog_mismatch",
                    "capabilities and database declarations belong to different catalogs",
                )],
            )]));
        }
        let conflicts = crate::models::ModelRegistry::conflicts(snapshot.extensions());
        if !conflicts.is_empty() {
            return Err(conflicts);
        }
        let mut registry = oxplow_config::settings::SettingsRegistry::default();
        let settings_valid = registry
            .register_many(oxplow_config::core_settings::definitions(""))
            .map_err(|error| error.to_string())
            .and_then(|_| {
                crate::extension::extension_settings::replace_extensions(
                    &mut registry,
                    snapshot.extensions(),
                )
                .map_err(|error| error.to_string())
            });
        if let Err(error) = settings_valid {
            return Err(VocabularyErrors::from([(
                "<settings>".to_owned(),
                vec![Diagnostic::error("activation.settings_registry", error)],
            )]));
        }
        let vocabulary = PreparedVocabulary::prepare(snapshot)?;
        Ok(Self::with_vocabulary(
            snapshot,
            vocabulary,
            Some(capabilities.clone()),
        ))
    }

    fn with_vocabulary(
        snapshot: &CatalogSnapshot,
        vocabulary: PreparedVocabulary,
        capabilities: Option<PreparedCapabilities>,
    ) -> Self {
        let datasets = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && !extension.datasets.is_empty())
            .map(|extension| ExtensionDatasets {
                extension: extension.name.clone(),
                namespace: extension.namespace.clone(),
                sources: extension.datasets.clone(),
                config: None,
            })
            .collect();
        let contributions = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| extension.contributions.iter().cloned())
            .collect();
        let mut listing = PageListing::default();
        for extension in snapshot.extensions().iter().filter(|e| e.enabled) {
            page_rows(extension, &mut listing);
        }
        let PageListing {
            pages,
            answers: page_answers,
            params: page_params,
            sections: page_sections,
        } = listing;
        let listed = snapshot.extensions().iter().map(extension_row).collect();
        let mut settings = std::collections::BTreeMap::new();
        for definition in oxplow_config::core_settings::definitions("")
            .into_iter()
            .chain(
                snapshot
                    .extensions()
                    .iter()
                    .filter(|e| e.enabled)
                    .flat_map(|e| {
                        crate::extension::extension_settings::definitions(e).unwrap_or_default()
                    }),
            )
        {
            settings.insert(definition.key.clone(), definition);
        }
        let config_keys = settings
            .into_values()
            .map(
                |definition| oxplow_db::catalog_listing_store::ConfigKeyRow {
                    key: definition.key,
                    doc: definition.description,
                    schema_json: definition.schema.to_string(),
                },
            )
            .collect();
        let configurations = snapshot
            .extensions()
            .iter()
            .filter(|extension| {
                extension.enabled
                    && configuration_readers(extension).contains(&ConfigReader::Datasets)
            })
            .filter_map(|extension| {
                Some((
                    extension.name.clone(),
                    extension.namespace.clone(),
                    extension.configuration.clone()?,
                ))
            })
            .collect();
        let file_kinds = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| {
                extension.file_kinds.iter().flat_map(|k| {
                    k.globs.iter().map(|glob| {
                        (
                            k.kind.clone(),
                            glob.clone(),
                            k.label.clone(),
                            extension.name.clone(),
                        )
                    })
                })
            })
            .collect();
        let languages = snapshot
            .extensions()
            .iter()
            .flat_map(language_rows)
            .collect();
        Self {
            languages,
            file_kinds,
            configurations,
            entities: crate::collector_runner::declared_entity_tables(snapshot.extensions()),
            models: crate::models::ModelRegistry::of(snapshot.extensions()),
            datasets,
            vocabulary,
            contributions,
            listing: oxplow_db::catalog_listing_store::CatalogListing {
                pages,
                page_answers,
                page_params,
                page_sections,
                extensions: listed,
                config_keys,
            },
            capabilities,
        }
    }

    /// List each extension `held` names with why: the reasons join its row
    /// (its last working version's), or — when nothing of it runs — its
    /// entry in `raw` is listed, off, with them.
    pub(crate) fn list_held(&mut self, held: &VocabularyErrors, raw: &CatalogSnapshot) {
        let rows = &mut self.listing.extensions;
        for (name, why) in held {
            let row = match rows.iter_mut().find(|row| &row.name == name) {
                Some(row) => row,
                None => {
                    let Some(entry) = raw.extensions().iter().find(|e| &e.name == name) else {
                        continue;
                    };
                    rows.push(extension_row(&crate::extension::extensions::Extension {
                        enabled: false,
                        ..entry.clone()
                    }));
                    rows.last_mut().expect("just pushed")
                }
            };
            let mut errors: Vec<Diagnostic> =
                serde_json::from_str(&row.errors_json).unwrap_or_default();
            for reason in why {
                if !errors.iter().any(|d| d.same_as(reason)) {
                    errors.push(reason.clone());
                }
            }
            // Shown under the folder its source is in, as the row's own are.
            let root = raw
                .extensions()
                .iter()
                .find(|e| &e.name == name)
                .map(|e| e.root().to_string())
                .unwrap_or_default();
            row.errors_json = json_text(&crate::extension::extensions::listed(&root, &errors));
        }
    }

    /// Replace datasets, vocabulary, contributions and optional capability rows
    /// and switch events as one database candidate.
    /// Any rejection restores the candidate, even if a trusted caller catches
    /// the rejection. Storage/rollback failures require aborting the outer
    /// transaction. Only storage contention may be retried.
    ///
    /// This makes no outer commit and never swaps the running vocabulary. The
    /// activation owner must commit, coordinate in-memory activation and check
    /// candidate identity under its activation boundary before exposing it.
    /// The shared models this generation activates.
    pub fn models(&self) -> &crate::models::ModelRegistry {
        &self.models
    }

    pub fn replace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        now: Timestamp,
        project: &std::path::Path,
        global: &std::path::Path,
    ) -> Result<Vocabulary, TxError<VocabularyErrors>> {
        tx.execute_batch("SAVEPOINT candidate_extension_database")
            .map_err(oxplow_db::map_sql_err)?;
        let result = (|| {
            oxplow_db::collector_store::ensure_entities_tx(tx, &self.entities)?;
            for (id, spec) in &self.models.models {
                let live = self.models.live_stores(id);
                oxplow_db::model_store::ensure_model_for_stores_tx(tx, &spec.decl, Some(&live))?;
            }
            oxplow_db::model_store::replace_field_decls_tx(tx, &self.models.field_decls())?;
            oxplow_db::model_store::replace_stage_decls_tx(tx, &self.models.stage_decls())?;
            oxplow_db::model_store::replace_store_decls_tx(tx, &self.models.store_decls())?;
            oxplow_db::model_store::replace_file_kind_rules_tx(tx, false, &self.file_kinds)?;
            oxplow_db::model_store::replace_language_decls_tx(tx, &self.languages)?;
            let mut candidate = self.datasets.clone();
            for (extension, namespace, declaration) in &self.configurations {
                let values = crate::extension::extension_configuration::effective_values(
                    project,
                    global,
                    namespace,
                    declaration,
                )
                .map_err(|error| {
                    TxError::Aborted(VocabularyErrors::from([(
                        extension.clone(),
                        vec![Diagnostic::error(
                            "activation.configuration_unreadable",
                            format!(
                                "its stored configuration can't be read for its datasets' \
                                 config() ({error:?})"
                            ),
                        )
                        .fix("fix or reset it")],
                    )]))
                })?;
                if let Some(datasets) = candidate.iter_mut().find(|m| &m.extension == extension) {
                    datasets.config = Some(values);
                }
            }
            datasets::replace_extensions(tx, &candidate).map_err(|error| match error {
                TxError::Aborted(refused) => TxError::Aborted(
                    refused
                        .into_iter()
                        .map(|(owner, why)| {
                            let why = why
                                .into_iter()
                                .map(|line| Diagnostic::error("dataset.invalid", line))
                                .collect();
                            (owner, why)
                        })
                        .collect(),
                ),
                TxError::Storage(error) => TxError::Storage(error),
            })?;
            let vocabulary = self.vocabulary.replace_tx(tx, now)?;
            // The models' edges, repaired when how they're projected (or
            // which ids resolve) changed since their last repair.
            crate::models::repair_edges_if_needed_tx(tx, &self.models, &vocabulary.resources)?;
            oxplow_db::contribution_store::reset_tx(tx, &self.contributions)?;
            oxplow_db::catalog_listing_store::reset_tx(tx, &self.listing)?;
            if let Some(capabilities) = &self.capabilities {
                capabilities.restate_tx(tx, &vocabulary)?;
            }
            Ok(vocabulary)
        })();
        match result {
            Ok(vocabulary) => {
                tx.execute_batch("RELEASE candidate_extension_database")
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(vocabulary)
            }
            Err(error) => {
                rollback_candidate(tx)?;
                Err(error)
            }
        }
    }
}

fn json_text(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("a listed field serializes")
}

/// A serde unit variant's name (`table`, `data`): the lowercase word.
fn variant_name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        _ => String::new(),
    }
}

/// The page files as listed rows: the pages, their answers, params and
/// switch sections.
#[derive(Default)]
struct PageListing {
    pages: Vec<oxplow_db::catalog_listing_store::PageRow>,
    answers: Vec<oxplow_db::catalog_listing_store::PageAnswerRow>,
    params: Vec<oxplow_db::catalog_listing_store::PageParamRow>,
    sections: Vec<oxplow_db::catalog_listing_store::PageSectionRow>,
}

/// The switch a page's view shows its sections by: its view, or the first
/// in a stack of it.
fn page_switch(node: &crate::pages::schema::Node) -> Option<(&str, &[crate::pages::schema::Node])> {
    use crate::pages::schema::NodeKind;
    match &node.kind {
        NodeKind::Switch { param, children } => Some((param.as_str(), children.as_slice())),
        NodeKind::Stack { children } => children.iter().find_map(page_switch),
        _ => None,
    }
}

/// Each section of `page` beside the one it opens on (its switch param's
/// default, else its first), as rows: titled "<page>: <section>", each
/// the page's URI showing it.
fn page_sections(
    page: &crate::pages::schema::Page,
    uri: &str,
) -> Vec<oxplow_db::catalog_listing_store::PageSectionRow> {
    let Some((param, children)) = page_switch(&page.view) else {
        return Vec::new();
    };
    let Ok(base) = uri.parse::<oxplow_domain::uri::OxplowUri>() else {
        return Vec::new();
    };
    let first = page
        .params
        .iter()
        .find(|p| p.name == param)
        .and_then(|p| match p.default.as_ref()? {
            oxplow_db::SqlCell::Null(_) => None,
            oxplow_db::SqlCell::Text(text) => Some(text.clone()),
            other => Some(json_text(other).trim_matches('"').to_string()),
        })
        .or_else(|| children.first().map(|c| c.id.clone()));
    children
        .iter()
        .filter(|c| Some(&c.id) != first.as_ref())
        .enumerate()
        .map(|(i, c)| oxplow_db::catalog_listing_store::PageSectionRow {
            page_uri: uri.to_string(),
            position: i as i64 + 1,
            section: c.id.clone(),
            title: format!("{}: {}", page.title, c.title.as_deref().unwrap_or(&c.id)),
            uri: base
                .with_view([(param.to_string(), c.id.clone())].into())
                .to_string(),
        })
        .collect()
}

/// An extension's page files as listed rows, pushed onto `listing`.
fn page_rows(extension: &crate::extension::extensions::Extension, listing: &mut PageListing) {
    use oxplow_db::catalog_listing_store::{PageAnswerRow, PageParamRow, PageRow};
    let PageListing {
        pages,
        answers,
        params,
        sections,
    } = listing;
    for page in &extension.page_files {
        let uri = oxplow_domain::uri::build::extension_page_uri(page.namespace(), &page.slug);
        pages.push(PageRow {
            uri: uri.clone(),
            extension: page.extension.clone(),
            id: page.id.clone(),
            title: page.title.clone(),
            description: page.description.clone(),
            label: page.label.clone(),
            icon: page.icon.clone(),
            category: page.category.as_ref().map(variant_name),
            keywords: page.keywords.clone(),
            params_json: json_text(&page.params),
            renders_json: json_text(&page.renders),
            is_default: page.default,
            pinned: page.pinned,
            kind: variant_name(&page.kind),
            tool: page.tool.clone(),
            needs_json: json_text(&page.needs),
            source: page.origin.source.clone(),
            deployment: deployment_name(&extension.deployment).to_string(),
            running_uri: page.origin.running_uri.clone(),
        });
        sections.extend(page_sections(page, &uri));
        answers.extend(page.answers.iter().enumerate().map(|(i, a)| PageAnswerRow {
            page_uri: uri.clone(),
            position: i as i64 + 1,
            question: a.question.clone(),
            about: a.about.clone(),
            params_json: json_text(&a.params),
        }));
        params.extend(page.params.iter().enumerate().map(|(i, p)| {
            PageParamRow {
                page_uri: uri.clone(),
                position: i as i64 + 1,
                name: p.name.clone(),
                r#type: variant_name(&p.kind()),
                role: p
                    .role
                    .as_ref()
                    .map_or_else(|| "resource".to_string(), variant_name),
                label: p.label.clone(),
                default_json: p.default.as_ref().map(json_text),
                options_json: json_text(&p.options),
            }
        }));
    }
}

fn deployment_name(deployment: &crate::extension::extensions::Deployment) -> &'static str {
    match deployment {
        crate::extension::extensions::Deployment::Source => "source",
        crate::extension::extensions::Deployment::Shipped { .. } => "shipped",
        crate::extension::extensions::Deployment::Deployed { .. } => "deployed",
        crate::extension::extensions::Deployment::Undeployed => "undeployed",
        crate::extension::extensions::Deployment::NotHere => "not_here",
    }
}

pub(crate) fn extension_row(
    extension: &crate::extension::extensions::Extension,
) -> oxplow_db::catalog_listing_store::ExtensionRow {
    oxplow_db::catalog_listing_store::ExtensionRow {
        name: extension.name.clone(),
        namespace: extension.namespace.clone(),
        description: extension.description.clone(),
        path: extension.path.clone(),
        origin: extension.origin.clone(),
        version: extension.version.clone(),
        enabled: extension.enabled,
        purpose: extension
            .intent
            .as_ref()
            .map(|intent| intent.purpose.clone()),
        deployment: deployment_name(&extension.deployment).to_string(),
        hash: match &extension.deployment {
            crate::extension::extensions::Deployment::Shipped { hash }
            | crate::extension::extensions::Deployment::Deployed { hash } => Some(hash.clone()),
            _ => None,
        },
        errors_json: json_text(&extension.listed(&extension.errors)),
        warnings_json: json_text(&extension.listed(&extension.warnings)),
        permissions_json: json_text(&extension.permissions),
        pending_json: json_text(&extension.pending),
        shared: extension.shared,
    }
}

fn rollback_candidate(tx: &rusqlite::Transaction<'_>) -> Result<(), DomainError> {
    tx.execute_batch(
        "ROLLBACK TO candidate_extension_database; RELEASE candidate_extension_database",
    )
    .map_err(oxplow_db::map_sql_err)
}

/// `extension`'s languages as `v_language` rows: what each is, and which
/// facets its `analysis.yaml` and starter give it.
fn language_rows(
    extension: &crate::extension::extensions::Extension,
) -> impl Iterator<Item = oxplow_db::model_store::LanguageDeclRow> + '_ {
    extension.languages.iter().map(move |source| {
        let decl = &source.decl;
        let spec = source
            .analysis
            .as_deref()
            .and_then(|text| oxplow_code_metrics::LanguageSpec::parse(text).ok());
        let has = |present: bool| i64::from(present);
        oxplow_db::model_store::LanguageDeclRow {
            id: decl.id.clone(),
            title: decl.title.clone(),
            extension: extension.name.clone(),
            enabled: extension.enabled,
            analyzable: decl.grammar.is_some(),
            extensions: decl.extensions.clone(),
            aliases: decl.aliases.clone(),
            lsp_ids: decl.lsp_ids.clone(),
            lsp_package: decl.lsp.as_ref().map(|l| l.package.clone()),
            monaco: decl.editor.as_ref().map(|e| e.monaco.clone()),
            facets: serde_json::json!({
                "imports": has(spec.as_ref().is_some_and(|s| s.imports.is_some())),
                "merge": has(spec.as_ref().is_some_and(|s| s.merge.is_some())),
                "duplication": has(spec.as_ref().is_some_and(|s| {
                    !s.duplication.skip_kinds.is_empty() || !s.duplication.skip_form_heads.is_empty()
                })),
                "starter": has(decl.starter.is_some()),
            }),
        }
    })
}
