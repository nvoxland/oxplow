//! The agent-answerability check (P5.F1, `.context/extensions.md`
//! "Answerability"): can an agent that reads the right skill answer the
//! questions a capability exists for?
//!
//! A questions file (`crates/oxplow-plugin/assets/questions/<capability>.yaml`,
//! or an extension's own `questions.yaml`) lists entries:
//!
//! ```yaml
//! - question: Which commits touched src/lib.rs?
//!   skill: oxplow-codebase            # what should lead the agent there
//!   reaches: { sql: "SELECT … FROM v_commit …" }   # or { command, input }
//!   shape: { columns: [sha, subject] }             # for sql
//! ```
//!
//! For each: the skill's text names every model (`v_*`) the SQL reads and
//! the command it runs; the SQL runs through the gateway and returns
//! exactly `shape.columns`; a command's input validates against its spec.
//! With `OXPLOW_LIVE_ANSWERABILITY=1`, [`live`] also asks the `decide`
//! model, given only the skill text and the catalog, what it would reach
//! for — it must pick the same model or command.

use std::collections::BTreeMap;

use oxplow_app::ai_service::{AiService, Question as Ask, Role};
use oxplow_app::sql_gateway::SqlGateway;
use oxplow_domain::InputValidator;
use serde::Deserialize;
use serde_json::Value;

/// One question an agent should be able to answer.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Question {
    pub question: String,
    /// The skill (or, in an extension, the markdown file) that should
    /// lead the agent to the answer.
    pub skill: String,
    pub reaches: Reaches,
    #[serde(default)]
    pub shape: Option<Shape>,
}

/// What answers it: a query, or a command with its input.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reaches {
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub input: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub columns: Vec<String>,
}

/// Parse a questions file.
pub fn parse(yaml: &str) -> Result<Vec<Question>, String> {
    serde_yaml::from_str(yaml).map_err(|e| e.to_string())
}

