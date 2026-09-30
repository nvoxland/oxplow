//! Wiki page freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file/directory
//! ref the wiki page carries, joining the captured snapshot pin on
//! `page_ref` with the latest `file_snapshot` for that path so the
//! UI can render a per-ref staleness flag. Marking a ref verified is
//! `knowledge.write_page` with it in `verified_refs`.

pub use oxplow_rpc::commands::wiki_freshness::WikiRefFreshness;
