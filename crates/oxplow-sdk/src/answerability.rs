//! The agent-answerability check (`.context/extensions.md`
//! "Answerability"): can an agent that reads the right skill answer the
//! questions oxplow, or an extension, exists for?
//!
//! Core's questions are its pages' `answers:` ([`check_pages`], below); an
//! extension may also have its own `questions.yaml` ([`check`]) listing
//! entries:
//!
//! ```yaml
//! - question: Which commits touched src/lib.rs?
//!   skill: oxplow-codebase            # what should lead the agent there
//!   reaches: { sql: "SELECT … FROM v_commit …" }   # or { command, input } or { tool, input }
//!   shape: { columns: [sha, subject] }             # for sql
//! ```
//!
//! For each: the skill's text names every model (`v_*`) the SQL reads and
//! the command or tool it runs; the SQL runs through the gateway and
//! returns exactly `shape.columns`; a command's or tool's input validates
//! against its spec (a tool must be one foundation, or the extension,
//! declares).
//! A page's `answers:` (`{ question, about?, params, skill? }`) are the same
//! check from the page's side ([`check_pages`]): the named skill names
//! every model a `sql` loader reads and every command a `command` loader
//! runs (an answer with no `skill`, a user extension's, skips that), and
//! the page runs with the answer's params with no loader failing.
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
    /// The kind of ref it's about (`file`, `commit`): a page for one
    /// offers it. Phrase it with "this".
    #[serde(default)]
    pub about: Option<String>,
}

/// What answers it: a query, or a command or tool with its input.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reaches {
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    /// An agent tool by its offered name (`oxplow_effort_files`).
    #[serde(default)]
    pub tool: Option<String>,
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

/// The models (`v_*`) a query reads: the tables named after `FROM`,
/// `JOIN` or a comma in a `FROM` list, lowercased — never a `v_*` word in
/// a string, a comment or an alias.
pub fn models_in(sql: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // The clause each open parenthesis interrupted, and the current one.
    let mut clauses: Vec<String> = Vec::new();
    let mut clause = String::new();
    let mut prev = String::new();
    for token in sql_tokens(sql) {
        match token {
            SqlToken::Word(w) => {
                let upper = w.to_ascii_uppercase();
                let table_position =
                    prev == "FROM" || prev == "JOIN" || (prev == "," && clause == "FROM");
                if table_position && w.to_ascii_lowercase().starts_with("v_") {
                    let name = w.to_ascii_lowercase();
                    if !out.contains(&name) {
                        out.push(name);
                    }
                }
                if matches!(
                    upper.as_str(),
                    "SELECT"
                        | "FROM"
                        | "WHERE"
                        | "GROUP"
                        | "ORDER"
                        | "HAVING"
                        | "LIMIT"
                        | "ON"
                        | "USING"
                        | "UNION"
                        | "EXCEPT"
                        | "INTERSECT"
                        | "WINDOW"
                ) {
                    clause = upper.clone();
                }
                prev = if upper == "JOIN" || upper == "FROM" {
                    upper
                } else {
                    w
                };
            }
            SqlToken::Punct(c) => {
                match c {
                    '(' => clauses.push(std::mem::take(&mut clause)),
                    ')' => clause = clauses.pop().unwrap_or_default(),
                    _ => {}
                }
                prev = c.to_string();
            }
        }
    }
    out
}

enum SqlToken {
    Word(String),
    Punct(char),
}

/// SQL's words (a `"quoted"` identifier included) and punctuation;
/// strings and comments dropped.
fn sql_tokens(sql: &str) -> Vec<SqlToken> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c.is_whitespace() {
            i += 1;
        } else if c == '-' && next == Some('-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                i += 1;
            }
            i += 2;
        } else if c == '\'' {
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    break;
                }
                i += 1;
            }
            i += 1;
        } else if c == '"' {
            let start = i + 1;
            i = start;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            out.push(SqlToken::Word(
                chars[start..i.min(chars.len())].iter().collect(),
            ));
            i += 1;
        } else if word(c) {
            let start = i;
            while i < chars.len() && word(chars[i]) {
                i += 1;
            }
            out.push(SqlToken::Word(chars[start..i].iter().collect()));
        } else {
            out.push(SqlToken::Punct(c));
            i += 1;
        }
    }
    out
}

