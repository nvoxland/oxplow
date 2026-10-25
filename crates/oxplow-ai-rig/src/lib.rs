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

/// A Rig model handle for `model` on `instance` (its key and base URL,
/// else the declaration's) of a declared provider: what a resolved role
/// binding gives a caller that drives the model itself (the Assistant's
/// agent loop). Only a Rig kind has one: TypeSafe only decides, and a
/// host's provider (the fixture model) speaks no wire.
pub fn completion_model(
    provider: &dyn ModelProvider,
    instance: &ProviderInstance<'_>,
    model: &str,
) -> Result<DynModel<Completion>, AiError> {
    provider
        .as_any()
        .and_then(|any| any.downcast_ref::<RigProvider>())
        .ok_or_else(|| AiError::BadRequest {
            provider: instance.id.into(),
            detail: format!(
                "the `{}` provider can't drive an agent: it isn't a kind Rig speaks",
                provider.kind()
            ),
        })?
        .completion_model(instance, model)
}

#[cfg(test)]
mod tests;