/// The models (`v_*`) a query reads.
pub fn models_in(sql: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = sql.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i + 1 < bytes.len() {
        let starts = bytes[i] == b'v' && bytes[i + 1] == b'_' && (i == 0 || !word(bytes[i - 1]));
        if starts {
            let end = (i..bytes.len())
                .find(|&j| !word(bytes[j]))
                .unwrap_or(bytes.len());
            let name = sql[i..end].to_string();
            if !out.contains(&name) {
                out.push(name);
            }
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

/// What the check needs from its host.
pub struct Checker<'a> {
    /// A skill's (or an extension file's) text by name.
    pub skill_text: &'a dyn Fn(&str) -> Option<String>,
    /// Where the SQL runs; `None` skips running it (a warning).
    pub sql: Option<&'a SqlGateway>,
    /// A command's input schema by name.
    pub command_schema: &'a dyn Fn(&str) -> Option<Value>,
}

/// What checking a file found: `<file>: question N (…): what — fix`
/// lines.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Checked {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// Check every question in `file` (shown as `file`).
pub async fn check(file: &str, questions: &[Question], c: &Checker<'_>) -> Checked {
    let mut out = Checked::default();
    for (i, q) in questions.iter().enumerate() {
        let at = format!("{file}: question {} ({:?})", i + 1, q.question);
        let Some(text) = (c.skill_text)(&q.skill) else {
            out.errors.push(format!(
                "{at}: no skill `{}` — fix: name a skill that exists",
                q.skill
            ));
            continue;
        };
        match (&q.reaches.sql, &q.reaches.command) {
            (Some(sql), None) => {
                for model in models_in(sql) {
                    if !text.contains(&model) {
                        out.errors.push(format!(
                            "{at}: skill `{}` never names `{model}` — fix: say in the skill what \
                             `{model}` answers",
                            q.skill
                        ));
                    }
                }
                let Some(shape) = &q.shape else {
                    out.errors.push(format!(
                        "{at}: a sql question needs `shape.columns` — fix: add them"
                    ));
                    continue;
                };
                match c.sql {
                    None => out.warnings.push(format!(
                        "{at}: the SQL wasn't run (no database) — fix: none needed here"
                    )),
                    Some(gateway) => match gateway.query_sql(sql, vec![], Some(1)).await {
                        Ok(result) if result.columns == shape.columns => {}
                        Ok(result) => out.errors.push(format!(
                            "{at}: the SQL returns {:?}, the shape says {:?} — fix: one of them",
                            result.columns, shape.columns
                        )),
                        Err(e) => out
                            .errors
                            .push(format!("{at}: the SQL fails: {e} — fix: the query")),
                    },
                }
            }
            (None, Some(name)) => {
                if !text.contains(name.as_str()) {
                    out.errors.push(format!(
                        "{at}: skill `{}` never names `{name}` — fix: say in the skill when to \
                         run it",
                        q.skill
                    ));
                }
                match (c.command_schema)(name) {
                    None => out.errors.push(format!(
                        "{at}: no command `{name}` — fix: name a registered one"
                    )),
                    Some(schema) => {
                        let input = q.reaches.input.clone().unwrap_or(Value::Null);
                        let valid = InputValidator::compile(&schema)
                            .map_err(|e| e.to_string())
                            .and_then(|v| v.check(&input).map_err(|e| e.to_string()));
                        if let Err(e) = valid {
                            out.errors.push(format!(
                                "{at}: the input doesn't fit `{name}`: {e} — fix: the input"
                            ));
                        }
                    }
                }
            }
            _ => out.errors.push(format!(
                "{at}: `reaches` is `{{ sql }}` or `{{ command, input }}` — fix: pick one"
            )),
        }
    }
    out
}

/// The live half (`OXPLOW_LIVE_ANSWERABILITY=1`): the `decide` model,
/// given only the skill's text and the list of what exists, picks what it
/// would reach for; a finding per question it gets wrong.
pub async fn live(
    file: &str,
    questions: &[Question],
    skill_text: &dyn Fn(&str) -> Option<String>,
    catalog: &[String],
    ai: &AiService,
) -> Vec<String> {
    let mut out = Vec::new();
    for (i, q) in questions.iter().enumerate() {
        let want = match (&q.reaches.sql, &q.reaches.command) {
            (Some(sql), _) => models_in(sql).into_iter().next(),
            (_, Some(command)) => Some(command.clone()),
            _ => None,
        };
        let (Some(want), Some(text)) = (want, skill_text(&q.skill)) else {
            continue;
        };
        let state = format!(
            "You are a coding agent. This is the guidance you have:\n\n{text}\n\n\
             The question you must answer: {}",
            q.question
        );
        let questions = BTreeMap::from([(
            "reach".to_string(),
            Ask::Choice {
                instructions: "Which model or command would you use first to answer it?".into(),
                options: catalog.to_vec(),
            },
        )]);
        match ai
            .decide(Role::Decide, "answerability", &state, &questions)
            .await
        {
            Ok(d) => {
                let got = match d.answers.get("reach") {
                    Some(oxplow_app::ai_service::Answer::Choice { choice, .. }) => choice.clone(),
                    other => format!("{other:?}"),
                };
                if got != want {
                    out.push(format!(
                        "{file}: question {} ({:?}): given skill `{}`, the model reached for \
                         `{got}`, not `{want}` — fix: make the skill point there",
                        i + 1,
                        q.question,
                        q.skill
                    ));
                }
            }
            Err(e) => out.push(format!(
                "{file}: question {}: the model call failed: {e}",
                i + 1
            )),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_are_the_v_words_of_a_query() {
        assert_eq!(
            models_in(
                "SELECT c.sha FROM v_commit c JOIN v_commit_file f ON f.sha = c.sha, xv_nope"
            ),
            vec!["v_commit", "v_commit_file"]
        );
    }

    /// P5.F1's red: every capability question reaches something its skill
    /// names — the SQL runs with its declared columns and a command's
    /// input fits its spec.
    #[tokio::test]
    async fn every_capability_question_reaches_what_its_skill_names() {
        let tmp = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(tmp.path())
            .await
            .unwrap();
        let svc = oxplow_app::Services::in_memory(tmp.path()).unwrap();
        let skill = |name: &str| oxplow_plugin::skill_body(name).map(str::to_string);
        let command = |name: &str| svc.commands.spec(name).map(|s| s.input_schema);
        let checker = Checker {
            skill_text: &skill,
            sql: Some(&svc.sql),
            command_schema: &command,
        };
        let mut errors = Vec::new();
        let mut asked = 0;
        for (capability, yaml) in oxplow_plugin::CAPABILITY_QUESTIONS {
            let file = format!("questions/{capability}.yaml");
            let questions = parse(yaml).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(!questions.is_empty(), "{file} asks nothing");
            asked += questions.len();
            let checked = check(&file, &questions, &checker).await;
            assert_eq!(checked.warnings, Vec::<String>::new());
            errors.extend(checked.errors);
            if std::env::var("OXPLOW_LIVE_ANSWERABILITY").as_deref() == Ok("1") {
                // This machine's roles and keys; the throwaway services'
                // are empty.
                let ai = AiService::for_this_machine(std::sync::Arc::new(
                    oxplow_db::SqliteAiCallStore::new(svc.db.clone()),
                ));
                let catalog = catalog(&svc).await;
                errors.extend(live(&file, &questions, &skill, &catalog, &ai).await);
            }
        }
        assert!(asked >= 12, "{asked}");
        assert_eq!(errors, Vec::<String>::new());
    }

    /// Every model and every command an agent may run: what the live check
    /// offers the model.
    async fn catalog(svc: &oxplow_app::Services) -> Vec<String> {
        let models = svc
            .sql
            .query_sql("SELECT view FROM v_model ORDER BY view", vec![], None)
            .await
            .unwrap();
        let mut out: Vec<String> = models
            .rows
            .into_iter()
            .filter_map(|r| match r.into_iter().next() {
                Some(oxplow_db::SqlCell::Text(t)) => Some(t),
                _ => None,
            })
            .collect();
        out.extend(
            svc.commands
                .list(&oxplow_domain::Actor::Agent {
                    thread_id: None,
                    stream_id: None,
                })
                .into_iter()
                .map(|s| s.name),
        );
        out
    }
}
