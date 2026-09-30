//! The knowledge capability (P5.C4, `.context/knowledge.md`): pages of
//! durable understanding and what they rely on. oxplow's wiki is the
//! built-in provider; a documentation system could be another. Reads are
//! SQL over `v_knowledge_page`; writes and freshness go through a
//! provider.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::Actor;

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

/// How current one of a page's refs is. Freshness is the provider's: a
/// provider that can't pin to snapshots reports what it can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefFreshness {
    /// What the page relies on (`file:src/lib.rs`).
    pub target: String,
    /// The snapshot the ref was pinned to, if any.
    pub pinned_snapshot: Option<i64>,
    /// The target's latest snapshot, if it has one.
    pub latest_snapshot: Option<i64>,
    /// The target changed since it was pinned (or was never pinned but
    /// has been captured).
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KnowledgeError {
    /// The write was refused: a bad slug, a dangling link, a ref the body
    /// doesn't bear out. The message names what.
    #[error("{0}")]
    Refused(String),
    #[error("{0}")]
    Failed(String),
}

/// One source of knowledge pages.
#[async_trait]
pub trait KnowledgeProvider: Send + Sync {
    /// The provider's name (`oxplow`).
    fn provider(&self) -> &str;
    /// Write (create or replace) a page; its ref (`wiki:<slug>`).
    async fn write_page(&self, actor: &Actor, draft: PageDraft) -> Result<String, KnowledgeError>;
    async fn delete_page(&self, actor: &Actor, page: &str) -> Result<(), KnowledgeError>;
    /// Link `page` to `target` (another page, a file, a task, …).
    async fn link(&self, actor: &Actor, page: &str, target: &str) -> Result<(), KnowledgeError>;
    /// How current each of `page`'s pinned refs is.
    async fn freshness(&self, page: &str) -> Result<Vec<RefFreshness>, KnowledgeError>;
}
