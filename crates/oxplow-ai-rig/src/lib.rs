//! oxplow's model providers (`.context/ai-providers.md` "Providers"): the
//! Rust kinds an `ai_provider` declaration names with `provider:`. Each
//! kind Rig speaks is a [`RigProvider`] — oxplow's
//! [`oxplow_ai::client::ModelProvider`] over Rig (`rig-core`), one
//! maintained multi-provider layer instead of a request mapping per
//! vendor of our own. `typesafe` (Jev's typed decisions) is
//! [`jev::TypeSafe`], since Rig doesn't speak it.
//!
//! Keys always come from the call's [`ProviderInstance`] (the keychain),
//! never from Rig's environment lookup; the base URL is the instance's,
//! else the declaration's.

use std::sync::Arc;

use oxplow_ai::client::ModelProvider;

mod convert;
mod errors;
pub mod jev;
mod kinds;
mod provider;

pub use kinds::{kind, names, Kind, UnknownKind, KINDS};
pub use provider::RigProvider;
/// The provider an `ai_provider` declaration names: its declared `id`
/// (what `ai.yaml`'s `kind:` names), `title`, default base URL, and the
/// Rust kind it is.
pub fn provider(
    kind: &str,
    id: &str,
    title: &str,
    default_base_url: Option<&str>,
) -> Result<Arc<dyn ModelProvider>, UnknownKind> {
    let found = kinds::kind(kind)?;
    Ok(match found.name {
        "typesafe" => Arc::new(jev::TypeSafe::new(id, title, default_base_url)),
        _ => Arc::new(RigProvider::new(found, id, title, default_base_url)),
    })
}

#[cfg(test)]
mod tests;
