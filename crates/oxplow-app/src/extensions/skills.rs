//! `skills:` in `extension.yaml` (stable): text an extension gives the
//! coding agent — a skill (`SKILL.md`) or a slash command — offered while
//! what it needs is active (`.context/extensions.md` "Skills").
//!
//! ```yaml
//! skills:
//!   - { name: work-items, file: skills/work-items/SKILL.md, needs: [work_items] }
//!   - { name: work-next, kind: command, file: commands/work-next.md }
//! implementations:
//!   - { capability: work_items, id: oxplow, entry: oxplow:tasks, skills: [work-next] }
//! ```
//!
//! An implementation that lists a skill owns it: it's offered only while
//! that implementation is the active one. Which are offered is
//! `capabilities::agent_text`'s to say; the agent runtimes write them
//! (`oxplow_plugin::AgentText`).

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, entry_line, key_line};
use super::ExtensionFiles;

/// What the agent's runtime makes of it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
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
    /// What it needs active (`work_items`, `work_items.comments`).
    pub needs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillFile {
    name: String,
    #[serde(default)]
    kind: SkillKind,
    file: String,
    #[serde(default)]
    needs: Vec<String>,
}

/// Parse `skills:`: the valid declarations, and what's wrong with the
/// rest (at their lines).
pub(crate) fn parse_skills(
    value: &Value,
    file: &str,
    manifest: &str,
    files: &dyn ExtensionFiles,
) -> (Vec<SkillDecl>, Vec<String>) {
    let block = key_line(manifest, "skills");
    let Some(items) = value.as_sequence() else {
        return (Vec::new(), vec![at(file, block, "`skills` must be a list")]);
    };
    let core = oxplow_plugin::AgentText::core();
    let mut out: Vec<SkillDecl> = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let f: SkillFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, block, format!("skill: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "skills", "name", &f.name).or(block);
        let problem = if !well_formed(&f.name) {
            Some(format!(
                "skill name `{}` must be lowercase letters, digits and `-`, starting with a letter",
                f.name
            ))
        } else if core.names(&f.name) {
            Some(format!("skill name `{}` is one of oxplow's own", f.name))
        } else if out.iter().any(|o| o.name == f.name) {
            Some(format!("skill `{}` is declared twice", f.name))
        } else if let Some(e) = f
            .needs
            .iter()
            .find_map(|n| oxplow_domain::capability::check_need(n).err())
        {
            Some(e)
        } else {
            match files.read(&f.file) {
                None => Some(format!("skill `{}`: no file `{}`", f.name, f.file)),
                Some(body) => body_problem(&f, &body),
            }
        };
        match problem {
            Some(p) => errors.push(at(file, line, p)),
            None => out.push(SkillDecl {
                name: f.name,
                kind: f.kind,
                file: f.file,
                needs: f.needs,
            }),
        }
    }
    (out, errors)
}

fn well_formed(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A skill's frontmatter names it (the runtimes key discovery on it) and
/// says when it applies; a command's says what it does.
fn body_problem(f: &SkillFile, body: &str) -> Option<String> {
    let Some((front, _)) = body
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---"))
    else {
        return Some(format!(
            "skill `{}`: `{}` has no `---` frontmatter",
            f.name, f.file
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
        return Some(format!(
            "skill `{}`: its frontmatter has no `description:`",
            f.name
        ));
    }
    match (f.kind, field("name:")) {
        (SkillKind::Skill, Some(name)) if name == f.name => None,
        (SkillKind::Skill, _) => Some(format!(
            "skill `{}`: its frontmatter `name:` must be `{}`",
            f.name, f.name
        )),
        (SkillKind::Command, _) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str, files: &[(&str, &str)]) -> (Vec<SkillDecl>, Vec<String>) {
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
        )
    }

    const SKILL: &str = "---\nname: work-items\ndescription: Work items.\n---\n# Work items\n";

    #[test]
    fn a_skill_and_a_command_are_declared() {
        let (decls, errors) = parse(
            "  - { name: work-items, file: s/SKILL.md, needs: [work_items] }\n  - { name: work-next, kind: command, file: c.md }\n",
            &[("s/SKILL.md", SKILL), ("c.md", "---\ndescription: Next.\n---\nGo.\n")],
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(
            decls,
            vec![
                SkillDecl {
                    name: "work-items".into(),
                    kind: SkillKind::Skill,
                    file: "s/SKILL.md".into(),
                    needs: vec!["work_items".into()],
                },
                SkillDecl {
                    name: "work-next".into(),
                    kind: SkillKind::Command,
                    file: "c.md".into(),
                    needs: vec![],
                },
            ]
        );
    }

    #[test]
    fn what_wont_install_is_refused() {
        let errors = |yaml: &str, files: &[(&str, &str)]| parse(yaml, files).1.join("\n");
        assert!(errors("  - { name: Work, file: s.md }\n", &[]).contains("lowercase"));
        assert!(
            errors("  - { name: configure, kind: command, file: s.md }\n", &[])
                .contains("oxplow's own")
        );
        assert!(errors("  - { name: work-items, file: s.md }\n", &[]).contains("no file"));
        assert!(errors(
            "  - { name: work-items, file: s.md }\n",
            &[("s.md", "# x\n")]
        )
        .contains("frontmatter"));
        assert!(errors(
            "  - { name: work-items, file: s.md, needs: [tickets] }\n",
            &[("s.md", SKILL)]
        )
        .contains("isn't a capability"));
        assert!(
            errors("  - { name: other, file: s.md }\n", &[("s.md", SKILL)])
                .contains("`name:` must be `other`")
        );
        assert!(errors(
            "  - { name: work-items, file: s.md }\n",
            &[("s.md", "---\nname: work-items\n---\n")]
        )
        .contains("description"));
        assert!(errors(
            "  - { name: work-items, file: s.md }\n  - { name: work-items, file: s.md }\n",
            &[("s.md", SKILL)]
        )
        .contains("twice"));
    }
}
