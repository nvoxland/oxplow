//! External providers (P5.D, `.context/providers.md`): programs that
//! implement a capability over the provider protocol.
//!
//! - [`spec`] — the `providers:` manifest kind and its declarations file;
//! - [`host`] — consent, the trusted spawn and the handshake;
//! - [`registry`] — the instances (`Services.providers`): reconciling
//!   them with `extensionInstances`, their bus commands, health, restart
//!   with backoff and automatic disable;
//! - [`endpoint`] — what a capability's adapter calls: an instance, or
//!   [`script`] — a script implementation an extension declares;
//! - [`tasks`] — `ExternalTaskTracker`, the task tracker capability over
//!   an endpoint;
//! - [`filesystems`] — `ExternalFileSystem`, a filesystem over an instance;
//! - [`sync`] — reading an instance's collectors (`oxplow.provider.sync`, the
//!   schedule), checkpointed.

pub mod agent_harness;
pub mod endpoint;
pub mod filesystems;
pub mod host;
pub mod knowledge;
pub mod model_records;
pub mod oauth;
pub mod registry;
pub mod script;
pub mod snapshots;
pub mod spec;
pub mod sync;
pub mod tasks;
pub mod worktree_tree;

#[cfg(test)]
mod script_capabilities;
#[cfg(test)]
mod tests;

pub use host::HostError;

pub use oauth::SignInState;
pub use registry::{BegunSignIn, SignInId};
pub use registry::{
    CollectorView, ConfigProblem, HostDeps, Instance, InstanceHealth, InstanceState,
    ProviderInstanceView, ProviderRegistry, Scope, SignInCompletion,
};
pub use spec::{parse_providers, ProviderSpec};
