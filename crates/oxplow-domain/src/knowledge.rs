//! The knowledge capability (P5.C4, `.context/knowledge.md`): pages of
//! durable understanding and what they rely on. A project chooses where
//! they're kept — oxplow's wiki (the bundled built-in), a documentation
//! system an extension's provider speaks to (an Obsidian vault, a
//! Confluence or MediaWiki server), or none — and the active one is
//! [`KnowledgeRegistry::active`]. Reads are SQL over `v_knowledge_page`;
//! writes go through the active store, and core records what it kept.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::events::Envelope;
use crate::work_items::ActiveSource;
use crate::{Actor, CommandError};

/// A page to write.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageDraft {
    pub slug: String,
    /// Sets the page's heading.
    pub title: Option<String>,
    pub body: String,
    /// Refs re-checked against this body: their pins move to now.
    pub verified_refs: Vec<String>,
    /// Refs taken out of the body.
    pub removed_refs: Vec<String>,
}

/// How current one of a page's refs is: core's, from the pins it keeps for
/// every store (`v_knowledge_ref`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefFreshness {
    /// What the page relies on (`file:src/lib.rs`).
    pub target: String,
    /// The snapshot the ref was pinned to, if any.
    pub pinned_snapshot: Option<i64>,
    /// The VCS revision nearest the pin (`None` without one), and whether
    /// the pinned snapshot is exactly that revision.
    pub pinned_revision: Option<String>,
    pub pinned_revision_exact: bool,
    /// The target's latest snapshot, if it has one.
    pub latest_snapshot: Option<i64>,
    /// The target changed since it was pinned (or was never pinned but
    /// has been captured).
    pub stale: bool,
}

/// Who writes a page, as a store sees it.
#[derive(Debug, Clone)]
pub struct PageCall<'a> {
    pub actor: &'a Actor,
    /// The write's idempotency key (an effect's step's); `None` mints one
    /// where it's re-sent.
    pub idempotency_key: Option<String>,
}

/// What a store kept of a write: the page's body as it now stands there
/// (what core records, pins and links from), and the events it reports
/// of its own (a provider's `knowledge.page.recorded`).
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub body: String,
    pub events: Vec<Envelope>,
}

/// Where a project keeps its pages: oxplow's wiki (its files), a
/// documentation system a provider speaks to, or nowhere (none). A store
/// only keeps pages; the record — the page rows, their links and pins,
/// freshness and `knowledge.page.written` — is core's, made from what it
/// kept, the same for every store. The commands check a write (its slug,
/// its links) before it reaches the store.
#[async_trait]
pub trait KnowledgeProvider: Send + Sync {
    /// Its id: what `activeProviders` names (`oxplow`, `none`, an
    /// instance's).
    fn id(&self) -> &str;
    /// Whether it keeps nothing (none): it takes any page, so a delete or
    /// a link isn't refused for a page core doesn't record.
    fn sink(&self) -> bool {
        false
    }
    /// Write (create or replace) the page `draft.slug`; `None` when it
    /// keeps nothing.
    async fn write_page(
        &self,
        call: &PageCall<'_>,
        draft: &PageDraft,
    ) -> Result<Option<Kept>, CommandError>;
    /// Delete the page `slug`: the events it reports, `None` when it
    /// keeps nothing.
    async fn delete_page(
        &self,
        call: &PageCall<'_>,
        slug: &str,
    ) -> Result<Option<Vec<Envelope>>, CommandError>;
    /// Add a link to `target` (a ref: `wiki:other`, `file:src/lib.rs`)
    /// under the page's related links; `None` when it keeps nothing.
    async fn link(
        &self,
        call: &PageCall<'_>,
        slug: &str,
        target: &str,
    ) -> Result<Option<Kept>, CommandError>;
}

/// The knowledge implementations, by id, and the one a project uses now
/// (`active`: the capability registry's resolution; `none` is
/// registered like any other, a sink). A catalog reload restates the
/// declared ones ([`Self::set_declared`]) and keeps a registered instance
/// ([`Self::register`]).
pub struct KnowledgeRegistry {
    providers: RwLock<BTreeMap<String, Arc<dyn KnowledgeProvider>>>,
    declared: RwLock<BTreeSet<String>>,
    active: ActiveSource,
}

impl KnowledgeRegistry {
    pub fn new(active: ActiveSource) -> Self {
        Self {
            providers: RwLock::default(),
            declared: RwLock::default(),
            active,
        }
    }

    /// Register a running instance; a reload keeps it.
    pub fn register(&self, provider: Arc<dyn KnowledgeProvider>) {
        let id = provider.id().to_string();
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id);
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, provider);
    }

    pub fn unregister(&self, id: &str) {
        self.providers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        self.declared
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    /// The declared implementations are now `providers`: those declared
    /// before and not now go, these replace theirs, a registered instance
    /// stays.
    pub fn set_declared(&self, providers: Vec<Arc<dyn KnowledgeProvider>>) {
        let mut map = self.providers.write().unwrap_or_else(|e| e.into_inner());
        let mut declared = self.declared.write().unwrap_or_else(|e| e.into_inner());
        for id in std::mem::take(&mut *declared) {
            map.remove(&id);
        }
        for provider in providers {
            let id = provider.id().to_string();
            declared.insert(id.clone());
            map.insert(id, provider);
        }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn KnowledgeProvider>> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Whether `id` names one already (a provider's id-clash check).
    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    pub fn ids(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// The id the project's choice resolves to now.
    pub fn active_id(&self) -> String {
        (self.active)()
    }

    /// The active implementation; `None` only while the one chosen isn't
    /// registered (an instance between its stop and the switch).
    pub fn active(&self) -> Option<Arc<dyn KnowledgeProvider>> {
        self.get(&self.active_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named(&'static str);

    #[async_trait]
    impl KnowledgeProvider for Named {
        fn id(&self) -> &str {
            self.0
        }
        async fn write_page(
            &self,
            _: &PageCall<'_>,
            _: &PageDraft,
        ) -> Result<Option<Kept>, CommandError> {
            Ok(None)
        }
        async fn delete_page(
            &self,
            _: &PageCall<'_>,
            _: &str,
        ) -> Result<Option<Vec<Envelope>>, CommandError> {
            Ok(None)
        }
        async fn link(
            &self,
            _: &PageCall<'_>,
            _: &str,
            _: &str,
        ) -> Result<Option<Kept>, CommandError> {
            Ok(None)
        }
    }

    /// The active implementation follows the project's choice; a reload
    /// restates the declared ones and keeps a running instance.
    #[test]
    fn the_active_one_follows_the_choice_and_a_reload_keeps_instances() {
        let chosen = Arc::new(RwLock::new("oxplow".to_string()));
        let registry = KnowledgeRegistry::new({
            let chosen = chosen.clone();
            Arc::new(move || chosen.read().unwrap().clone())
        });
        registry.set_declared(vec![Arc::new(Named("oxplow")), Arc::new(Named("none"))]);
        registry.register(Arc::new(Named("vault")));
        assert_eq!(
            registry.active().map(|p| p.id().to_string()),
            Some("oxplow".into())
        );
        *chosen.write().unwrap() = "vault".into();
        assert_eq!(
            registry.active().map(|p| p.id().to_string()),
            Some("vault".into())
        );

        registry.set_declared(vec![Arc::new(Named("none"))]);
        assert_eq!(registry.ids(), ["none", "vault"]);
        registry.unregister("vault");
        assert!(registry.active().is_none(), "the chosen one is gone");
    }
}
