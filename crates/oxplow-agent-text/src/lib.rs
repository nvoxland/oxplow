//! The text oxplow gives every agent: core's skills and slash commands
//! ([`core_text`]), and the answerability questions an agent should be
//! able to answer, by area ([`ANSWERABILITY_QUESTIONS`],
//! [`answerability_prompts`]). Each
//! harness (`oxplow-harnesses`) writes it into its own runtime.

use oxplow_domain::agent::text::{AgentText, Text};

/// The answerability questions, one file per area of oxplow
/// (`assets/questions/<area>.yaml`): what an agent should be able to
/// answer, the skill that should lead it there, and what it reaches.
/// `oxplow_sdk::answerability` checks them.
/// What every agent must know of oxplow, whatever project it's in: oxplow
/// adds it to the prompt of every session it launches (terminal and ACP),
/// so no project repeats it in its own instruction files. The detail is in
/// the skills, which load on demand.
pub const WORKING_IN_OXPLOW: &str = include_str!("../assets/working-in-oxplow.md");

pub const ANSWERABILITY_QUESTIONS: &[(&str, &str)] = &[
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

/// An answerability question as the person sees it: offered with an
/// Ask button on the catalog page, and on a page for a ref of `about`'s
/// kind (`file`, `commit`, `effort`, `work_item`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerabilityPrompt {
    /// The area it's about (`vcs`, `work_items`, `knowledge`, …).
    pub area: &'static str,
    pub prompt: String,
    pub about: Option<String>,
}

/// Every answerability question, in file order. The files' other keys are
/// the answerability check's (`oxplow_sdk::answerability`).
pub fn answerability_prompts() -> Vec<AnswerabilityPrompt> {
    #[derive(serde::Deserialize)]
    struct Entry {
        question: String,
        #[serde(default)]
        about: Option<String>,
    }
    ANSWERABILITY_QUESTIONS
        .iter()
        .flat_map(|(area, yaml)| {
            serde_yaml::from_str::<Vec<Entry>>(yaml)
                .expect("a bundled questions file parses (checked by its test)")
                .into_iter()
                .map(|e| AnswerabilityPrompt {
                    area,
                    prompt: e.question,
                    about: e.about,
                })
        })
        .collect()
}

