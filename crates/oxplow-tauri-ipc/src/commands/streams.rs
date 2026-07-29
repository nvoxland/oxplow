//! Stream commands. The adapters are generated from the command table
//! in `oxplow-rpc` (see `commands::generated`); what remains here is the
//! re-export of the request types the renderer's bindings name.

pub use oxplow_rpc::commands::streams::{
    AdoptWorktreeRequest, CreateWorktreeRequest, RenameStreamRequest, SetStreamPromptRequest,
};
