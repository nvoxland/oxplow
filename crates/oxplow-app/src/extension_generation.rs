//! The database portion of an extension generation candidate. Runtime registry
//! publication and compatible reader pinning belong to the activation owner.

use oxplow_db::models::{self, ExtensionModels};
use oxplow_db::TxError;
use oxplow_domain::{vocabulary::Vocabulary, DomainError, Timestamp};

use crate::capabilities::PreparedCapabilities;
use crate::extension_catalog::CatalogSnapshot;
use crate::vocabulary_reactor::{PreparedVocabulary, VocabularyErrors, VocabularyService};

/// Database declarations copied from one catalog snapshot. No external code
/// runs during preparation, and nothing is published by constructing this.
pub struct PreparedExtensionDatabase {
    /// Declared entities, created empty when absent so models over them
    /// compile before their collectors first run.
    entities: Vec<oxplow_db::collector_store::EntityTable>,
    models: Vec<ExtensionModels>,
    vocabulary: PreparedVocabulary,
    contributions: Vec<oxplow_domain::contribution::Descriptor>,
    /// The enabled extensions' pages, every loaded extension, what a
    /// person can ask and what they can configure, listed.
    listing: oxplow_db::catalog_listing_store::CatalogListing,
    capabilities: Option<PreparedCapabilities>,
    /// The declarations of the extensions whose models read their own
    /// configuration (`config()`): their effective values are resolved in the
    /// publishing transaction and compiled into the views.
    configurations: Vec<(String, crate::extensions::config_schema::ConfigurationFile)>,
}

/// The effective configuration the published models were compiled against,
/// per extension that reads any; a change in it republishes them.
pub type ConfigSignature = std::collections::BTreeMap<String, String>;

/// Whether any model of `extension` reads its configuration with `config()`.
pub fn reads_configuration(extension: &crate::extensions::Extension) -> bool {
    extension.configuration.is_some()
        && extension
            .datasets
            .iter()
            .any(|model| models::config_reads(&model.sql).is_ok_and(|reads| !reads.is_empty()))
}

