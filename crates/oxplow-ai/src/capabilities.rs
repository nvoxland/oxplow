//! Versioned, provider-neutral model capability declarations.

use serde::{Deserialize, Deserializer, Serialize};

/// A V1 envelope carried a schema version this binary cannot interpret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{envelope} schemaVersion {actual} is unsupported (expected {expected})")]
pub struct SchemaVersionError {
    pub envelope: &'static str,
    pub expected: u32,
    pub actual: u32,
}

pub(crate) fn check_v1(envelope: &'static str, actual: u32) -> Result<(), SchemaVersionError> {
    if actual == 1 {
        Ok(())
    } else {
        Err(SchemaVersionError {
            envelope,
            expected: 1,
            actual,
        })
    }
}

/// The knowledge a provider has about one model feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum CapabilitySupport {
    Unknown,
    Supported,
    Unsupported,
}

/// Features callers may discover before requesting a model operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ModelFeature {
    Text,
    Image,
    ToolCalling,
    Streaming,
}

/// A limit whose absent value means the provider does not know it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct KnownLimit {
    pub value: u64,
    pub unit: String,
}

/// Capabilities for a requested model. `model` is the requested identity;
/// `model_known` says whether the provider resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilitySnapshotV1 {
    pub schema_version: u32,
    pub model: Option<String>,
    pub model_known: bool,
    pub text: CapabilitySupport,
    pub image: CapabilitySupport,
    pub tool_calling: CapabilitySupport,
    pub streaming: CapabilitySupport,
    pub context_tokens: Option<KnownLimit>,
    pub output_tokens: Option<KnownLimit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CapabilitySnapshotV1Repr {
    schema_version: u32,
    model: Option<String>,
    model_known: bool,
    text: CapabilitySupport,
    image: CapabilitySupport,
    tool_calling: CapabilitySupport,
    streaming: CapabilitySupport,
    context_tokens: Option<KnownLimit>,
    output_tokens: Option<KnownLimit>,
}

impl<'de> Deserialize<'de> for CapabilitySnapshotV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error as _;
        let repr = CapabilitySnapshotV1Repr::deserialize(deserializer)?;
        check_v1("capability snapshot", repr.schema_version).map_err(D::Error::custom)?;
        Ok(Self {
            schema_version: repr.schema_version,
            model: repr.model,
            model_known: repr.model_known,
            text: repr.text,
            image: repr.image,
            tool_calling: repr.tool_calling,
            streaming: repr.streaming,
            context_tokens: repr.context_tokens,
            output_tokens: repr.output_tokens,
        })
    }
}

impl CapabilitySnapshotV1 {
    pub fn validate_version(&self) -> Result<(), SchemaVersionError> {
        check_v1("capability snapshot", self.schema_version)
    }

    /// A conservative snapshot for an older provider that has no discovery.
    pub fn unknown(model: &str) -> Self {
        Self {
            schema_version: 1,
            model: Some(model.to_string()),
            model_known: false,
            text: CapabilitySupport::Unknown,
            image: CapabilitySupport::Unknown,
            tool_calling: CapabilitySupport::Unknown,
            streaming: CapabilitySupport::Unknown,
            context_tokens: None,
            output_tokens: None,
        }
    }
}

/// Metadata the provider can enumerate, without probing a model endpoint.
/// An unknown catalog differs from a known, empty catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ModelCatalogV1 {
    pub schema_version: u32,
    pub catalog_known: bool,
    pub models: Vec<CapabilitySnapshotV1>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelCatalogV1Repr {
    schema_version: u32,
    catalog_known: bool,
    models: Vec<CapabilitySnapshotV1>,
}

impl<'de> Deserialize<'de> for ModelCatalogV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let value = ModelCatalogV1Repr::deserialize(deserializer)?;
        let catalog = Self {
            schema_version: value.schema_version,
            catalog_known: value.catalog_known,
            models: value.models,
        };
        catalog.validate().map_err(D::Error::custom)?;
        Ok(catalog)
    }
}

