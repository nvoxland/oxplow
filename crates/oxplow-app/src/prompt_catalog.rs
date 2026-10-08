//! What a person can ask: every area's answerability questions
//! (`oxplow_agent_text::answerability_prompts`) and every enabled
//! extension's `intent.prompts`, in one list. The catalog page groups it
//! by source; a page for a ref shows the prompts `about` that ref's kind.
//! See `.context/extensions.md` → "The prompt catalog".

use crate::extensions::Extension;

/// One question the person can ask, with where it comes from.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CatalogPrompt {
    pub prompt: String,
    /// A ref kind (`file`, `commit`, `effort`): a page for one offers it.
    pub about: Option<String>,
    pub source: PromptSource,
}

/// Who offers a prompt.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", tag = "kind", content = "name")]
pub enum PromptSource {
    /// One of core's areas (`vcs`, `work_items`, `knowledge`,
    /// `code_intel`, `extensions`).
    Area(String),
    /// An extension.
    Extension(String),
}

/// Core's prompts, then each enabled extension's, in name order.
pub fn prompt_catalog(extensions: &[Extension]) -> Vec<CatalogPrompt> {
    let core = oxplow_agent_text::answerability_prompts()
        .into_iter()
        .map(|p| CatalogPrompt {
            prompt: p.prompt,
            about: p.about,
            source: PromptSource::Area(p.area.to_string()),
        });
    let mut exts: Vec<&Extension> = extensions.iter().filter(|e| e.enabled).collect();
    exts.sort_by(|a, b| a.name.cmp(&b.name));
    let from_extensions = exts.into_iter().flat_map(|e| {
        e.intent
            .iter()
            .flat_map(|i| i.prompts.iter())
            .map(|p| CatalogPrompt {
                prompt: p.prompt.clone(),
                about: p.about.clone(),
                source: PromptSource::Extension(e.name.clone()),
            })
    });
    core.chain(from_extensions).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::{Intent, IntentPrompt};

    fn ext(name: &str, enabled: bool, prompts: &[(&str, Option<&str>)]) -> Extension {
        let mut e = crate::extensions::empty_extension(name, name, "project");
        e.enabled = enabled;
        e.intent = Some(Intent {
            purpose: "p".into(),
            origin: None,
            examples: vec![],
            prompts: prompts
                .iter()
                .map(|(p, a)| IntentPrompt {
                    prompt: (*p).into(),
                    about: a.map(str::to_string),
                })
                .collect(),
        });
        e
    }

    #[test]
    fn the_catalog_merges_core_and_enabled_extension_prompts() {
        let catalog = prompt_catalog(&[
            ext(
                "gh",
                true,
                &[
                    ("Which PRs wait on me?", None),
                    ("Who reviews this commit?", Some("commit")),
                ],
            ),
            ext("off", false, &[("Hidden?", None)]),
        ]);
        assert!(catalog
            .iter()
            .any(|p| p.prompt == "Who has changed this file the most?"
                && p.about.as_deref() == Some("file")
                && p.source == PromptSource::Area("vcs".into())));
        let mine: Vec<_> = catalog
            .iter()
            .filter(|p| p.source == PromptSource::Extension("gh".into()))
            .map(|p| (p.prompt.as_str(), p.about.as_deref()))
            .collect();
        assert_eq!(
            mine,
            vec![
                ("Which PRs wait on me?", None),
                ("Who reviews this commit?", Some("commit"))
            ]
        );
        assert!(!catalog.iter().any(|p| p.prompt == "Hidden?"));
        // Core first.
        assert!(matches!(catalog[0].source, PromptSource::Area(_)));
    }
}