/// The effective configuration of each extension in `extensions` whose models
/// read theirs, as it is stored now. A configuration that can't be resolved
/// signs as its failure, so repairing it republishes too.
pub fn config_signature_tx(
    conn: &rusqlite::Connection,
    extensions: &[crate::extensions::Extension],
) -> ConfigSignature {
    extensions
        .iter()
        .filter(|extension| extension.enabled && reads_configuration(extension))
        .filter_map(|extension| {
            let declaration = extension.configuration.as_ref()?;
            let signed = match crate::extension_configuration::effective_values(
                conn,
                &extension.name,
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
    pub fn prepare(
        snapshot: &CatalogSnapshot,
        vocabulary: &VocabularyService,
    ) -> Result<Self, VocabularyErrors> {
        let vocabulary = vocabulary.prepare(snapshot)?;
        Ok(Self::with_vocabulary(snapshot, vocabulary, None))
    }

    /// Include capability rows/switch events and derive vocabulary recognition
    /// from that candidate's planned selection, rather than the active list.
    pub fn prepare_with_capabilities(
        snapshot: &CatalogSnapshot,
        capabilities: &PreparedCapabilities,
    ) -> Result<Self, VocabularyErrors> {
        if snapshot.generation() != capabilities.generation() {
            return Err(VocabularyErrors::from([(
                "<host>".to_string(),
                vec!["capabilities and database declarations belong to different catalogs".into()],
            )]));
        }
        let vocabulary = PreparedVocabulary::prepare(snapshot, capabilities.work_item_ids())?;
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
        let models = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && !extension.datasets.is_empty())
            .map(|extension| ExtensionModels {
                extension: extension.name.clone(),
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
        let (mut pages, mut page_answers, mut page_params) = (Vec::new(), Vec::new(), Vec::new());
        for extension in snapshot.extensions().iter().filter(|e| e.enabled) {
            page_rows(extension, &mut pages, &mut page_answers, &mut page_params);
        }
        let listed = snapshot.extensions().iter().map(extension_row).collect();
        let config_keys = oxplow_config::keys::config_keys()
            .into_iter()
            .map(|k| oxplow_db::catalog_listing_store::ConfigKeyRow {
                schema_json: json_text(&k.schema),
                key: k.key,
                doc: k.doc,
            })
            .collect();
        let configurations = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled && reads_configuration(extension))
            .filter_map(|extension| {
                Some((extension.name.clone(), extension.configuration.clone()?))
            })
            .collect();
        Self {
            configurations,
            entities: crate::collector_runner::declared_entity_tables(snapshot.extensions()),
            models,
            vocabulary,
            contributions,
            listing: oxplow_db::catalog_listing_store::CatalogListing {
                pages,
                page_answers,
                page_params,
                extensions: listed,
                config_keys,
            },
            capabilities,
        }
    }

    pub fn generation(&self) -> &str {
        self.vocabulary.generation()
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
                    rows.push(extension_row(&crate::extensions::Extension {
                        enabled: false,
                        ..entry.clone()
                    }));
                    rows.last_mut().expect("just pushed")
                }
            };
            let mut errors: Vec<String> =
                serde_json::from_str(&row.errors_json).unwrap_or_default();
            for reason in why {
                if !errors.contains(reason) {
                    errors.push(reason.clone());
                }
            }
            row.errors_json = json_text(&errors);
        }
    }

    /// Replace models, vocabulary, contributions and optional capability rows
    /// and switch events as one database candidate.
    /// Any rejection restores the candidate, even if a trusted caller catches
    /// the rejection. Storage/rollback failures require aborting the outer
    /// transaction. Only storage contention may be retried.
    ///
    /// This makes no outer commit and never swaps the running vocabulary. The
    /// activation owner must commit, coordinate in-memory publication and check
    /// candidate identity under its publication boundary before exposing it.
    pub fn replace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        now: Timestamp,
    ) -> Result<Vocabulary, TxError<VocabularyErrors>> {
        tx.execute_batch("SAVEPOINT candidate_extension_database")
            .map_err(oxplow_db::map_sql_err)?;
        let result = (|| {
            oxplow_db::collector_store::ensure_entities_tx(tx, &self.entities)?;
            let mut candidate = self.models.clone();
            for (extension, declaration) in &self.configurations {
                let values =
                    crate::extension_configuration::effective_values(tx, extension, declaration)
                        .map_err(|error| {
                            TxError::Aborted(VocabularyErrors::from([(
                                extension.clone(),
                                vec![format!(
                                    "its stored configuration can't be read for its models' \
                                     config() ({error:?}); fix or reset it"
                                )],
                            )]))
                        })?;
                if let Some(models) = candidate.iter_mut().find(|m| &m.extension == extension) {
                    models.config = Some(values);
                }
            }
            models::replace_extensions(tx, &candidate)?;
            let vocabulary = self.vocabulary.replace_tx(tx, now)?;
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

/// An extension's page files as listed rows, with their answers and
/// params, pushed onto the three lists.
fn page_rows(
    extension: &crate::extensions::Extension,
    pages: &mut Vec<oxplow_db::catalog_listing_store::PageRow>,
    answers: &mut Vec<oxplow_db::catalog_listing_store::PageAnswerRow>,
    params: &mut Vec<oxplow_db::catalog_listing_store::PageParamRow>,
) {
    use oxplow_db::catalog_listing_store::{PageAnswerRow, PageParamRow, PageRow};
    for page in &extension.page_files {
        let uri = if extension.namespace == crate::extensions::CORE_NAMESPACE {
            oxplow_domain::uri::build::page_uri(&page.slug, &[])
        } else {
            oxplow_domain::uri::build::extension_page_uri(&page.extension, &page.slug)
        };
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
            hidden: page.hidden,
            tool: page.tool.clone(),
            needs_json: json_text(&page.needs),
            source: page.origin.source.clone(),
            deployment: deployment_name(&extension.deployment).to_string(),
        });
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

fn deployment_name(deployment: &crate::extensions::Deployment) -> &'static str {
    match deployment {
        crate::extensions::Deployment::Source => "source",
        crate::extensions::Deployment::Shipped { .. } => "shipped",
        crate::extensions::Deployment::Deployed { .. } => "deployed",
        crate::extensions::Deployment::Undeployed => "undeployed",
        crate::extensions::Deployment::NotHere => "not_here",
    }
}

pub(crate) fn extension_row(
    extension: &crate::extensions::Extension,
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
            crate::extensions::Deployment::Shipped { hash }
            | crate::extensions::Deployment::Deployed { hash } => Some(hash.clone()),
            _ => None,
        },
        errors_json: json_text(&extension.errors),
        warnings_json: json_text(&extension.warnings),
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
