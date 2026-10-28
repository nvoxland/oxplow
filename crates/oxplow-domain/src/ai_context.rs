//! Guidance for an agent reading the semantic layer: the optional
//! `ai_context:` block on a model, a model's column, a metric and a
//! dimension. It says when to use the thing and how to read it; it is
//! never part of a contract, so editing it never forces a version bump.

use serde::{Deserialize, Serialize};

/// The `ai_context:` block. All keys are optional.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct AiContext {
    /// When to use this and how to read it.
    #[serde(default)]
    pub instructions: Option<String>,
    /// Other names a person or agent may call it.
    #[serde(default)]
    pub synonyms: Vec<String>,
    /// Example questions it answers.
    #[serde(default)]
    pub examples: Vec<String>,
}

impl AiContext {
    /// The JSON text stored in an `ai_context_json` column, with only the
    /// keys set; `None` when there is no block.
    pub fn to_column(ctx: &Option<AiContext>) -> Option<String> {
        ctx.as_ref().map(|c| c.to_value().to_string())
    }

    /// This block as JSON with only the keys set.
    pub fn to_value(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        if let Some(i) = &self.instructions {
            map.insert("instructions".into(), i.clone().into());
        }
        if !self.synonyms.is_empty() {
            map.insert("synonyms".into(), self.synonyms.clone().into());
        }
        if !self.examples.is_empty() {
            map.insert("examples".into(), self.examples.clone().into());
        }
        serde_json::Value::Object(map)
    }

    /// The block read back from an `ai_context_json` column.
    pub fn from_column(text: Option<String>) -> Option<AiContext> {
        text.and_then(|t| serde_json::from_str(&t).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_is_optional_and_an_unknown_one_is_refused() {
        let all: AiContext = serde_yaml::from_str(
            "instructions: Use for open work.\nsynonyms: [tickets]\nexamples: [open items]\n",
        )
        .unwrap();
        assert_eq!(all.synonyms, ["tickets"]);
        let none: AiContext = serde_yaml::from_str("{}").unwrap();
        assert_eq!(none, AiContext::default());
        let err = serde_yaml::from_str::<AiContext>("hint: no").unwrap_err();
        assert!(err.to_string().contains("unknown field `hint`"), "{err}");
    }
}
