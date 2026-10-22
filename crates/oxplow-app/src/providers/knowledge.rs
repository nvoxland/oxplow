//! An external provider's knowledge store: [`ExternalKnowledge`] is the
//! store registered under the instance's id while it runs
//! (`.context/knowledge.md` "The capability", `.context/providers.md` "What
//! a provider may implement"). The page commands check a write against
//! core's record, then call it; each verb is one invoke:
//!
//! - `write_page { slug, title?, body, verified_files, removed_files }`,
//! - `delete_page { slug }`,
//! - `link { page, target }` (`target` a wikilink's inside),
//!
//! each answering the page's `knowledge.page.recorded@2` — the body it
//! keeps and the refs it states, in oxplow's grammar — which core records
//! (row, links, pins) the same way it records the wiki's. Its collector
//! streams its pages as the same records, for pages edited outside oxplow;
//! [`KnowledgeRecords`] records those while it is the active store.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::capability::KNOWLEDGE;
use oxplow_domain::events::schema::{
    EventType as _, KnowledgePageRecorded, KnowledgePageRecordedV2,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::knowledge::{Kept, KnowledgeProvider, KnowledgeRegistry, PageCall, PageDraft};
use oxplow_domain::{CommandError, DomainError, InputValidator};
use serde_json::{json, Value};

use super::registry::{CapabilityHost, Instance, PreparedProviderCapability};

/// The knowledge side of the host: a started instance is a store in
/// `Services.knowledge`, under its id.
pub struct KnowledgeHost(pub Arc<KnowledgeRegistry>);

impl KnowledgeHost {
    pub const CAPABILITY: &'static str = "knowledge";
}

impl CapabilityHost for KnowledgeHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.0.has(id)
    }

    fn prepare(&self, instance: &Arc<Instance>) -> Result<PreparedProviderCapability, String> {
        let declared = &instance.declared;
        let features = declared
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .ok_or("it declares no `knowledge` capability")?
            .features
            .clone();
        let features = Some(features)
            .filter(|f| !f.is_null())
            .unwrap_or_else(|| json!({}));
        let mut verbs = BTreeMap::new();
        for verb in KNOWLEDGE.verbs {
            let c = declared
                .commands
                .iter()
                .find(|c| c.name == verb.name)
                .ok_or_else(|| format!("it declares no `{}` verb", verb.name))?;
            let input = InputValidator::compile(&c.input_schema)
                .map_err(|e| format!("verb `{}`: {e}", verb.name))?;
            verbs.insert(verb.name, input);
        }
        let adapter = Arc::new(ExternalKnowledge {
            instance: instance.clone(),
            verbs,
        });
        let registry = self.0.clone();
        Ok(PreparedProviderCapability::new(features, move || {
            registry.register(adapter);
        }))
    }

    fn retire(&self, id: &str) {
        self.0.unregister(id);
    }
}

pub struct ExternalKnowledge {
    instance: Arc<Instance>,
    /// Each contract verb's compiled input schema.
    verbs: BTreeMap<&'static str, InputValidator>,
}

impl ExternalKnowledge {
    fn failed(&self, message: impl std::fmt::Display) -> CommandError {
        CommandError::Failed {
            message: format!("knowledge store `{}`: {message}", self.instance.name),
        }
    }

    /// Invoke `verb`, answering its result and the events it recorded, as
    /// the envelopes core logs.
    async fn call(
        &self,
        call: &PageCall<'_>,
        verb: &'static str,
        input: Value,
    ) -> Result<(Value, Vec<Envelope>), CommandError> {
        let schema = self
            .verbs
            .get(verb)
            .ok_or_else(|| self.failed(format!("it declares no `{verb}` verb")))?;
        schema.check(&input).map_err(|e| match e {
            CommandError::Invalid { field, message } => CommandError::Invalid {
                field,
                message: format!("{} `{verb}`: {message}", self.instance.id),
            },
            other => other,
        })?;
        let key = call
            .idempotency_key
            .clone()
            .unwrap_or_else(|| format!("{verb}:{}", uuid::Uuid::new_v4().simple()));
        let out = self
            .instance
            .invoke(call.run, verb, input, Some(key))
            .await?;
        let events = out
            .events
            .into_iter()
            .map(|d| self.instance.envelope(call.actor(), None, d))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((out.result, events))
    }

