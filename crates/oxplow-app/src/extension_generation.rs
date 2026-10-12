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
    capabilities: Option<PreparedCapabilities>,
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
            })
            .collect();
        let contributions = snapshot
            .extensions()
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| extension.contributions.iter().cloned())
            .collect();
        Self {
            entities: crate::collector_runner::declared_entity_tables(snapshot.extensions()),
            models,
            vocabulary,
            contributions,
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
            models::replace_extensions(tx, &self.models)?;
            let vocabulary = self.vocabulary.replace_tx(tx, now)?;
            oxplow_db::contribution_store::reset_tx(tx, &self.contributions)?;
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

fn rollback_candidate(tx: &rusqlite::Transaction<'_>) -> Result<(), DomainError> {
    tx.execute_batch(
        "ROLLBACK TO candidate_extension_database; RELEASE candidate_extension_database",
    )
    .map_err(oxplow_db::map_sql_err)
}
