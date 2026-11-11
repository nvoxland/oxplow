//! The knowledge capability (`.context/knowledge.md`): pages of durable
//! understanding — notes, memory, instructions — kept in one or more
//! stores. oxwiki (oxplow's own, a wiki of markdown files in
//! `.oxplow/oxwiki/`) is one; an extension's store (an Obsidian vault, a
//! documentation server) is another. Every enabled store is in use at
//! once, with no priority between them ([`KnowledgeStores`]): where a page
//! goes is the writer's choice, never oxplow's. Reads are SQL over
//! `v_knowledge_page`; writes go through the page's store, and core records
//! what it kept, the same for every store.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::events::Envelope;
use crate::{Actor, CommandError};

/// oxwiki's store id: what its pages' URIs carry (`store=oxwiki`) and
/// what a bare `[[slug]]` means.
pub const OXWIKI: &str = "oxwiki";

/// oxwiki's folder, relative to the project: one `<slug>.md` per page.
pub const OXWIKI_DIR: &str = ".oxplow/oxwiki";

/// What a page is for, as core acts on it: a store's kinds each map to one
/// (as a task list's stages map to buckets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Bucket {
    /// Understanding to read later: a design note, a walkthrough.
    Note,
    /// What an agent keeps for itself between sessions.
    Memory,
    /// What an agent is told to do: rules, conventions.
    Instruction,
}

impl Bucket {
    pub const ALL: [Bucket; 3] = [Bucket::Note, Bucket::Memory, Bucket::Instruction];

    pub fn as_str(self) -> &'static str {
        match self {
            Bucket::Note => "note",
            Bucket::Memory => "memory",
            Bucket::Instruction => "instruction",
        }
    }
}

/// One kind of page a store keeps: its own id and title, and the bucket
/// core files it under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KindDecl {
    pub id: String,
    pub title: String,
    pub bucket: Bucket,
}

impl KindDecl {
    /// A note: oxwiki's one kind, and a store's when it declares none.
    pub fn note() -> Self {
        Self {
            id: "note".into(),
            title: "Note".into(),
            bucket: Bucket::Note,
        }
    }
}

/// What a store supports beyond listing its pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct KnowledgeFeatures {
    /// Whether oxplow may write and delete its pages; a read-only store
    /// (instruction files, a harness's memory) refuses them.
    pub edit: bool,
}

impl Default for KnowledgeFeatures {
    /// A feature a store doesn't declare is off.
    fn default() -> Self {
        Self { edit: false }
    }
}

/// A page to write.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageDraft {
    /// The store's page id; `None`: the store derives one from the title
    /// (oxwiki slugifies it).
    pub id: Option<String>,
    /// Sets the page's heading.
    pub title: Option<String>,
    pub body: String,
    /// Files re-checked against this body: their pins move to now.
    pub verified_files: Vec<String>,
    /// Files taken out of the body.
    pub removed_files: Vec<String>,
}

/// How current one of a page's refs is: core's, from the pins it keeps for
/// every store (`v_knowledge_ref`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefFreshness {
    /// What the page relies on (`oxplow:oxplow/file?path=src/lib.rs`).
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
    /// The run it's part of: who writes, and where a provider's
    /// `host/call`s are counted.
    pub run: &'a crate::run::RunContext,
    /// The write's idempotency key (an effect's step's); `None` mints one
    /// where it's re-sent.
    pub idempotency_key: Option<String>,
}

impl PageCall<'_> {
    /// Who writes.
    pub fn actor(&self) -> &Actor {
        self.run.actor()
    }
}

/// What a store kept of a write: the page's id and body as they now stand
/// there (what core records, pins and links from), and the events it
/// reports of its own (a provider's `knowledge.page.recorded`).
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    /// The store's page id: the draft's, or the one it derived.
    pub id: String,
    pub body: String,
    /// Its title as the store has it; `None`: the body's first heading,
    /// else the id.
    pub title: Option<String>,
    /// The page's kind, one the store declares; `None`: its first.
    pub kind: Option<String>,
    /// Where to open it in the store's own app.
    pub url: Option<String>,
    /// Who last changed it, as the store names them.
    pub author: Option<String>,
    /// The refs it states for the page, in oxplow's grammar
    /// (`oxplow:oxplow/file?path=…`, `oxplow:oxplow/knowledge?id=…&store=…`):
    /// a provider whose body isn't oxplow's markdown says what it links.
    /// `None`: core parses the body (oxwiki's).
    pub refs: Option<Vec<String>>,
    pub events: Vec<Envelope>,
}

/// A knowledge store: where some of a project's pages are kept. A store
/// only keeps pages; the record — the page rows, their links and pins,
/// freshness and `knowledge.page.written` — is core's, made from what it
/// kept, the same for every store. The commands check a write (its id, its
/// links, the store's `edit`) before it reaches the store; adding a link is
/// core's, as a write of the page's body.
#[async_trait]
pub trait KnowledgeProvider: Send + Sync {
    /// Its id: what its pages' URIs carry (`oxwiki`, an instance's).
    fn id(&self) -> &str;
    /// The kinds of page it keeps; the first is a page's when the store
    /// says none.
    fn kinds(&self) -> Vec<KindDecl> {
        vec![KindDecl::note()]
    }
    fn features(&self) -> KnowledgeFeatures {
        KnowledgeFeatures::default()
    }
    /// The page whose file `path` (absolute) is, when its pages are files:
    /// an agent's read of that file is a read of the page.
    fn page_at(&self, _path: &Path) -> Option<String> {
        None
    }
    /// Write (create or replace) the page `draft.id` (or the one it derives
    /// from the title).
    async fn write_page(
        &self,
        call: &PageCall<'_>,
        draft: &PageDraft,
    ) -> Result<Kept, CommandError>;
    /// Delete the page `id`: the events it reports.
    async fn delete_page(
        &self,
        call: &PageCall<'_>,
        id: &str,
    ) -> Result<Vec<Envelope>, CommandError>;
}

