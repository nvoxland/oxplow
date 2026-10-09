//! External providers (P5.D, `.context/providers.md`): programs that
//! implement a capability over the provider protocol.
//!
//! - [`spec`] — the `providers:` manifest kind and its declarations file;
//! - [`host`] — consent, the trusted spawn and the handshake;
//! - [`registry`] — the instances (`Services.providers`): reconciling
//!   them with `extensionInstances`, their bus commands, health, restart
//!   with backoff and automatic disable;
//! - [`work_items`] — `ExternalWorkItems`, the work-items capability over
//!   an instance;
//! - [`effort_policy`] — `ExternalEffortPolicy`, the effort policy over an
//!   instance;
//! - [`sync`] — reading an instance's collectors (`oxplow.provider.sync`, the
//!   schedule), checkpointed.

pub mod effort_policy;
pub mod host;
pub mod oauth;
pub mod registry;
pub mod spec;
pub mod sync;
pub mod work_items;

#[cfg(test)]
mod tests;

pub use host::HostError;

pub use registry::{BegunSignIn, SignInId};
pub use registry::{
    CollectorView, ConfigProblem, HostDeps, Instance, InstanceHealth, InstanceState,
    ProviderInstanceView, ProviderRegistry, Scope, SignInCompletion,
};
pub use spec::{parse_providers, ProviderSpec};
