//! Wiki page freshness reader.
//!
//! `list_wiki_freshness(slug)` returns one row per file/directory
//! ref the wiki page carries, joining the captured snapshot pin on
//! `page_ref` with the latest `file_snapshot` for that path so the
//! UI can render a per-ref staleness flag. `mark_wiki_ref_verified`
//! and `mark_all_wiki_refs_verified` re-stamp the pin to the
//! current resolved version when the user explicitly confirms the
//! page is still accurate.

pub use oxplow_rpc::commands::wiki_freshness::WikiRefFreshness;
