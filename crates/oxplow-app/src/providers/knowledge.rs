//! An external provider's knowledge store: [`ExternalKnowledge`] is the
//! store registered under the instance's id while it runs
//! (`.context/knowledge.md` "The capability", `.context/providers.md` "What
//! a provider may implement"). The page commands check a write against
//! core's record, then call it; each verb is one invoke:
//!
//! - `write_page { id?, title?, body, verified_files, removed_files }` (no
//!   `id`: the store derives one from the title),
//! - `delete_page { id }`,
//!
//! `write_page` answering the page's `knowledge.page.recorded@3` — its id,
//! the body it keeps and the refs it states, in oxplow's grammar — which
//! core records (row, links, pins) the same way it records oxwiki's. Adding
//! a link is core's: a `write_page` of the body with the link added. Its
//! collector streams its pages as the same records, for pages edited
//! outside oxplow; [`KnowledgeRecords`] records those while it is enabled.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::capability::KNOWLEDGE;
use oxplow_domain::events::schema::{
    EventType as _, KnowledgePageRecorded, KnowledgePageRecordedV3,
};
use oxplow_domain::events::Envelope;
use oxplow_domain::knowledge::{
    Kept, KindDecl, KnowledgeFeatures, KnowledgeProvider, KnowledgeStores, PageCall, PageDraft,
};
use oxplow_domain::{CommandError, DomainError, InputValidator};
use serde_json::{json, Value};

use super::endpoint::Endpoint;
use super::registry::{CapabilityHost, PreparedProviderCapability};

/// The knowledge side of the host: a started instance is a store in
/// `Services.knowledge`, under its id. Retired (stopped, or withdrawn
/// across a generation), its pages stay recorded and hidden
/// (`v_knowledge_page` lists the enabled stores' only).
pub struct KnowledgeHost {
    pub stores: Arc<KnowledgeStores>,
}

impl KnowledgeHost {
    pub const CAPABILITY: &'static str = "knowledge";
}

impl CapabilityHost for KnowledgeHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    fn has(&self, id: &str) -> bool {
        self.stores.get(id).is_some()
    }

    fn prepare(&self, instance: &Arc<dyn Endpoint>) -> Result<PreparedProviderCapability, String> {
        let declared = instance.declared();
        let decl = declared
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .ok_or("it declares no `knowledge` capability")?;
        let features = Some(decl.features.clone())
            .filter(|f| !f.is_null())
            .unwrap_or_else(|| json!({}));
        let kinds: Vec<KindDecl> = match decl.data.get("kinds") {
            None => vec![KindDecl::note()],
            Some(kinds) => serde_json::from_value(kinds.clone())
                .map_err(|e| format!("its `data.kinds` aren't `[{{ id, title, bucket }}]`: {e}"))?,
        };
        if kinds.is_empty() {
            return Err("it declares no `data.kinds`".into());
        }
        let edit = features
            .get("edit")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let published_kinds = serde_json::to_value(&kinds).expect("kinds serialize");
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
            kinds,
            features: KnowledgeFeatures { edit },
        });
        let stores = self.stores.clone();
        Ok(PreparedProviderCapability::new(features, move || {
            stores.register(adapter);
        })
        .with_kinds(published_kinds))
    }

    fn retire(&self, id: &str) {
        self.stores.unregister(id);
    }
}

pub struct ExternalKnowledge {
    instance: Arc<dyn Endpoint>,
    /// Each contract verb's compiled input schema.
    verbs: BTreeMap<&'static str, InputValidator>,
    kinds: Vec<KindDecl>,
    features: KnowledgeFeatures,
}

impl ExternalKnowledge {
    fn failed(&self, message: impl std::fmt::Display) -> CommandError {
        CommandError::Failed {
            message: format!("knowledge store `{}`: {message}", self.instance.name()),
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
                message: format!("{} `{verb}`: {message}", self.instance.id()),
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
            .map(|d| self.instance.envelope(call.actor(), d))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((out.result, events))
    }

    /// What it kept of the page `id` (the one it derived when `None`): its
    /// record of the page — one of its own, the only one it answered when
    /// it derived the id — and every event it recorded.
    fn kept(&self, id: Option<&str>, events: Vec<Envelope>) -> Result<Kept, CommandError> {
        let mine: Vec<(String, oxplow_domain::events::schema::KnowledgePageRecord)> = events
            .iter()
            .filter(|e| e.event_type == KnowledgePageRecorded::TYPE)
            .filter_map(|e| {
                let record =
                    serde_json::from_value::<KnowledgePageRecordedV3>(e.payload.clone()).ok()?;
                let (store, page) = oxplow_domain::uri::build::knowledge_of_uri(&record.page.uri)?;
                (store == self.id()).then_some((page, record.page))
            })
            .collect();
        let found = match id {
            Some(id) => mine.into_iter().find(|(page, _)| page == id),
            None if mine.len() == 1 => mine.into_iter().next(),
            None => None,
        };
        let (id, record) = found.ok_or_else(|| {
            self.failed(match id {
                Some(id) => format!("it answered no record of its page `{id}`"),
                None => "it answered no one record of the page it named".to_string(),
            })
        })?;
        if let Some(kind) = &record.kind {
            if !self.kinds.iter().any(|k| &k.id == kind) {
                return Err(
                    self.failed(format!("its page `{id}` is of an undeclared kind `{kind}`"))
                );
            }
        }
        Ok(Kept {
            id,
            body: record.body,
            title: Some(record.title),
            kind: record.kind,
            url: record.url,
            author: record.author,
            refs: Some(record.uris),
            events,
        })
    }
}

#[async_trait]
impl KnowledgeProvider for ExternalKnowledge {
    fn id(&self) -> &str {
        self.instance.id()
    }

