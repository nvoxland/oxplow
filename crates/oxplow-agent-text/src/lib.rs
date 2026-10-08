//! The text oxplow gives every agent: core's skills and slash commands
//! ([`core_text`]), and the capability questions an agent should be able
//! to answer ([`CAPABILITY_QUESTIONS`], [`capability_prompts`]). Each
//! harness (`oxplow-harnesses`) writes it into its own runtime.

use oxplow_domain::agent::text::{AgentText, Text};

/// The capability answerability questions (`assets/questions/<capability>.yaml`,
/// P5.F1): what an agent should be able to answer, the skill that should
/// lead it there, and what it reaches. `oxplow_sdk::answerability` checks
/// them.
pub const CAPABILITY_QUESTIONS: &[(&str, &str)] = &[
    ("vcs", include_str!("../assets/questions/vcs.yaml")),
    (
        "work_items",
        include_str!("../assets/questions/work_items.yaml"),
    ),
    (
        "knowledge",
        include_str!("../assets/questions/knowledge.yaml"),
    ),
    (
        "code_intel",
        include_str!("../assets/questions/code_intel.yaml"),
    ),
    (
        "extensions",
        include_str!("../assets/questions/extensions.yaml"),
    ),
];

/// A capability question as the person sees it (P6.D2): offered with an
/// Ask button on the catalog page, and on a page for a ref of `about`'s
/// kind (`file`, `commit`, `effort`, `work_item`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityPrompt {
    pub capability: &'static str,
    pub prompt: String,
    pub about: Option<String>,
}

/// Every capability question, in file order. The files' other keys are
/// the answerability check's (`oxplow_sdk::answerability`).
pub fn capability_prompts() -> Vec<CapabilityPrompt> {
    #[derive(serde::Deserialize)]
    struct Entry {
        question: String,
        #[serde(default)]
        about: Option<String>,
    }
    CAPABILITY_QUESTIONS
        .iter()
        .flat_map(|(capability, yaml)| {
            serde_yaml::from_str::<Vec<Entry>>(yaml)
                .expect("a bundled questions file parses (checked by its test)")
                .into_iter()
                .map(|e| CapabilityPrompt {
                    capability,
                    prompt: e.question,
                    about: e.about,
                })
        })
        .collect()
}

/// Core's own skills and commands.
pub fn core_text() -> AgentText {
    let text = |list: &[(&str, &str)]| {
        list.iter()
            .map(|(name, body)| Text {
                name: (*name).into(),
                body: (*body).into(),
            })
            .collect()
    };
    AgentText {
        skills: text(OXPLOW_SKILLS),
        commands: text(CORE_COMMANDS),
    }
}

/// Core's slash commands, as `(name, markdown)`: `/oxplow:<name>`.
const CORE_COMMANDS: &[(&str, &str)] = &[
    (
        "review-comments",
        include_str!("../assets/review-comments.md"),
    ),
    ("configure", include_str!("../assets/configure.md")),
    ("new-metric", include_str!("../assets/new-metric.md")),
];

/// The oxplow skills every agent runtime ships, as `(dir_name, SKILL.md body)`
/// pairs. The dir name must match the frontmatter `name:` — both Claude and
/// opencode key discovery on it.
const OXPLOW_SKILLS: &[(&str, &str)] = &[
    (
        "oxplow-runtime",
        include_str!("../assets/oxplow-runtime.SKILL.md"),
    ),
    (
        "oxplow-wiki-capture",
        include_str!("../assets/oxplow-wiki-capture.SKILL.md"),
    ),
    (
        "oxplow-mermaid",
        include_str!("../assets/oxplow-mermaid.SKILL.md"),
    ),
    (
        "oxplow-collection",
        include_str!("../assets/oxplow-collection.SKILL.md"),
    ),
    (
        "oxplow-metrics",
        include_str!("../assets/oxplow-metrics.SKILL.md"),
    ),
    (
        "oxplow-extension",
        include_str!("../assets/oxplow-extension.SKILL.md"),
    ),
    (
        "oxplow-codebase",
        include_str!("../assets/oxplow-codebase.SKILL.md"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// P6.D2: each capability question is a prompt; an `about` names a
    /// registered kind of ref, and the entity pages have some.
    #[test]
    fn capability_prompts_are_about_registered_ref_kinds() {
        let prompts = capability_prompts();
        let kinds = oxplow_domain::refs::kind::core_kinds();
        for p in &prompts {
            if let Some(about) = &p.about {
                assert!(kinds.get(about).is_some(), "{}: `{about}`", p.prompt);
            }
        }
        for kind in ["file", "commit", "effort", "work_item"] {
            assert!(
                prompts.iter().any(|p| p.about.as_deref() == Some(kind)),
                "no prompt about `{kind}`"
            );
        }
        assert!(prompts
            .iter()
            .any(|p| p.capability == "vcs" && p.prompt == "Who has changed this file the most?"));
    }

    /// Agents that can't discover skill files get an index instead
    /// (tsk376): every skill, with its frontmatter description.
    #[test]
    fn the_skill_index_names_every_skill_with_its_description() {
        let text = core_text();
        let index = text.skill_index();
        assert_eq!(index.len(), OXPLOW_SKILLS.len());
        let (name, description) = index
            .iter()
            .find(|(n, _)| *n == "oxplow-extension")
            .unwrap();
        assert_eq!(*name, "oxplow-extension");
        assert!(
            description.starts_with("Build oxplow lenses"),
            "{description}"
        );
        assert!(index.iter().all(|(_, d)| !d.is_empty()));
        assert!(text
            .skill_body("oxplow-extension")
            .unwrap()
            .contains("# Building oxplow lenses"));
        assert_eq!(text.skill_body("nope"), None);
    }

    /// `.context/` is this repo's own docs convention: it must never leak
    /// into the text oxplow writes into a person's project.
    #[test]
    fn the_agent_text_never_mentions_dot_context() {
        let text = core_text();
        for t in text.skills.iter().chain(&text.commands) {
            assert!(!t.body.contains(".context"), "{} mentions .context", t.name);
        }
    }
}
