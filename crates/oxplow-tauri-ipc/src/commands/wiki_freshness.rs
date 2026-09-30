//! Wiki page freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file ref the wiki
//! page carries, from `v_knowledge_ref` (the one definition of
//! staleness), so the UI can render a per-ref staleness flag. Marking a ref verified is
//! `knowledge.write_page` with it in `verified_refs`.

pub use oxplow_rpc::commands::wiki_freshness::WikiRefFreshness;
