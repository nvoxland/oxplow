//! `skills:` in `extension.yaml` (stable): text an extension gives the
//! coding agent — a skill (`SKILL.md`) or a slash command — offered while
//! what it needs is active (`.context/extensions-commands.md` "Skills").
//!
//! ```yaml
//! skills:
//!   - { name: tasks, file: skills/tasks/SKILL.md, needs: [task_tracker] }
//!   - { name: next-task, kind: command, file: commands/next-task.md }
//! implementations:
//!   - { capability: task_tracker, id: oxtask, entry: oxplow:oxtask, skills: [next-task] }
//! ```
//!
//! An implementation that lists a skill owns it: it's offered only while
//! that implementation is the active one. Which are offered is
//! `capabilities::agent_text`'s to say; the agent runtimes write them
//! (`oxplow_domain::agent::text::AgentText`).

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::config_schema::{ConfigCondition, ConfigurationFile};
use super::manifest_v2::{at, item_lines, key_line};
use super::ExtensionFiles;

/// What the agent's runtime makes of it.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    /// A skill the agent loads when it's relevant: `<name>/SKILL.md`.
    #[default]
    Skill,
    /// A slash command the person runs: `/oxplow:<name>`.
    Command,
}

/// One skill or command as the extension declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillDecl {
    pub name: String,
    pub kind: SkillKind,
    /// Its markdown, inside the extension.
    pub file: String,
    /// What it needs active (`task_tracker`, `task_tracker.comments`).
    pub needs: Vec<String>,
    /// The `description:` of its frontmatter: when the agent loads a skill,
    /// what a command does.
    pub description: String,
    /// The setting of the extension's own configuration it's offered by:
    /// offered only while it holds.
    pub when: Option<ConfigCondition>,
}

/// A `skills:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SkillFile {
    name: String,
    #[serde(default)]
    kind: SkillKind,
    file: String,
    #[serde(default)]
    needs: Vec<String>,
    /// Offered only while a setting of the extension's own configuration is
    /// one of the listed values: `{ config: /tasks/workflow, in: [agent_tracks] }`.
    #[serde(default)]
    when: Option<ConfigCondition>,
}

/// Parse `skills:`: the valid declarations, and what's wrong with the
/// rest (at their lines). A `when:` is checked against `configuration`, the
/// extension's own declaration.
pub(crate) fn parse_skills(
    value: &Value,
    file: &str,
    manifest: &str,
    files: &dyn ExtensionFiles,
    configuration: Option<&ConfigurationFile>,
) -> (Vec<SkillDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "skills");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "skill.not_a_list",
                file,
                block,
                "`skills` must be a list",
            )],
        );
    };
    let core = oxplow_agent_text::core_text();
    let mut out: Vec<SkillDecl> = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "skills");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let f: SkillFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at("skill.invalid", file, item_line, format!("skill: {e}")));
                continue;
            }
        };
        let line = item_line;
        let problem = if !well_formed(&f.name) {
            Some((
                "skill.invalid_name",
                format!(
                    "skill name `{}` must be lowercase letters, digits and `-`, starting with a letter",
                    f.name
                ),
            ))
        } else if core.names(&f.name) {
            Some((
                "skill.reserved_name",
                format!("skill name `{}` is one of oxplow's own", f.name),
            ))
        } else if out.iter().any(|o| o.name == f.name) {
            Some((
                "skill.duplicate",
                format!("skill `{}` is declared twice", f.name),
            ))
        } else if let Some(e) = f
            .needs
            .iter()
            .find_map(|n| oxplow_domain::capability::check_need(n).err())
        {
            Some(("skill.unknown_need", e))
        } else if let Some((code, why)) = f.when.as_ref().and_then(|w| w.problem(configuration)) {
            Some((code, format!("skill `{}`: {why}", f.name)))
        } else {
            match files.read(&f.file) {
                None => Some((
                    "skill.missing_file",
                    format!("skill `{}`: no file `{}`", f.name, f.file),
                )),
                Some(body) => body_problem(&f, &body),
            }
        };
        match problem {
            Some((code, p)) => errors.push(at(code, file, line, p)),
            None => out.push(SkillDecl {
                description: description_of(&files.read(&f.file).unwrap_or_default()),
                name: f.name,
                kind: f.kind,
                file: f.file,
                needs: f.needs,
                when: f.when,
            }),
        }
    }
    (out, errors)
}