    /// What it kept of `slug`: the body and refs its record of the page
    /// says, and every event it recorded.
    fn kept(&self, slug: &str, events: Vec<Envelope>) -> Result<Kept, CommandError> {
        let page = crate::knowledge::page_ref(slug);
        let record = events
            .iter()
            .filter(|e| e.event_type == KnowledgePageRecorded::TYPE)
            .filter_map(|e| {
                serde_json::from_value::<KnowledgePageRecordedV2>(e.payload.clone()).ok()
            })
            .find(|r| r.page.uri == page)
            .ok_or_else(|| self.failed(format!("it answered no record of `{page}`")))?;
        Ok(Kept {
            body: record.page.body,
            refs: Some(record.page.uris),
            events,
        })
    }
}

#[async_trait]
impl KnowledgeProvider for ExternalKnowledge {
    fn id(&self) -> &str {
        &self.instance.id
    }

    async fn write_page(
        &self,
        call: &PageCall<'_>,
        draft: &PageDraft,
    ) -> Result<Option<Kept>, CommandError> {
        let mut input = json!({
            "slug": draft.slug,
            "body": draft.body,
            "verified_files": draft.verified_files,
            "removed_files": draft.removed_files,
        });
        if let Some(title) = &draft.title {
            input["title"] = json!(title);
        }
        let (_, events) = self.call(call, "write_page", input).await?;
        self.kept(&draft.slug, events).map(Some)
    }

    async fn delete_page(
        &self,
        call: &PageCall<'_>,
        slug: &str,
    ) -> Result<Option<Vec<Envelope>>, CommandError> {
        let (_, events) = self
            .call(call, "delete_page", json!({ "slug": slug }))
            .await?;
        Ok(Some(events))
    }

    async fn link(
        &self,
        call: &PageCall<'_>,
        slug: &str,
        target: &str,
    ) -> Result<Option<Kept>, CommandError> {
        let (_, events) = self
            .call(call, "link", json!({ "page": slug, "target": target }))
            .await?;
        self.kept(slug, events).map(Some)
    }
}

/// Records the active store's pages its collector streamed (edited
/// outside oxplow): each `knowledge.page.recorded` a provider's read logged
/// (source `provider:<id>`) while that provider is the active store is
/// restated into core's record — or, `deleted`, taken out of it. A
/// record whose body the record already holds restates nothing; one a
/// page command answered was recorded by the command.
pub struct KnowledgeRecords {
    pub stores: Arc<KnowledgeRegistry>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
    pub project_dir: std::path::PathBuf,
}

impl KnowledgeRecords {
    pub const NAME: &'static str = "knowledge.records";
}

#[async_trait]
impl crate::event_pump::AsyncEventConsumer for KnowledgeRecords {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == KnowledgePageRecorded::TYPE
    }

    async fn handle(&self, event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        let env = &event.envelope;
        if env.source != oxplow_domain::uri::build::provider_source(&self.stores.active_id()) {
            return Ok(());
        }
        let KnowledgePageRecordedV2 { page } = serde_json::from_value(env.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("knowledge.page.recorded payload: {e}")))?;
        let Some(slug) =
            oxplow_domain::uri::build::id_of(&page.uri, oxplow_domain::uri::resources::WIKI)
                .ok()
                .filter(|s| crate::knowledge::valid_slug(s))
        else {
            return Ok(());
        };
        let (vocabulary, project_dir, cause) = (
            self.vocabulary.clone(),
            self.project_dir.clone(),
            env.id.clone(),
        );
        self.db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let mut ev = oxplow_db::EventCtx::system(&vocabulary, "knowledge_records");
                ev.cause = Some(cause.clone());
                let logged = if page.deleted {
                    crate::knowledge::delete_page_tx(
                        tx,
                        &ev,
                        oxplow_domain::Anchors::default(),
                        &slug,
                    )?
                } else {
                    let stored = oxplow_db::wiki_page_store::get_tx(tx, &slug)?;
                    let body = crate::wiki_pages::strip_body_version_literals(&page.body);
                    if stored.is_some_and(|(_, hash)| hash == crate::knowledge::body_hash(&body)) {
                        return Ok(());
                    }
                    let (_, env) = crate::knowledge::write_page_tx(
                        tx,
                        &ev,
                        &project_dir,
                        &crate::knowledge::PageWrite {
                            slug: &slug,
                            body: &body,
                            verified: &[],
                            removed: &[],
                            updated_at: oxplow_domain::Timestamp::now(),
                            stated: Some(&page.uris),
                            anchors: oxplow_domain::Anchors::default(),
                        },
                    )?;
                    Some(env)
                };
                if let Some(env) = logged {
                    ev.append(tx, &env)?;
                }
                Ok(())
            })
            .await
    }
}
