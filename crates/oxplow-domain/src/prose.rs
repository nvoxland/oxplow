//! Audience variants for agent-authored prose.
//!
//! Every agent-authored prose body for the "big three" entities (wiki
//! page bodies, task descriptions, effort summaries) can carry three
//! audience variants:
//!
//! - [`ProseAudience::Developer`] — the detailed default the agent
//!   already writes. This stays the canonical source of truth in its
//!   existing column/file; the other two are purely additive.
//! - [`ProseAudience::Executive`] — a shorter executive-summary rewrite.
//! - [`ProseAudience::Terse`] — a terse, fragment-style rewrite
//!   (drop filler words, sentence fragments, keep technical terms /
//!   paths / code verbatim).
//!
//! The agent writes all three inline at author time; there is no
//! backend LLM. Storage keeps `developer` in its existing slot and
//! the optional `executive`/`terse` pair in a single nullable JSON
//! blob (`optional_json`), so existing rows degrade to developer-only
//! with no backfill.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Which audience variant of a prose body to read or display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ProseAudience {
    #[default]
    Developer,
    Executive,
    Terse,
}

/// The three audience variants of one prose body. `developer` is
/// always present (it is the canonical text); the other two are
/// optional and fall back to `developer` when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ProseVariants {
    pub developer: String,
    // No `skip_serializing_if` here: this type is exported through
    // specta/tauri-specta, whose unified mode rejects conditional
    // field omission. `None` serializes as `null`, which the frontend
    // treats the same as absent.
    #[serde(default)]
    pub executive: Option<String>,
    #[serde(default)]
    pub terse: Option<String>,
}

/// The non-developer variants as they are stored in a JSON column.
/// Developer text never lives here — it stays in its canonical
/// column/file — so `optional_json` round-trips only `executive` and
/// `terse`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct StoredVariants {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    executive: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terse: Option<String>,
}

impl ProseVariants {
    /// A developer-only body (no executive/terse variants).
    pub fn developer_only(developer: impl Into<String>) -> Self {
        Self {
            developer: developer.into(),
            executive: None,
            terse: None,
        }
    }

    /// Reconstruct from the canonical developer text plus the optional
    /// stored JSON blob (the `*_variants` column). A `None` or empty
    /// blob yields a developer-only body; malformed JSON is treated as
    /// absent rather than erroring — readers must always degrade to
    /// developer.
    pub fn from_developer_and_json(developer: impl Into<String>, json: Option<&str>) -> Self {
        let stored = json
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|s| serde_json::from_str::<StoredVariants>(s).ok())
            .unwrap_or_default();
        Self {
            developer: developer.into(),
            executive: stored.executive,
            terse: stored.terse,
        }
    }

    /// The body for `audience`, falling back to developer when the
    /// requested variant is absent or empty.
    pub fn get(&self, audience: ProseAudience) -> &str {
        let chosen = match audience {
            ProseAudience::Developer => return &self.developer,
            ProseAudience::Executive => self.executive.as_deref(),
            ProseAudience::Terse => self.terse.as_deref(),
        };
        match chosen {
            Some(s) if !s.is_empty() => s,
            _ => &self.developer,
        }
    }

    /// The non-developer variants serialized for storage in a
    /// `*_variants` JSON column, or `None` when both are absent (so
    /// the column stays NULL rather than holding `{}`).
    pub fn optional_json(&self) -> Option<String> {
        if self.executive.is_none() && self.terse.is_none() {
            return None;
        }
        let stored = StoredVariants {
            executive: self.executive.clone(),
            terse: self.terse.clone(),
        };
        serde_json::to_string(&stored).ok()
    }
}

/// Derive a stable section id from a markdown heading's text. Used to
/// anchor comments to a logical section that survives switching
/// between audience variants (the variants keep an aligned heading
/// skeleton). GitHub-flavored: lowercase, drop characters that are not
/// alphanumeric / space / hyphen, collapse whitespace runs to single
/// hyphens.
///
/// NOTE: this must stay in lockstep with the frontend `sectionSlug`
/// helper and react-markdown's heading-`id` rule (see the prose-variant
/// plan, Risks). Comment anchoring (phase 5) tests both against shared
/// fixtures.
pub fn heading_slug(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    let mut prev_hyphen = false;
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lower in ch.to_lowercase() {
                slug.push(lower);
            }
            prev_hyphen = false;
        } else if (ch.is_whitespace() || ch == '-' || ch == '_') && !prev_hyphen && !slug.is_empty()
        {
            slug.push('-');
            prev_hyphen = true;
        }
        // all other punctuation is dropped
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audience_round_trips_as_snake_case() {
        let json = serde_json::to_string(&ProseAudience::Terse).unwrap();
        assert_eq!(json, "\"terse\"");
        let back: ProseAudience = serde_json::from_str("\"executive\"").unwrap();
        assert_eq!(back, ProseAudience::Executive);
    }

    #[test]
    fn default_audience_is_developer() {
        assert_eq!(ProseAudience::default(), ProseAudience::Developer);
    }

    #[test]
    fn get_returns_requested_variant_when_present() {
        let v = ProseVariants {
            developer: "dev".into(),
            executive: Some("exec".into()),
            terse: Some("ugh".into()),
        };
        assert_eq!(v.get(ProseAudience::Developer), "dev");
        assert_eq!(v.get(ProseAudience::Executive), "exec");
        assert_eq!(v.get(ProseAudience::Terse), "ugh");
    }

    #[test]
    fn get_falls_back_to_developer_when_variant_absent_or_empty() {
        let v = ProseVariants {
            developer: "dev".into(),
            executive: None,
            terse: Some(String::new()),
        };
        assert_eq!(v.get(ProseAudience::Executive), "dev");
        assert_eq!(v.get(ProseAudience::Terse), "dev");
    }

    #[test]
    fn optional_json_is_none_when_no_variants() {
        let v = ProseVariants::developer_only("dev");
        assert_eq!(v.optional_json(), None);
    }

    #[test]
    fn optional_json_omits_developer_and_round_trips() {
        let v = ProseVariants {
            developer: "dev".into(),
            executive: Some("exec".into()),
            terse: None,
        };
        let json = v.optional_json().expect("some json");
        assert!(
            !json.contains("dev"),
            "developer text leaked into blob: {json}"
        );
        let back = ProseVariants::from_developer_and_json("dev", Some(&json));
        assert_eq!(back, v);
    }

    #[test]
    fn from_json_degrades_to_developer_on_missing_or_malformed() {
        assert_eq!(
            ProseVariants::from_developer_and_json("dev", None),
            ProseVariants::developer_only("dev")
        );
        assert_eq!(
            ProseVariants::from_developer_and_json("dev", Some("")),
            ProseVariants::developer_only("dev")
        );
        assert_eq!(
            ProseVariants::from_developer_and_json("dev", Some("{not json")),
            ProseVariants::developer_only("dev")
        );
    }

    #[test]
    fn heading_slug_is_github_flavored() {
        assert_eq!(heading_slug("Storage model"), "storage-model");
        assert_eq!(heading_slug("## Why we did it"), "why-we-did-it");
        assert_eq!(
            heading_slug("Per-page (per-tab) selector!"),
            "per-page-per-tab-selector"
        );
        assert_eq!(heading_slug("  Trailing  "), "trailing");
        assert_eq!(heading_slug("CRATES & tools"), "crates-tools");
    }
}
