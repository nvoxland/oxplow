//! External providers (P5.D, `.context/providers.md`): programs that
//! implement a capability over the provider protocol.
//!
//! - [`spec`] — the `providers:` manifest kind and its declarations file;
//! - [`host`] — consent, the trusted spawn and the handshake;
//! - [`registry`] — the enabled instances (`Services.providers`), their
//!   bus commands and restart with backoff;
//! - [`work_items`] — `ExternalWorkItems`, the work-items capability over
//!   an instance.

pub mod host;
pub mod registry;
pub mod spec;
pub mod work_items;

#[cfg(test)]
mod tests;

pub use host::HostError;
pub use registry::{HostDeps, Instance, ProviderRegistry};
pub use spec::{parse_providers, ProviderSpec};
