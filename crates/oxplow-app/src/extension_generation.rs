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
    /// The enabled extensions' lenses and every loaded extension, listed.
    lenses: Vec<oxplow_db::catalog_listing_store::LensRow>,
    listed: Vec<oxplow_db::catalog_listing_store::ExtensionRow>,
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
            .models
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
            .filter(|extension| extension.enabled && !extension.models.is_empty())
            .map(|extension| ExtensionModels {
                extension: extension.name.clone(),
                sources: extension.models.clone(),
                config: None,
            })
            .collect();
        let contributions = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| extension.contributions.iter().cloned())
            .collect();
        let lenses = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| extension.lenses.iter().map(lens_row))
            .collect();
        let listed = snapshot.extensions().iter().map(extension_row).collect();
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
            lenses,
            listed,
            capabilities,
        }
    }

    pub fn generation(&self) -> &str {
        self.vocabulary.generation()
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
            oxplow_db::catalog_listing_store::reset_tx(tx, &self.lenses, &self.listed)?;
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

fn lens_row(lens: &crate::extensions::Lens) -> oxplow_db::catalog_listing_store::LensRow {
    oxplow_db::catalog_listing_store::LensRow {
        id: lens.id.clone(),
        extension: lens.extension.clone(),
        slug: lens.slug.clone(),
        title: lens.title.clone(),
        description: lens.description.clone(),
        viz: variant_name(&lens.viz),
        params_json: json_text(&lens.params),
        hidden: lens.hidden,
        launcher_category: lens.launcher_category.as_ref().map(variant_name),
        needs_json: json_text(&lens.needs),
        path: lens.path.clone(),
    }
}

fn extension_row(
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
        sharing: variant_name(&extension.sharing),
        purpose: extension
            .intent
            .as_ref()
            .map(|intent| intent.purpose.clone()),
        source_git: extension.source.as_ref().map(|source| source.git.clone()),
        source_sha: extension.source.as_ref().map(|source| source.sha.clone()),
        errors_json: json_text(&extension.errors),
        warnings_json: json_text(&extension.warnings),
    }
}

fn rollback_candidate(tx: &rusqlite::Transaction<'_>) -> Result<(), DomainError> {
    tx.execute_batch(
        "ROLLBACK TO candidate_extension_database; RELEASE candidate_extension_database",
    )
    .map_err(oxplow_db::map_sql_err)
}
