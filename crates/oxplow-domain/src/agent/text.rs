//! What an agent is told: skills and slash commands, as data. Core's own
//! live in `oxplow-agent-text`; the project's extensions offer more
//! (`oxplow_app::capabilities::agent_text`); each harness writes them where
//! its agent reads them.

/// One piece of text for the agent: a skill (its `SKILL.md`, whose
/// frontmatter `name:` is `name`) or a slash command (its markdown).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Text {
    pub name: String,
    pub body: String,
    /// A skill's reference files (`references/<name>.md`), which the agent
    /// reads only when `body` sends it there; empty for a command.
    pub references: Vec<Text>,
}

/// Every skill and slash command an agent runtime gets.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AgentText {
    pub skills: Vec<Text>,
    pub commands: Vec<Text>,
}

impl AgentText {
    /// Every skill as `(name, description)`, the description taken from
    /// its frontmatter: the index an agent that can't discover skill files
    /// (an ACP agent) is given, to fetch bodies with `oxplow_get_skill`.
    pub fn skill_index(&self) -> Vec<(&str, &str)> {
        self.skills
            .iter()
            .map(|s| (s.name.as_str(), frontmatter_description(&s.body)))
            .collect()
    }

    /// One skill's `SKILL.md` body by name.
    pub fn skill_body(&self, name: &str) -> Option<&str> {
        self.skills
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.body.as_str())
    }

    /// A skill's `SKILL.md` and all its reference files, as one text: all
    /// an agent can learn from the skill, for checking what it says.
    pub fn skill_full_text(&self, name: &str) -> Option<String> {
        let skill = self.skills.iter().find(|s| s.name == name)?;
        let mut all = skill.body.clone();
        for reference in &skill.references {
            all.push('\n');
            all.push_str(&reference.body);
        }
        Some(all)
    }

    /// One of a skill's reference files by name.
    pub fn skill_reference(&self, name: &str, reference: &str) -> Option<&str> {
        self.skills
            .iter()
            .find(|s| s.name == name)?
            .references
            .iter()
            .find(|r| r.name == reference)
            .map(|r| r.body.as_str())
    }

    /// The names of a skill's reference files.
    pub fn skill_reference_names(&self, name: &str) -> Vec<&str> {
        self.skills
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.references.iter().map(|r| r.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Whether `name` is taken by a skill or command already.
    pub fn names(&self, name: &str) -> bool {
        self.skills
            .iter()
            .chain(&self.commands)
            .any(|t| t.name == name)
    }
}

/// The `description:` line of a `---`-fenced frontmatter block.
pub fn frontmatter_description(body: &str) -> &str {
    let Some(front) = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split("\n---").next())
    else {
        return "";
    };
    front
        .lines()
        .find_map(|l| l.strip_prefix("description:"))
        .map(str::trim)
        .unwrap_or("")
}
