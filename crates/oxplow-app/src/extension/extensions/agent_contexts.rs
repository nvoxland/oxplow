//! `context:` in `extension.yaml`: a short text an extension puts in the
//! coding agent's context — what it should know now, such as the task in
//! progress — sent on a prompt once, then only when it changes
//! (`.context/extensions-commands.md` "Agent context").
//!
//! ```yaml
//! permissions: [agent.write]
//! context:
//!   - id: task-state
//!     doc: What the thread works on now.
//!     needs: [task_tracker]
//!     query: SELECT 'In progress: ' || title AS text FROM ... WHERE thread_id = :thread_id
//! ```
//!
//! The query is the extension's SQL (it may read `config('/ptr')`), with
//! `:thread_id`, `:stream_id` and `:request_id` bound (each may be NULL),
//! and returns a `text` column: its first row is what the agent is told,
//! no row (or an empty text) says nothing. Core runs it and delivers the
//! text (`agent_context::extension_contexts`).

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

/// The params every context query may use (each may be NULL).
pub const PARAMS: &[&str] = &["thread_id", "stream_id", "request_id"];

/// The column a context query returns.
pub const TEXT_COLUMN: &str = "text";

/// One agent context as the extension declares it. A new condition on
/// when it runs is a field here beside `needs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentContextDecl {
    pub id: String,
    /// What it tells the agent, for a person reading the manifest or the
    /// catalog.
    pub doc: String,
    /// What it needs active (`task_tracker`); without it, it doesn't run.
    pub needs: Vec<String>,
    /// The extension's own setting that offers it (as a skill's `when:`);
    /// `None`: always.
    pub when: Option<super::config_schema::ConfigCondition>,
    pub query: String,
}

/// A `context:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentContextFile {
    /// Lowercase letters, digits and dashes; the agent sees it as
    /// `<context from="<namespace>/<id>">`.
    id: String,
    /// What it tells the agent, in a line.
    doc: String,
    /// Capabilities or features it needs active.
    #[serde(default)]
    needs: Vec<String>,
    /// Sent only while a setting of the extension's own holds (`{ config:
    /// /tasks/workflow, in: [agent_tracks] }`), as a skill's `when:`.
    #[serde(default)]
    when: Option<super::config_schema::ConfigCondition>,
    /// SQL returning a `text` column, at most one row; `:thread_id`,
    /// `:stream_id`, `:request_id` bound (NULL when unknown).
    query: String,
}

/// Parse `context:`: the valid declarations, and what's wrong with the
/// rest (at their lines).
pub(crate) fn parse_contexts(
    value: &Value,
    file: &str,
    manifest: &str,
    configuration: Option<&super::config_schema::ConfigurationFile>,
) -> (Vec<AgentContextDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "context");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "context.not_a_list",
                file,
                block,
                "`context` must be a list",
            )],
        );
    };
    let lines = item_lines(manifest, "context");
    let mut out: Vec<AgentContextDecl> = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let f: AgentContextFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at("context.invalid", file, line, format!("context: {e}")));
                continue;
            }
        };
        let problem = if !oxplow_domain::contribution::valid_local(
            oxplow_domain::contribution::ContributionKind::AgentContext,
            &f.id,
        ) {
            Some((
                "context.invalid_id",
                format!(
                    "context id `{}` must be lowercase letters, digits and dashes",
                    f.id
                ),
            ))
        } else if out.iter().any(|o| o.id == f.id) {
            Some((
                "context.duplicate",
                format!("context `{}` is declared twice", f.id),
            ))
        } else if f.doc.trim().is_empty() {
            Some((
                "context.missing_doc",
                format!("context `{}`: `doc` says what it tells the agent", f.id),
            ))
        } else if let Some((code, why)) = f.when.as_ref().and_then(|w| w.problem(configuration)) {
            Some((code, format!("context `{}`: {why}", f.id)))
        } else {
            f.needs
                .iter()
                .find_map(|n| oxplow_domain::capability::check_need(n).err())
                .map(|e| ("context.invalid_need", format!("context `{}`: {e}", f.id)))
        };
        match problem {
            Some((code, message)) => errors.push(at(code, file, line, message)),
            None => out.push(AgentContextDecl {
                id: f.id,
                doc: f.doc,
                needs: f.needs,
                when: f.when,
                query: f.query,
            }),
        }
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> (Vec<AgentContextDecl>, Vec<Diagnostic>) {
        let v: serde_yaml::Value = serde_yaml::from_str(text).unwrap();
        parse_contexts(&v["context"], "extension.yaml", text, None)
    }

    /// Valid entries load; a bad id, a repeated id, a missing doc, an
    /// unknown need and an unknown key are errors at their lines.
    #[test]
    fn contexts_parse_and_report_their_problems() {
        let text = "context:\n  - { id: state, doc: Now., needs: [task_tracker], query: SELECT 'x' AS text }\n  - { id: Bad Id, doc: D., query: SELECT 1 }\n  - { id: state, doc: Again., query: SELECT 1 }\n  - { id: nodoc, doc: ' ', query: SELECT 1 }\n  - { id: needy, doc: D., needs: ['no such thing!'], query: SELECT 1 }\n  - { id: extra, doc: D., query: SELECT 1, when: x }\n";
        let (ok, errors) = parse(text);
        assert_eq!(
            ok.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["state"]
        );
        assert_eq!(ok[0].needs, vec!["task_tracker".to_string()]);
        let codes: Vec<&str> = errors.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "context.invalid_id",
                "context.duplicate",
                "context.missing_doc",
                "context.invalid_need",
                "context.invalid",
            ],
            "{errors:?}"
        );
        let lines: Vec<Option<u32>> = errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, vec![Some(3), Some(4), Some(5), Some(6), Some(7)]);
    }

    #[test]
    fn context_must_be_a_list() {
        let (ok, errors) = parse("context: { id: x }\n");
        assert!(ok.is_empty());
        assert_eq!(errors[0].code, "context.not_a_list");
    }

    /// A `when:` that can never hold is refused at load, as a skill's is.
    #[test]
    fn a_contexts_when_names_a_setting_of_its_own() {
        let manifest = "context:\n  - id: now\n    doc: A fact.\n    when: { config: /tasks/workflow, in: [agent_tracks] }\n    query: SELECT 'x' AS text\n";
        let v: serde_yaml::Value = serde_yaml::from_str(manifest).unwrap();
        let (decls, errors) = parse_contexts(&v["context"], "extension.yaml", manifest, None);
        assert!(decls.is_empty());
        assert!(
            errors.iter().any(|e| e.code == "when.undeclared"),
            "{errors:?}"
        );
    }
}
