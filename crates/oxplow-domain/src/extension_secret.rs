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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_generated_reference_round_trips_in_its_camel_case_shape() {
        let reference = SecretRef::generate();
        let wire = serde_json::to_value(&reference).unwrap();
        assert_eq!(wire, json!({"schemaVersion": 1, "id": reference.id()}));
        let back: SecretRef = serde_json::from_value(wire).unwrap();
        assert_eq!(back, reference);
        assert_ne!(SecretRef::generate(), reference, "each reference is fresh");
    }

    #[test]
    fn the_wire_shape_is_strict() {
        let id = SecretRef::generate().id().to_owned();
        let parse = |value: serde_json::Value| serde_json::from_value::<SecretRef>(value);
        assert!(parse(json!({"schemaVersion": 1, "id": id})).is_ok());

        // Unknown, missing and snake_case fields.
        assert!(parse(json!({"schemaVersion": 1, "id": id, "value": "hunter2"})).is_err());
        assert!(parse(json!({"schemaVersion": 1})).is_err());
        assert!(parse(json!({"id": id})).is_err());
        assert!(parse(json!({"schema_version": 1, "id": id})).is_err());
        // Wrong version.
        assert!(parse(json!({"schemaVersion": 2, "id": id})).is_err());
        assert!(parse(json!({"schemaVersion": 0, "id": id})).is_err());
        // The id must be a canonical lowercase v7 UUID.
        for bad in [
            "".to_string(),
            "not-a-uuid".to_string(),
            id.to_uppercase(),
            id.replace('-', ""),
            format!("{{{id}}}"),
            "550e8400-e29b-41d4-a716-446655440000".into(), // a v4
            "00000000-0000-0000-0000-000000000000".into(),
        ] {
            assert!(
                parse(json!({"schemaVersion": 1, "id": bad})).is_err(),
                "{bad} must be refused"
            );
        }
        // Types are exact, and a bare string is not a reference.
        assert!(parse(json!({"schemaVersion": "1", "id": id})).is_err());
        assert!(parse(json!({"schemaVersion": 1, "id": 7})).is_err());
        assert!(parse(json!(id)).is_err());
        assert!(parse(json!(null)).is_err());
    }

    #[test]
    fn a_refusal_does_not_echo_the_input() {
        let error = serde_json::from_value::<SecretRef>(
            json!({"schemaVersion": 1, "id": "hunter2-not-an-id"}),
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("hunter2"), "{error}");
    }

    #[test]
    fn debug_output_names_the_type_and_nothing_else() {
        let reference = SecretRef::generate();
        let debug = format!("{reference:?} {reference:#?}");
        assert!(!debug.contains(reference.id()), "{debug}");
        assert_eq!(format!("{reference:?}"), "SecretRef");
        // Nor does a container that derives Debug leak it.
        let wrapped = format!("{:?}", Some(&reference));
        assert!(!wrapped.contains(reference.id()), "{wrapped}");
    }
}
