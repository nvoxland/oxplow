//! Opaque identity for a host-owned extension secret. The reference carries
//! no secret bytes, namespace, lookup authority or storage location.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

/// A versioned reference. Only the host can create a fresh identity; a wire
/// reference conveys identity alone and must be resolved under host authority.
#[derive(Clone, PartialEq, Eq, Serialize, specta::Type, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretRef {
    #[schemars(range(min = 1, max = 1))]
    schema_version: u8,
    #[schemars(
        length(min = 36, max = 36),
        regex(pattern = "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
    )]
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SecretRefWire {
    schema_version: u8,
    id: String,
}

impl SecretRef {
    pub const SCHEMA_VERSION: u8 = 1;

    /// Allocate a host-owned reference without reading or storing a secret.
    pub fn generate() -> Self {
        Self {
            schema_version: Self::SCHEMA_VERSION,
            id: Uuid::now_v7().to_string(),
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }
}

impl<'de> Deserialize<'de> for SecretRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = SecretRefWire::deserialize(deserializer)
            .map_err(|_| serde::de::Error::custom("invalid secret reference"))?;
        let valid_id = wire.id.len() == 36
            && Uuid::parse_str(&wire.id).is_ok_and(|id| {
                id.get_version_num() == 7
                    && id.get_variant() == uuid::Variant::RFC4122
                    && id.to_string() == wire.id
            });
        if wire.schema_version != Self::SCHEMA_VERSION || !valid_id {
            return Err(serde::de::Error::custom("invalid secret reference"));
        }
        Ok(Self {
            schema_version: wire.schema_version,
            id: wire.id,
        })
    }
}

impl fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretRef")
    }
}
