//! Unified cross-page reference graph reader.
//!
//! Both directions of the edge are exposed:
//! - `list_backlinks(target)` — what points AT this page (its URI).
//! - `list_outbound(source)` — what this page points to.
//!
//! The reader joins labels (wiki title, task title, commit subject) at
//! read time; when the other end is gone the label is `None` and the
//! renderer falls back to its URI.

pub use oxplow_rpc::commands::page_refs::BacklinkEdge;
