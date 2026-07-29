//! Comments — threaded annotations anchored to a text selection on any
//! page. Each mutation emits `CommentsChanged` so the renderer (and any
//! other window) refetches the affected page's comments + the inbox.

pub use oxplow_rpc::commands::comments::CreateCommentRequest;
