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

use oxplow_ai::client::{AiError, ModelProvider, ProviderInstance};

mod convert;
mod errors;
pub mod jev;
mod kinds;
mod provider;

pub use kinds::{kind, names, Kind, UnknownKind, KINDS};
pub use provider::RigProvider;
/// What a caller driving a model itself needs: the handle, its
/// operation, and Rig's request and stream types.
pub use rig_core;
pub use rig_core::operation::Completion;
pub use rig_core::DynModel;

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

/// A Rig model handle for `model` of the Rust kind `kind` on `instance`
/// (its key and base URL, else `default_base_url`): what a resolved role
/// binding gives a caller that drives the model itself (an agent loop).
/// TypeSafe has none: it only decides.
pub fn completion_model(
    kind: &str,
    default_base_url: Option<&str>,
    instance: &ProviderInstance<'_>,
    model: &str,
) -> Result<DynModel<Completion>, AiError> {
    let found = kinds::kind(kind).map_err(|e| AiError::BadRequest {
        provider: instance.id.into(),
        detail: e.to_string(),
    })?;
    RigProvider::new(found, kind, kind, default_base_url).completion_model(instance, model)
}

#[cfg(test)]
mod tests;