/// Whether `text` names `name` (a model or a command) as a whole word,
/// in any case: `v_commit_file` doesn't name `v_commit`, and a sentence's
/// full stop after `oxplow.work_item.link` doesn't hide it.
pub fn names(text: &str, name: &str) -> bool {
    let (text, name) = (text.to_lowercase(), name.to_lowercase());
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    text.match_indices(&name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let mut after = text[at + name.len()..].chars();
        let (a1, a2) = (after.next(), after.next());
        !(word(before) || before == Some('.')) && !word(a1) && !(a1 == Some('.') && word(a2))
    })
}

/// What the check needs from its host.
pub struct Checker<'a> {
    /// A skill's (or an extension file's) text by name.
    pub skill_text: &'a (dyn Fn(&str) -> Option<String> + Sync),
    /// Where the SQL runs; `None` skips running it (a warning).
    pub sql: Option<&'a SqlGateway>,
    /// A command's input schema by name.
    pub command_schema: &'a (dyn Fn(&str) -> Option<Value> + Sync),
    /// An offered agent tool's input schema by name.
    pub tool_schema: &'a (dyn Fn(&str) -> Option<Value> + Sync),
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
        if let Some(about) = &q.about {
            if let Some(why) = oxplow_app::extension_resources::resource_problem(about) {
                out.errors.push(format!(
                    "{at}: `about: {about}`: {why} — fix: `oxplow/file`, `oxplow/commit`, \
                     `oxplow/effort`, `oxplow/work_item`, …"
                ));
            }
        }
        let Some(text) = (c.skill_text)(&q.skill) else {
            out.errors.push(format!(
                "{at}: no skill `{}` — fix: name a skill that exists",
                q.skill
            ));
            continue;
        };
        match (&q.reaches.sql, &q.reaches.command, &q.reaches.tool) {
            (Some(sql), None, None) => {
                for model in models_in(sql) {
                    if !names(&text, &model) {
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
            (None, Some(name), None) => {
                reach(&mut out, &at, q, &text, "command", name, c.command_schema);
            }
            (None, None, Some(name)) => {
                reach(&mut out, &at, q, &text, "tool", name, c.tool_schema);
            }
            _ => out.errors.push(format!(
                "{at}: `reaches` is `{{ sql }}`, `{{ command, input }}` or `{{ tool, input }}` — fix: pick one"
            )),
        }
    }
    out
}

/// A command or tool question: the skill names it, it exists, and the
/// input fits its schema.
fn reach(
    out: &mut Checked,
    at: &str,
    q: &Question,
    text: &str,
    what: &str,
    name: &str,
    schema_of: &(dyn Fn(&str) -> Option<Value> + Sync),
) {
    if !names(text, name) {
        out.errors.push(format!(
            "{at}: skill `{}` never names `{name}` — fix: say in the skill when to use it",
            q.skill
        ));
    }
    match schema_of(name) {
        None => out.errors.push(format!(
            "{at}: no {what} `{name}` — fix: name a registered one"
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

/// The live half (`OXPLOW_LIVE_ANSWERABILITY=1`): the `decide` model,
/// given only the skill's text and the list of what exists, picks what it
/// would reach for; a finding per question it gets wrong.
pub async fn live(
    file: &str,
    questions: &[Question],
    skill_text: &(dyn Fn(&str) -> Option<String> + Sync),
    catalog: &[String],
    ai: &AiService,
) -> Vec<String> {
    let mut out = Vec::new();
    for (i, q) in questions.iter().enumerate() {
        let want = match (&q.reaches.sql, &q.reaches.command, &q.reaches.tool) {
            (Some(sql), _, _) => models_in(sql).into_iter().next(),
            (_, Some(command), _) => Some(command.clone()),
            (_, _, Some(tool)) => Some(tool.clone()),
            _ => None,
        };
        let (Some(want), Some(text)) = (want, skill_text(&q.skill)) else {
            continue;
        };
        match reached(ai, &text, &q.question, catalog).await {
            Ok(got) => {
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

/// What the `decide` model, given only `text` and the list of what
/// exists, would reach for first to answer `question`.
async fn reached(
    ai: &AiService,
    text: &str,
    question: &str,
    catalog: &[String],
) -> Result<String, String> {
    let state = format!(
        "You are a coding agent. This is the guidance you have:\n\n{text}\n\n\
         The question you must answer: {question}"
    );
    let questions = BTreeMap::from([(
        "reach".to_string(),
        Ask::Choice {
            instructions: "Which model, command or tool would you use first to answer it?".into(),
            options: catalog.to_vec(),
        },
    )]);
    let d = ai
        .decide(&Role::DECIDE, "answerability", &state, &questions)
        .await
        .map_err(|e| e.to_string())?;
    Ok(match d.answers.get("reach") {
        Some(oxplow_app::ai_service::Answer::Choice { choice, .. }) => choice.clone(),
        other => format!("{other:?}"),
    })
}

/// What a page's answers reach: the models its `sql` loaders read and the
/// commands its `command` loaders run, in loader order.
fn page_reaches(page: &oxplow_app::pages::schema::Page) -> Vec<String> {
    use oxplow_app::pages::schema::Loader;
    page.loaders
        .iter()
        .flat_map(|l| match &l.loader {
            Loader::Sql { sql, .. } => models_in(sql),
            Loader::Command { command, .. } => vec![command.clone()],
            _ => Vec::new(),
        })
        // The commands its whole-page actions run: how a page answers
        // "do this".
        .chain(page.actions.iter().map(|a| a.command.clone()))
        .collect()
}

/// Check every page `answers:` entry of `extensions` (loaded from their
/// sources): the named skill names what the page's loaders reach, and with
/// `svc` the page runs with the answer's params and no loader fails.
pub async fn check_pages(
    extensions: &[oxplow_app::extensions::Extension],
    skill_text: &(dyn Fn(&str) -> Option<String> + Sync),
    svc: Option<&oxplow_app::Services>,
) -> Checked {
    use oxplow_app::pages::runner::{PageRunner, RunRequest, RunTarget};
    let mut out = Checked::default();
    for ext in extensions {
        let rel = ext.path.trim_end_matches('/');
        for page in &ext.page_files {
            for (i, answer) in page.answers.iter().enumerate() {
                let at = format!(
                    "{rel}/pages/{}.yaml: answer {} ({:?})",
                    page.slug,
                    i + 1,
                    answer.question
                );
                // A question is put into the agent's input, never sent, and
                // the terminal treats a pasted line break as Enter: one line.
                if answer.question.trim().is_empty() || answer.question.contains(['\n', '\r']) {
                    out.errors.push(format!(
                        "{at}: a question is one non-empty line (a line break pasted into the \
                         agent's input would send it) — fix: write it on one line"
                    ));
                }
                if let Some(about) = &answer.about {
                    if let Some(why) = oxplow_app::extension_resources::resource_problem(about) {
                        out.errors.push(format!(
                            "{at}: `about: {about}`: {why} — fix: a resource kind"
                        ));
                    }
                }
                if let Some(skill) = &answer.skill {
                    match skill_text(skill) {
                        None => out.errors.push(format!(
                            "{at}: no skill `{skill}` — fix: name a skill that exists"
                        )),
                        Some(text) => {
                            for reach in page_reaches(page) {
                                if !names(&text, &reach) {
                                    out.errors.push(format!(
                                        "{at}: skill `{skill}` never names `{reach}` — fix: say in \
                                         the skill what `{reach}` answers"
                                    ));
                                }
                            }
                        }
                    }
                }
                let Some(svc) = svc else {
                    continue;
                };
                // A page answers "do this" with an action: its command
                // exists and the input it gives fits the command.
                for action in &page.actions {
                    match svc.commands.spec(&action.command) {
                        None => out.errors.push(format!(
                            "{at}: action `{}` runs no command `{}` — fix: name a registered one",
                            action.id, action.command
                        )),
                        Some(spec) => {
                            let valid = InputValidator::compile(&spec.input_schema)
                                .map_err(|e| e.to_string())
                                .and_then(|v| v.check(&action.input).map_err(|e| e.to_string()));
                            if let Err(e) = valid {
                                out.errors.push(format!(
                                    "{at}: action `{}`'s input doesn't fit `{}`: {e} — fix: the input",
                                    action.id, action.command
                                ));
                            }
                        }
                    }
                }
                let params = answer
                    .params
                    .iter()
                    .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                    .collect();
                let ran = PageRunner::of(svc)
                    .run(RunRequest {
                        target: RunTarget::Spec(std::sync::Arc::new(page.clone())),
                        params,
                        ctx: oxplow_app::extensions::ViewContext::default(),
                        viewer: oxplow_domain::Actor::Human,
                    })
                    .await;
                match ran {
                    Err(e) => out.errors.push(format!(
                        "{at}: the page doesn't run with its params: {e} — fix: the params, or the page"
                    )),
                    Ok(run) => {
                        if let Some((name, e)) = run
                            .loaders
                            .iter()
                            .find_map(|(n, l)| l.error.as_ref().map(|e| (n, e)))
                        {
                            out.errors.push(format!(
                                "{at}: loader `{name}` fails with its params: {e} — fix: the \
                                 params, or the page"
                            ));
                        }
                    }
                }
            }
        }
    }
    out
}

/// The live half for pages: given the answer's skill, the model must reach
/// for the page's first loader's model or command.
pub async fn live_pages(
    extensions: &[oxplow_app::extensions::Extension],
    skill_text: &(dyn Fn(&str) -> Option<String> + Sync),
    catalog: &[String],
    ai: &AiService,
) -> Vec<String> {
    let mut out = Vec::new();
    for ext in extensions {
        for page in &ext.page_files {
            let Some(want) = page_reaches(page).into_iter().next() else {
                continue;
            };
            for answer in &page.answers {
                let Some(skill) = &answer.skill else { continue };
                let Some(text) = skill_text(skill) else {
                    continue;
                };
                match reached(ai, &text, &answer.question, catalog).await {
                    Ok(got) if got == want => {}
                    Ok(got) => out.push(format!(
                        "{}/pages/{}.yaml: answer ({:?}): given skill `{skill}`, the model reached \
                         for `{got}`, not `{want}` — fix: make the skill point there",
                        ext.path.trim_end_matches('/'),
                        page.slug,
                        answer.question
                    )),
                    Err(e) => out.push(format!(
                        "{}/pages/{}.yaml: answer ({:?}): the model call failed: {e}",
                        ext.path.trim_end_matches('/'),
                        page.slug,
                        answer.question
                    )),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tsk570: a model is a table a query reads — not a `v_*` word in a
    /// string, a comment or an alias — whatever its case.
    #[test]
    fn models_are_the_tables_a_query_reads() {
        assert_eq!(
            models_in(
                "WITH x AS (SELECT 1 FROM V_Work_Item) \
                 SELECT a.n AS v_total, 'v_quoted' -- v_comment\n\
                 FROM v_commit a, v_branch b /* v_block */ \
                 JOIN (SELECT * FROM v_tag) t ON 1 LEFT JOIN json_each(a.parents) p"
            ),
            vec!["v_work_item", "v_commit", "v_branch", "v_tag"]
        );
    }

    /// tsk570: a skill names a model or command as a whole word, in any
    /// case — `v_commit_file` doesn't name `v_commit`.
    #[test]
    fn a_skill_names_a_thing_as_a_whole_word() {
        assert!(names("read `v_commit` for history", "v_commit"));
        assert!(names("Read V_COMMIT.", "v_commit"));
        assert!(!names("read v_commit_file", "v_commit"));
        assert!(!names("read xv_commit", "v_commit"));
        assert!(names("run oxplow.work_item.link.", "oxplow.work_item.link"));
        assert!(!names(
            "run oxplow.work_item.link_all",
            "oxplow.work_item.link"
        ));
        assert!(!names(
            "run oxplow.work_item.links",
            "oxplow.work_item.link"
        ));
    }

    #[test]
    fn models_are_the_v_words_of_a_query() {
        assert_eq!(
            models_in(
                "SELECT c.sha FROM v_commit c JOIN v_commit_file f ON f.sha = c.sha, xv_nope"
            ),
            vec!["v_commit", "v_commit_file"]
        );
    }

    /// Every page `answers:` entry of the shipped extensions reaches what
    /// its skill names and runs with its params.
    #[tokio::test(flavor = "multi_thread")]
    async fn every_page_answer_reaches_what_its_skill_names() {
        let tmp = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(tmp.path())
            .await
            .unwrap();
        let svc = oxplow_app::Services::in_memory(tmp.path()).await.unwrap();
        let text = oxplow_app::capabilities::agent_text(&svc);
        let skill = |name: &str| {
            text.skill_full_text(name).or_else(|| {
                oxplow_app::bundled_extensions::BUNDLED
                    .iter()
                    .find_map(|b| b.file(name).map(str::to_string))
            })
        };
        let shipped: Vec<_> = svc
            .extension_catalog
            .snapshot(tmp.path())
            .extensions()
            .iter()
            .filter(|e| oxplow_app::bundled_extensions::is_reserved(&e.name))
            .cloned()
            .collect();
        let checked = check_pages(&shipped, &skill, Some(&svc)).await;
        let mut errors = checked.errors;
        if std::env::var("OXPLOW_LIVE_ANSWERABILITY").as_deref() == Ok("1") {
            let ai = AiService::for_this_machine(
                std::sync::Arc::new(oxplow_db::SqliteAiCallStore::new(svc.db.clone())),
                svc.ai.providers().clone(),
            );
            errors.extend(live_pages(&shipped, &skill, &catalog(&svc).await, &ai).await);
        }
        assert_eq!(errors, Vec::<String>::new());
        // The questions the person is offered about the kinds of thing
        // pages show come from these answers.
        let answers: Vec<_> = shipped
            .iter()
            .flat_map(|e| e.page_files.iter().flat_map(|p| p.answers.iter()))
            .collect();
        assert!(answers.len() >= 40, "{}", answers.len());
        for kind in [
            "oxplow/file",
            "oxplow/commit",
            "oxplow/effort",
            "oxplow/work_item",
        ] {
            assert!(
                answers.iter().any(|a| a.about.as_deref() == Some(kind)),
                "no answer about `{kind}`"
            );
        }
    }

    /// The mechanism: a page whose answer's skill names its model passes;
    /// one whose skill doesn't fails naming the model; params that break
    /// the query fail too; an answer with no skill skips the naming.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_page_answer_must_be_named_by_its_skill_and_run() {
        let tmp = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(tmp.path())
            .await
            .unwrap();
        let svc = oxplow_app::Services::in_memory(tmp.path()).await.unwrap();
        let extension = |answer: &str, query: &str| {
            let dir = tmp.path().join("acme");
            std::fs::create_dir_all(dir.join("pages")).unwrap();
            std::fs::write(
                dir.join("extension.yaml"),
                "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
            )
            .unwrap();
            std::fs::write(
                dir.join("pages/items.yaml"),
                format!(
                    "title: Items\nparams: [{{ name: state }}]\ndata:\n  items: {{ sql: \"{query}\" }}\nview: {{ viz: table, from: items }}\nanswers:\n  - {{ question: \"What is open?\", {answer} }}\n"
                ),
            )
            .unwrap();
            oxplow_app::extensions::load_extension_at(&dir, "acme")
        };
        let skill = |name: &str| {
            (name == "acme-skill").then(|| "Use v_work_item to list work.".to_string())
        };
        let list = "SELECT title FROM v_work_item WHERE state = :state";

        let ok = extension("params: { state: open }, skill: acme-skill", list);
        let checked = check_pages(&[ok], &skill, Some(&svc)).await;
        assert_eq!(checked.errors, Vec::<String>::new());

        let unnamed = extension(
            "params: { state: open }, skill: acme-skill",
            "SELECT uri FROM v_effort",
        );
        let checked = check_pages(&[unnamed], &skill, Some(&svc)).await;
        assert!(
            checked
                .errors
                .iter()
                .any(|e| e.contains("never names `v_effort`")),
            "{:?}",
            checked.errors
        );

        let broken = extension(
            "params: { state: open }",
            "SELECT title FROM v_work_item WHERE nope = :state",
        );
        let checked = check_pages(&[broken], &skill, Some(&svc)).await;
        assert!(
            checked
                .errors
                .iter()
                .any(|e| e.contains("loader `items` fails")),
            "{:?}",
            checked.errors
        );
    }

    /// P7.C5: a bundled extension's `questions.yaml` reaches what its
    /// skill (a markdown file in it) names, against a running oxplow — its
    /// models published and its commands registered, as at boot.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_bundled_extensions_answer_their_questions() {
        let tmp = tempfile::tempdir().unwrap();
        oxplow_app::vcs::GitProvider
            .init_repository(tmp.path())
            .await
            .unwrap();
        let svc = oxplow_app::Services::in_memory(tmp.path()).await.unwrap();
        let command = |name: &str| svc.commands.spec(name).map(|s| s.input_schema);
        let tools = tool_schemas(&svc).await;
        let tool = |name: &str| tools.get(name).cloned();
        let mut asked = 0;
        for b in oxplow_app::bundled_extensions::BUNDLED {
            let file_of = |name: &str| b.file(name).map(str::to_string);
            let Some(yaml) = file_of("questions.yaml") else {
                continue;
            };
            let file = format!("{}/questions.yaml", b.name);
            let questions = parse(&yaml).unwrap_or_else(|e| panic!("{file}: {e}"));
            asked += questions.len();
            let checked = check(
                &file,
                &questions,
                &Checker {
                    skill_text: &file_of,
                    sql: Some(&svc.sql),
                    command_schema: &command,
                    tool_schema: &tool,
                },
            )
            .await;
            assert_eq!(checked.errors, Vec::<String>::new());
            assert_eq!(checked.warnings, Vec::<String>::new());
        }
        assert!(asked >= 3, "{asked}");
    }

    /// The tools an agent is offered, by name, with their input schemas.
    async fn tool_schemas(svc: &oxplow_app::Services) -> BTreeMap<String, Value> {
        oxplow_app::agent_tools::offered(svc, &agent())
            .await
            .unwrap()
            .into_iter()
            .map(|t| (t.name, t.input_schema))
            .collect()
    }

    fn agent() -> oxplow_domain::Actor {
        oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        }
    }

    /// A question may reach a tool: the skill names it, it is offered, and
    /// the input fits its schema.
    #[tokio::test]
    async fn a_question_can_reach_a_tool() {
        let skill = |_: &str| Some("Use oxplow_effort_files for an effort's files.".to_string());
        let command = |_: &str| None;
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "effort": { "type": "string" } },
            "additionalProperties": false
        });
        let tool = |name: &str| (name == "oxplow_effort_files").then(|| schema.clone());
        let checker = Checker {
            skill_text: &skill,
            sql: None,
            command_schema: &command,
            tool_schema: &tool,
        };
        let ask = |tool: &str, input: &str| {
            parse(&format!(
                "- question: Which files did this effort touch?\n  skill: s\n  reaches: {{ tool: {tool}, input: {input} }}\n"
            ))
            .unwrap()
        };
        let ok = check(
            "q.yaml",
            &ask(
                "oxplow_effort_files",
                "{ effort: 'oxplow:oxplow/effort?id=eff1' }",
            ),
            &checker,
        )
        .await;
        assert_eq!(ok.errors, Vec::<String>::new());
        let unknown = check("q.yaml", &ask("oxplow_nope", "{}"), &checker).await;
        assert!(
            unknown
                .errors
                .iter()
                .any(|e| e.contains("no tool `oxplow_nope`")),
            "{:?}",
            unknown.errors
        );
        assert!(
            unknown.errors.iter().any(|e| e.contains("never names")),
            "{:?}",
            unknown.errors
        );
        let bad = check(
            "q.yaml",
            &ask("oxplow_effort_files", "{ nope: 1 }"),
            &checker,
        )
        .await;
        assert!(
            bad.errors.iter().any(|e| e.contains("doesn't fit")),
            "{:?}",
            bad.errors
        );
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
                    session_id: None,
                    thread_id: None,
                    stream_id: None,
                })
                .into_iter()
                .map(|s| s.id),
        );
        out.extend(tool_schemas(svc).await.into_keys());
        out
    }
}