/// The frontmatter's `description:` (a skill without one is refused).
fn description_of(body: &str) -> String {
    body.strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
        .and_then(|(front, _)| front.lines().find_map(|l| l.strip_prefix("description:")))
        .map(|v| v.trim().trim_matches('"').to_string())
        .unwrap_or_default()
}

fn well_formed(name: &str) -> bool {
    oxplow_domain::contribution::valid_local(
        oxplow_domain::contribution::ContributionKind::Skill,
        name,
    )
}

/// A skill's frontmatter names it (the runtimes key discovery on it) and
/// says when it applies; a command's says what it does.
fn body_problem(f: &SkillFile, body: &str) -> Option<(&'static str, String)> {
    let Some((front, _)) = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
    else {
        return Some((
            "skill.no_frontmatter",
            format!("skill `{}`: `{}` has no `---` frontmatter", f.name, f.file),
        ));
    };
    let field = |key: &str| {
        front
            .lines()
            .find_map(|l| l.strip_prefix(key))
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    if field("description:").is_none() {
        return Some((
            "skill.no_description",
            format!("skill `{}`: its frontmatter has no `description:`", f.name),
        ));
    }
    match (f.kind, field("name:")) {
        (SkillKind::Skill, Some(name)) if name == f.name => None,
        (SkillKind::Skill, _) => Some((
            "skill.frontmatter_name",
            format!(
                "skill `{}`: its frontmatter `name:` must be `{}`",
                f.name, f.name
            ),
        )),
        (SkillKind::Command, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str, files: &[(&str, &str)]) -> (Vec<SkillDecl>, Vec<Diagnostic>) {
        parse_configured(yaml, files, None)
    }

    fn parse_configured(
        yaml: &str,
        files: &[(&str, &str)],
        configuration: Option<&ConfigurationFile>,
    ) -> (Vec<SkillDecl>, Vec<Diagnostic>) {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        let manifest = format!("skills:\n{yaml}");
        let doc: Value = serde_yaml::from_str(&manifest).unwrap();
        parse_skills(
            &doc["skills"],
            "extension.yaml",
            &manifest,
            &super::super::Disk(dir.path().to_path_buf()),
            configuration,
        )
    }

    /// Settings with a pick-one `workflow`, a free `note`, a `limit` and an
    /// object `tasks`.
    fn settings() -> ConfigurationFile {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "schema": {
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "object",
                        "properties": {
                            "workflow": { "type": "string", "enum": ["off", "agent_tracks"] },
                            "note": { "type": "string" }
                        }
                    },
                    "limit": { "type": "integer" }
                }
            },
            "defaults": { "tasks": { "workflow": "agent_tracks" } }
        }))
        .unwrap()
    }

    const SKILL: &str = "---\nname: tasks\ndescription: Tasks.\n---\n# Tasks\n";

    #[test]
    fn a_skill_and_a_command_are_declared() {
        let (decls, errors) = parse(
            "  - { name: tasks, file: s/SKILL.md, needs: [task_tracker] }\n  - { name: next-task, kind: command, file: c.md }\n",
            &[("s/SKILL.md", SKILL), ("c.md", "---\ndescription: Next.\n---\nGo.\n")],
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            decls,
            vec![
                SkillDecl {
                    name: "tasks".into(),
                    kind: SkillKind::Skill,
                    file: "s/SKILL.md".into(),
                    needs: vec!["task_tracker".into()],
                    description: "Tasks.".into(),
                    when: None,
                },
                SkillDecl {
                    name: "next-task".into(),
                    kind: SkillKind::Command,
                    file: "c.md".into(),
                    needs: vec![],
                    description: "Next.".into(),
                    when: None,
                },
            ]
        );
    }

    #[test]
    fn what_wont_install_is_refused() {
        let errors = |yaml: &str, files: &[(&str, &str)]| {
            parse(yaml, files)
                .1
                .iter()
                .map(|d| d.render(""))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(errors("  - { name: Work, file: s.md }\n", &[]).contains("lowercase"));
        assert!(
            errors("  - { name: configure, kind: command, file: s.md }\n", &[])
                .contains("oxplow's own")
        );
        assert!(errors("  - { name: tasks, file: s.md }\n", &[]).contains("no file"));
        assert!(
            errors("  - { name: tasks, file: s.md }\n", &[("s.md", "# x\n")])
                .contains("frontmatter")
        );
        assert!(errors(
            "  - { name: tasks, file: s.md, needs: [tickets] }\n",
            &[("s.md", SKILL)]
        )
        .contains("isn't a capability"));
        assert!(
            errors("  - { name: other, file: s.md }\n", &[("s.md", SKILL)])
                .contains("`name:` must be `other`")
        );
        assert!(errors(
            "  - { name: tasks, file: s.md }\n",
            &[("s.md", "---\nname: tasks\n---\n")]
        )
        .contains("description"));
        assert!(errors(
            "  - { name: tasks, file: s.md }\n  - { name: tasks, file: s.md }\n",
            &[("s.md", SKILL)]
        )
        .contains("twice"));
    }

    /// `when:` offers a skill by a setting of the extension's own.
    #[test]
    fn a_skill_names_the_setting_that_offers_it() {
        let (decls, errors) = parse_configured(
            "  - { name: tasks, file: s.md, when: { config: /tasks/workflow, in: [agent_tracks] } }\n",
            &[("s.md", SKILL)],
            Some(&settings()),
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            decls[0].when,
            Some(ConfigCondition {
                config: "/tasks/workflow".into(),
                values: vec![serde_json::json!("agent_tracks")],
            })
        );
    }

    /// A `when:` that can never hold is refused at load, saying why.
    #[test]
    fn a_when_that_names_no_setting_is_refused() {
        let refused = |when: &str, configuration: Option<&ConfigurationFile>| {
            let (decls, errors) = parse_configured(
                &format!("  - {{ name: tasks, file: s.md, when: {when} }}\n"),
                &[("s.md", SKILL)],
                configuration,
            );
            assert!(decls.is_empty(), "{when}: refused skills aren't declared");
            errors
                .iter()
                .map(|d| format!("{} {}", d.code, d.render("")))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let settings = settings();
        let no_configuration = refused("{ config: /tasks/workflow, in: [off] }", None);
        assert!(
            no_configuration.contains("when.undeclared")
                && no_configuration.contains("configuration:"),
            "{no_configuration}"
        );
        let unknown = refused("{ config: /tasks/flow, in: [off] }", Some(&settings));
        assert!(
            unknown.contains("when.unknown_key") && unknown.contains("/tasks/flow"),
            "{unknown}"
        );
        let object = refused("{ config: /tasks, in: [off] }", Some(&settings));
        assert!(object.contains("when.not_scalar"), "{object}");
        let outside = refused(
            "{ config: /tasks/workflow, in: [off, sometimes] }",
            Some(&settings),
        );
        assert!(
            outside.contains("when.not_in_enum") && outside.contains("sometimes"),
            "{outside}"
        );
        let wrong_type = refused("{ config: /limit, in: [many] }", Some(&settings));
        assert!(wrong_type.contains("when.wrong_type"), "{wrong_type}");
        let empty = refused("{ config: /tasks/workflow, in: [] }", Some(&settings));
        assert!(empty.contains("when.empty"), "{empty}");
        let free = parse_configured(
            "  - { name: tasks, file: s.md, when: { config: /tasks/note, in: [anything] } }\n",
            &[("s.md", SKILL)],
            Some(&settings),
        );
        assert!(
            free.1.is_empty(),
            "a key without an enum takes any value of its type: {:?}",
            free.1
        );
    }
}