    fn kinds(&self) -> Vec<KindDecl> {
        self.kinds.clone()
    }

    fn features(&self) -> KnowledgeFeatures {
        self.features
    }

    async fn write_page(
        &self,
        call: &PageCall<'_>,
        draft: &PageDraft,
    ) -> Result<Kept, CommandError> {
        let mut input = json!({
            "body": draft.body,
            "verified_files": draft.verified_files,
            "removed_files": draft.removed_files,
        });
        if let Some(id) = &draft.id {
            input["id"] = json!(id);
        }
        if let Some(title) = &draft.title {
            input["title"] = json!(title);
        }
        let (_, events) = self.call(call, "write_page", input).await?;
        self.kept(draft.id.as_deref(), events)
    }

    async fn delete_page(
        &self,
        call: &PageCall<'_>,
        id: &str,
    ) -> Result<Vec<Envelope>, CommandError> {
        let (_, events) = self.call(call, "delete_page", json!({ "id": id })).await?;
        Ok(events)
    }
}

/// Records the pages an enabled store's collector streamed (edited
/// outside oxplow): each `knowledge.page.recorded` a provider's read logged
/// (source `provider:<id>`) of its own pages is restated into core's record
/// — or, `deleted`, taken out of it. A record whose body the record already
/// holds restates nothing; one a page command answered was recorded by the
/// command.
pub struct KnowledgeRecords {
    pub stores: Arc<KnowledgeStores>,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
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
        use oxplow_db::knowledge_record::{body_hash, delete_page_tx, write_page_tx, PageWrite};
        let env = &event.envelope;
        let KnowledgePageRecordedV3 { page } = serde_json::from_value(env.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("knowledge.page.recorded payload: {e}")))?;
        // Its own pages, while it's enabled.
        let Some((store, id)) = oxplow_domain::uri::build::knowledge_of_uri(&page.uri) else {
            return Ok(());
        };
        if env.source != oxplow_domain::uri::build::provider_source(&store) {
            return Ok(());
        }
        let Some(provider) = self.stores.get(&store) else {
            return Ok(());
        };
        // Its URI's one spelling, whatever order the provider wrote.
        let uri = oxplow_domain::uri::build::knowledge_uri(&store, &id);
        // A kind it doesn't declare is no kind: the store's first.
        let kind = page
            .kind
            .clone()
            .filter(|k| provider.kinds().iter().any(|d| &d.id == k));
        let updated_at = oxplow_db::string_to_ts(&page.updated_at)
            .unwrap_or_else(|_| oxplow_domain::Timestamp::now());
        let (vocabulary, cause) = (self.vocabulary.clone(), env.id.clone());
        self.db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let mut ev = oxplow_db::EventCtx::system(&vocabulary, "knowledge_records");
                ev.cause = Some(cause.clone());
                let logged = if page.deleted {
                    delete_page_tx(tx, &ev, oxplow_domain::Anchors::default(), &uri)?
                } else {
                    // What the record already holds restates nothing.
                    let same = oxplow_db::knowledge_page_store::get_tx(tx, &uri)?.is_some_and(
                        |(row, hash)| {
                            hash == body_hash(&page.body)
                                && row.title == page.title
                                && row.kind == kind
                                && row.url == page.url
                                && row.author == page.author
                        },
                    );
                    if same {
                        return Ok(());
                    }
                    let (_, env) = write_page_tx(
                        tx,
                        &ev,
                        &PageWrite {
                            title: Some(&page.title),
                            kind: kind.as_deref(),
                            url: page.url.as_deref(),
                            author: page.author.as_deref(),
                            stated: Some(&page.uris),
                            updated_at,
                            ..PageWrite::new(&store, &id, &page.body)
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