/// Core's own skills and commands.
pub fn core_text() -> AgentText {
    AgentText {
        skills: OXPLOW_SKILLS
            .iter()
            .map(|(name, body, references)| Text {
                name: (*name).into(),
                body: (*body).into(),
                references: references
                    .iter()
                    .map(|(reference, body)| Text {
                        name: (*reference).into(),
                        body: (*body).into(),
                        references: Vec::new(),
                    })
                    .collect(),
            })
            .collect(),
        commands: CORE_COMMANDS
            .iter()
            .map(|(name, body)| Text {
                name: (*name).into(),
                body: (*body).into(),
                references: Vec::new(),
            })
            .collect(),
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

/// One skill: its `SKILL.md` and the reference files beside it
/// (`assets/skills/<name>/references/<reference>.md`).
/// `(dir_name, SKILL.md body, references)`.
type SkillFiles = (
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

macro_rules! skill {
    ($name:literal, [$($reference:literal),* $(,)?]) => {
        (
            $name,
            include_str!(concat!("../assets/skills/", $name, "/SKILL.md")),
            &[$((
                $reference,
                include_str!(concat!("../assets/skills/", $name, "/references/", $reference, ".md")),
            )),*] as &[(&str, &str)],
        )
    };
}

/// The oxplow skills every agent runtime ships, as `(dir_name, SKILL.md
/// body, references)`. The dir name must match the frontmatter `name:` —
/// both Claude and opencode key discovery on it. A `SKILL.md` stays short
/// (`SKILL_BODY_LIMIT`): loading one costs the whole file, so the detail
/// lives in references the agent opens when it needs them.
const OXPLOW_SKILLS: &[SkillFiles] = &[
    skill!("oxplow-runtime", ["efforts", "commands"]),
    skill!("oxplow-wiki-capture", ["search-existing", "write-page"]),
    skill!("oxplow-mermaid", ["syntax"]),
    skill!("oxplow-collection", ["custom-parsers"]),
    skill!(
        "oxplow-metrics",
        [
            "declaring",
            "collector-scripts",
            "verify-and-read",
            "entity-metrics"
        ]
    ),
    skill!(
        "oxplow-extension",
        [
            "scaffolding",
            "semantic-layer",
            "lens-format",
            "ui-contributions",
            "commands-and-forms",
            "collectors",
            "metrics",
            "jobs-and-ai",
            "check",
            "testing",
            "providers",
            "lenses-in-use",
            "repair-and-sharing",
            "reading-data"
        ]
    ),
    skill!("oxplow-codebase", ["code", "history"]),
];

/// The most a `SKILL.md` may be, in bytes.
pub const SKILL_BODY_LIMIT: usize = 4096;

#[cfg(test)]
mod tests {
    use super::*;

    /// Each answerability question is a prompt; an `about` names a
    /// registered kind of ref, and the entity pages have some.
    #[test]
    fn answerability_prompts_are_about_registered_ref_kinds() {
        let prompts = answerability_prompts();
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
            .any(|p| p.area == "vcs" && p.prompt == "Who has changed this file the most?"));
    }

    /// "Can you see that error?": an operation that failed in front of the
    /// person is recorded (`v_op_error`, its output `read_event_content`),
    /// and the agent is told so — in the always-on text, and in the skill
    /// that loads when the person asks about an error.
    #[test]
    fn the_agent_is_told_it_can_read_the_errors_a_person_saw() {
        assert!(
            WORKING_IN_OXPLOW.contains("v_op_error"),
            "{WORKING_IN_OXPLOW}"
        );
        let text = core_text();
        let skill = text
            .skills
            .iter()
            .find(|s| s.name == "oxplow-codebase")
            .expect("the codebase skill");
        let all = text.skill_full_text("oxplow-codebase").unwrap();
        for needle in ["v_op_error", "`oxplow_read`", "output_size", "red"] {
            assert!(all.contains(needle), "the skill never says `{needle}`");
        }
        let description = skill.body.lines().nth(2).unwrap_or_default();
        assert!(description.contains("v_op_error"), "{description}");
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

    /// Whether `text` has a `tsk<digits>` word in it.
    fn names_a_task(text: &str) -> bool {
        text.match_indices("tsk").any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let digits = text[at + 3..]
                .chars()
                .take_while(char::is_ascii_digit)
                .count();
            digits > 0 && !before.is_some_and(char::is_alphanumeric)
        })
    }

    /// Loading a skill costs its whole file: each `SKILL.md` stays short
    /// and sends the agent to its references for detail, every reference
    /// a skill names exists, and a skill's text carries no task id (they're
    /// local to one project's tracker).
    #[test]
    fn skills_are_short_and_send_the_agent_to_references_that_exist() {
        for skill in core_text().skills {
            assert!(
                skill.body.len() <= SKILL_BODY_LIMIT,
                "{} is {} bytes (limit {SKILL_BODY_LIMIT}); move detail into references/",
                skill.name,
                skill.body.len()
            );
            for t in std::iter::once(&skill).chain(&skill.references) {
                assert!(!names_a_task(&t.body), "{} names a task id", t.name);
            }
            for reference in &skill.references {
                assert!(
                    skill
                        .body
                        .contains(&format!("references/{}.md", reference.name)),
                    "{} never sends the agent to `references/{}.md`",
                    skill.name,
                    reference.name
                );
            }
        }
    }

    /// `.context/` is this repo's own docs convention: it must never leak
    /// into the text oxplow writes into a person's project.
    #[test]
    fn the_agent_text_never_mentions_dot_context() {
        let text = core_text();
        for t in text.skills.iter().chain(&text.commands) {
            for t in std::iter::once(t).chain(&t.references) {
                assert!(!t.body.contains(".context"), "{} mentions .context", t.name);
            }
        }
    }
}