/// The enabled knowledge stores, by id: every one is in use at once. A
/// catalog reload restates the declared ones ([`Self::set_declared`]) and
/// keeps a registered instance ([`Self::register`]).
#[derive(Default)]
pub struct KnowledgeStores {
    providers: RwLock<BTreeMap<String, Arc<dyn KnowledgeProvider>>>,
    declared: RwLock<BTreeSet<String>>,
}

impl KnowledgeStores {
    pub fn new() -> Self {
        Self::default()
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

    /// The declared stores are now `providers`: those declared before and
    /// not now go, these replace theirs, a registered instance stays.
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

    /// The enabled stores' ids, in order.
    pub fn enabled(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// The one enabled store, when exactly one is.
    pub fn only(&self) -> Option<Arc<dyn KnowledgeProvider>> {
        let map = self.providers.read().unwrap_or_else(|e| e.into_inner());
        (map.len() == 1)
            .then(|| map.values().next().cloned())
            .flatten()
    }

    /// The store a write names (`store`), or the only one when it names
    /// none. Refused naming the fix: a store that isn't enabled, no store
    /// enabled, or several to choose from.
    pub fn resolve(&self, store: Option<&str>) -> Result<Arc<dyn KnowledgeProvider>, CommandError> {
        let invalid = |message: String| CommandError::Invalid {
            field: Some("/store".into()),
            message,
        };
        let enabled = self.enabled();
        match store {
            Some(id) => self.get(id).ok_or_else(|| {
                invalid(if enabled.is_empty() {
                    format!("no knowledge store `{id}`: {}", none_enabled())
                } else {
                    format!(
                        "no knowledge store `{id}` is enabled; the enabled ones: {}",
                        enabled.join(", ")
                    )
                })
            }),
            None => match enabled.len() {
                0 => Err(invalid(none_enabled())),
                1 => self.get(&enabled[0]).ok_or_else(|| invalid(none_enabled())),
                _ => Err(invalid(format!("name the store: {}", enabled.join(", ")))),
            },
        }
    }
}

fn none_enabled() -> String {
    "no knowledge store is enabled: turn oxwiki on in Settings → Integrations → \
     Knowledge stores (or `oxplow.knowledge.set_oxwiki { on: true }`), or enable an \
     extension's store"
        .to_string()
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
        async fn write_page(&self, _: &PageCall<'_>, _: &PageDraft) -> Result<Kept, CommandError> {
            Err(CommandError::Failed {
                message: "unused".into(),
            })
        }
        async fn delete_page(
            &self,
            _: &PageCall<'_>,
            _: &str,
        ) -> Result<Vec<Envelope>, CommandError> {
            Ok(Vec::new())
        }
    }

    fn message(e: CommandError) -> String {
        match e {
            CommandError::Invalid { message, .. } => message,
            other => panic!("{other:?}"),
        }
    }

    /// Every registered store is enabled; a reload restates the declared
    /// ones and keeps a running instance.
    #[test]
    fn a_reload_restates_the_declared_stores_and_keeps_instances() {
        let stores = KnowledgeStores::new();
        stores.set_declared(vec![Arc::new(Named(OXWIKI))]);
        stores.register(Arc::new(Named("vault")));
        assert_eq!(stores.enabled(), [OXWIKI, "vault"]);
        stores.set_declared(Vec::new());
        assert_eq!(stores.enabled(), ["vault"]);
        stores.unregister("vault");
        assert!(stores.enabled().is_empty());
    }

    /// A write names its store, or takes the only one; with none or
    /// several it's refused, naming the fix.
    #[test]
    fn a_write_resolves_its_store_or_is_refused_naming_the_fix() {
        let stores = KnowledgeStores::new();
        assert!(message(stores.resolve(None).err().unwrap()).contains("Settings"));
        assert!(message(stores.resolve(Some("vault")).err().unwrap())
            .contains("no knowledge store is enabled"));

        stores.set_declared(vec![Arc::new(Named(OXWIKI))]);
        assert_eq!(stores.resolve(None).unwrap().id(), OXWIKI);
        assert!(stores.only().is_some());

        stores.register(Arc::new(Named("vault")));
        assert!(stores.only().is_none());
        assert_eq!(
            message(stores.resolve(None).err().unwrap()),
            "name the store: oxwiki, vault"
        );
        assert_eq!(stores.resolve(Some("vault")).unwrap().id(), "vault");
        assert_eq!(
            message(stores.resolve(Some("gone")).err().unwrap()),
            "no knowledge store `gone` is enabled; the enabled ones: oxwiki, vault"
        );
    }
}