impl ModelCatalogV1 {
    pub fn unknown() -> Self {
        Self {
            schema_version: 1,
            catalog_known: false,
            models: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        check_v1("model catalog", self.schema_version).map_err(|e| e.to_string())?;
        let mut names = std::collections::BTreeSet::new();
        for model in &self.models {
            model.validate_version().map_err(|e| e.to_string())?;
            let name = model
                .model
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .ok_or("a catalog model needs an identity")?;
            if !model.model_known || !names.insert(name) {
                return Err("catalog models must be resolved and have unique identities".into());
            }
        }
        if !self.catalog_known && !self.models.is_empty() {
            return Err("an unknown catalog cannot enumerate resolved models".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn known(name: &str) -> CapabilitySnapshotV1 {
        CapabilitySnapshotV1 {
            model: Some(name.into()),
            model_known: true,
            text: CapabilitySupport::Supported,
            image: CapabilitySupport::Unsupported,
            context_tokens: Some(KnownLimit {
                value: 0,
                unit: "tokens".into(),
            }),
            ..CapabilitySnapshotV1::unknown(name)
        }
    }

    #[test]
    fn a_snapshot_round_trips_and_keeps_known_zero_apart_from_unknown() {
        let snap = known("m");
        let wire = serde_json::to_value(&snap).unwrap();
        assert_eq!(wire["schemaVersion"], 1);
        assert_eq!(wire["image"], "unsupported");
        assert_eq!(wire["toolCalling"], "unknown");
        assert_eq!(
            wire["contextTokens"],
            json!({ "value": 0, "unit": "tokens" })
        );
        assert!(wire["outputTokens"].is_null());
        let back: CapabilitySnapshotV1 = serde_json::from_value(wire).unwrap();
        assert_eq!(back, snap);
        assert_eq!(back.output_tokens, None);
    }

    #[test]
    fn the_conservative_snapshot_guesses_nothing() {
        let snap = CapabilitySnapshotV1::unknown("gpt-whatever");
        assert!(!snap.model_known);
        for support in [snap.text, snap.image, snap.tool_calling, snap.streaming] {
            assert_eq!(support, CapabilitySupport::Unknown);
        }
        assert!(snap.context_tokens.is_none() && snap.output_tokens.is_none());
    }

    #[test]
    fn an_unknown_snapshot_or_catalog_version_is_refused() {
        let mut wire = serde_json::to_value(known("m")).unwrap();
        wire["schemaVersion"] = json!(2);
        let err = serde_json::from_value::<CapabilitySnapshotV1>(wire.clone())
            .unwrap_err()
            .to_string();
        assert!(err.contains("capability snapshot schemaVersion 2"), "{err}");
        let catalog = json!({ "schemaVersion": 1, "catalogKnown": true, "models": [wire] });
        let err = serde_json::from_value::<ModelCatalogV1>(catalog)
            .unwrap_err()
            .to_string();
        assert!(err.contains("schemaVersion 2"), "{err}");
        let catalog = json!({ "schemaVersion": 3, "catalogKnown": false, "models": [] });
        let err = serde_json::from_value::<ModelCatalogV1>(catalog)
            .unwrap_err()
            .to_string();
        assert!(err.contains("model catalog schemaVersion 3"), "{err}");
    }

    #[test]
    fn an_unknown_field_or_support_value_is_refused() {
        let mut wire = serde_json::to_value(known("m")).unwrap();
        wire["vision"] = json!(true);
        assert!(serde_json::from_value::<CapabilitySnapshotV1>(wire).is_err());
        let mut wire = serde_json::to_value(known("m")).unwrap();
        wire["text"] = json!("maybe");
        assert!(serde_json::from_value::<CapabilitySnapshotV1>(wire).is_err());
    }

    /// An unknown catalog is not an empty one, and a catalog lists only
    /// resolved, uniquely named models.
    #[test]
    fn a_catalog_distinguishes_unknown_from_empty_and_checks_identities() {
        let unknown = ModelCatalogV1::unknown();
        assert!(!unknown.catalog_known && unknown.models.is_empty());
        assert!(unknown.validate().is_ok());
        let empty = ModelCatalogV1 {
            catalog_known: true,
            ..ModelCatalogV1::unknown()
        };
        assert_ne!(unknown, empty);
        assert!(empty.validate().is_ok());

        let listed = ModelCatalogV1 {
            schema_version: 1,
            catalog_known: true,
            models: vec![known("a"), known("b")],
        };
        let back: ModelCatalogV1 =
            serde_json::from_value(serde_json::to_value(&listed).unwrap()).unwrap();
        assert_eq!(back, listed);

        let dup = ModelCatalogV1 {
            models: vec![known("a"), known("a")],
            ..listed.clone()
        };
        assert!(dup.validate().is_err());
        let unresolved = ModelCatalogV1 {
            models: vec![CapabilitySnapshotV1::unknown("a")],
            ..listed.clone()
        };
        assert!(unresolved.validate().is_err());
        let nameless = ModelCatalogV1 {
            models: vec![CapabilitySnapshotV1 {
                model: None,
                ..known("a")
            }],
            ..listed.clone()
        };
        assert!(nameless.validate().is_err());
        let unknown_with_models = ModelCatalogV1 {
            catalog_known: false,
            ..listed
        };
        assert!(unknown_with_models.validate().is_err());
        // The same check guards deserialization.
        assert!(serde_json::from_value::<ModelCatalogV1>(
            serde_json::to_value(&unknown_with_models).unwrap()
        )
        .is_err());
    }
}
