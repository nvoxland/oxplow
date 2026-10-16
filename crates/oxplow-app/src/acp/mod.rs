//! ACP agents: which ones a project can run (`agents`) and the connection
//! to one (`wire`, the only module that names ACP SDK types). The chat
//! session machinery on top of it is `crate::chat`. See
//! `.context/agent-model.md` → "ACP agents: configuration".

pub mod agents;
pub mod wire;
