//! URI builders and readers (`.context/resources.md` "Using URIs").
//!
//! Every Rust producer that names a thing — in an event subject, a
//! payload, a `page_ref` row, a command's input — builds the URI here, and
//! every reader reads one back through [`parse_as`] / [`id_of`] /
//! [`param_of`] / [`parse_resource`], so a resource's name and params are
//! spelled once (`uri::resources::{EFFORT, FILE, …}`) and a value is always
//! escaped and decoded by the grammar. A work list builds its own items'
//! URIs through [`work_item_uri`]. The `refs_are_built_and_parsed_through_the_grammar`
//! source guard refuses a hand-built or hand-parsed URI anywhere else.
//!
//! The params of oxplow's resources are oxplow-foundation's declarations
//! (its manifest's `resources:`); a test there pins that every URI built
//! here is one it declares.

use std::collections::BTreeMap;
use std::str::FromStr;

use crate::ids::{AgentSessionId, AgentTurnId, CommentId, EffortId, HandoffId, StreamId, ThreadId};
use crate::uri::oxplow_uri::{OxplowUri, ResourceName};
use crate::uri::resources as r;

/// `resource` (`oxplow/effort`, `acme/pr`) with `params`, canonical: what
/// every builder below is. A producer calls the typed builder for its
/// resource; this is for one it only knows at run time (an extension's).
///
/// # Panics
/// On an invalid resource name or param key: a builder is called with a
/// resource constant and literal keys, so either is a bug at the call
/// site.
pub fn of(resource: &str, params: &[(&str, &str)]) -> String {
    of_with_view(resource, params, &[])
}

/// [`of`] with a view (`#lines=10-20`).
pub fn of_with_view(resource: &str, params: &[(&str, &str)], view: &[(&str, &str)]) -> String {
    uri(resource, params, view).to_string()
}

fn uri(resource: &str, params: &[(&str, &str)], view: &[(&str, &str)]) -> OxplowUri {
    let name: ResourceName = resource
        .parse()
        .unwrap_or_else(|e| panic!("building a `{resource}` URI: {e}"));
    let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    OxplowUri::new(name, map(params), map(view))
}

/// The one param that names one of `resource`'s, when one does: a
/// contribution's id is its own name's param (`oxplow/lens` → `lens`), a
/// stored thing's is `id`, and a few say what they are (`commit` → `sha`).
/// `None` for a resource named by several (a file's `path` and `rev`, a
/// work item's `provider` and `id`), which has its own readers.
pub fn key_param(resource: &str) -> Option<String> {
    let (extension, name) = resource.split_once('/')?;
    if extension != "oxplow" {
        return Some("id".to_string());
    }
    if crate::contribution::KINDS
        .iter()
        .any(|k| k.kind.segment() == name)
    {
        return Some(name.to_string());
    }
    Some(
        match name {
            "commit" => "sha",
            "branch" | "ai_model" | "extension" | "consumer" => "name",
            "wiki" => "slug",
            "config" => "key",
            "nudge" => "kind",
            "file"
            | "dir"
            | "symbol"
            | "work_item"
            | "subagent"
            | "block"
            | "test"
            | "extension_deployment" => return None,
            _ => "id",
        }
        .to_string(),
    )
}

/// An author's short form of one of oxplow's single-key resources —
/// `<name>:<key>` (`effect:acme/notify`, `oxplow:oxplow/wiki?slug=architecture`,
/// `oxplow:oxplow/commit?sha=4c44d495`) — as its URI; `None` for anything else. What a
/// wikilink or a manifest's `after:` may say instead of the whole URI;
/// what is stored is always the URI.
pub fn expand_short(text: &str) -> Option<String> {
    if text.starts_with("oxplow:") {
        return None;
    }
    let (name, value) = text.split_once(':')?;
    let resource = format!("oxplow/{name}");
    resource.parse::<ResourceName>().ok()?;
    let key = key_param(&resource)?;
    (!value.is_empty()).then(|| of(&resource, &[(&key, value)]))
}

/// Why a string isn't a URI of the resource its reader expects.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` isn't an {resource} URI (oxplow:{resource}?…)")]
pub struct NotA {
    pub text: String,
    pub resource: String,
}

fn not_a(text: &str, resource: &str) -> NotA {
    NotA {
        text: text.to_string(),
        resource: resource.to_string(),
    }
}

/// `text` parsed, when it is a URI of `resource`: its view kept.
pub fn parse_resource(text: &str, resource: &str) -> Result<OxplowUri, NotA> {
    OxplowUri::parse(text)
        .ok()
        .filter(|u| u.resource().to_string() == resource)
        .ok_or_else(|| not_a(text, resource))
}

/// The decoded value of `text`'s param `name`, when `text` is a URI of
/// `resource` that has it.
pub fn param_of(text: &str, resource: &str, name: &str) -> Result<String, NotA> {
    parse_resource(text, resource)?
        .param(name)
        .map(str::to_string)
        .ok_or_else(|| not_a(text, resource))
}

/// The value of the one param that names a `resource` (its [`key_param`])
/// in `text`, which names nothing else: no other param and no view (a
/// reader that wants them calls [`parse_resource`]).
pub fn id_of(text: &str, resource: &str) -> Result<String, NotA> {
    let key = key_param(resource).ok_or_else(|| not_a(text, resource))?;
    let u = parse_resource(text, resource)?;
    if u.params().len() != 1 || !u.view().is_empty() {
        return Err(not_a(text, resource));
    }
    u.param(&key)
        .map(str::to_string)
        .ok_or_else(|| not_a(text, resource))
}

/// The key of a `resource` URI, parsed as `T` (`oxplow:oxplow/effort?id=eff12`
/// → `EffortId(12)`, a decision's → `3`).
pub fn parse_as<T: FromStr>(text: &str, resource: &str) -> Result<T, NotA> {
    id_of(text, resource)?
        .parse()
        .map_err(|_| not_a(text, resource))
}

/// The resource a URI names (`oxplow/wiki`), or `None` for a string that
/// isn't one of oxplow's URIs.
pub fn resource_of(text: &str) -> Option<String> {
    OxplowUri::parse(text)
        .ok()
        .map(|u| u.resource().to_string())
}

/// The URI of a `resource` named by its key param alone ([`key_param`]):
/// `keyed("oxplow/measure", "oxplow.loc")`.
///
/// # Panics
/// On a resource without a key param (`oxplow/file`, …): those have
/// their own builders.
pub fn keyed(resource: &str, value: &str) -> String {
    let key = key_param(resource).expect("a keyed resource");
    of(resource, &[(&key, value)])
}

pub fn project_uri(name: &str) -> String {
    keyed(r::PROJECT, name)
}

pub fn stream_uri(id: StreamId) -> String {
    keyed(r::STREAM, &id.to_string())
}

pub fn thread_uri(id: ThreadId) -> String {
    keyed(r::THREAD, &id.to_string())
}

pub fn effort_uri(id: EffortId) -> String {
    keyed(r::EFFORT, &id.to_string())
}

/// The effort an effort's URI names, the inverse of [`effort_uri`];
/// `None` for any other string, a bare id included.
pub fn effort_of_uri(text: &str) -> Option<EffortId> {
    parse_as(text, r::EFFORT).ok()
}

pub fn turn_uri(id: AgentTurnId) -> String {
    keyed(r::TURN, &id.to_string())
}

/// A snapshot (`snapshot.id`).
pub fn snapshot_uri(id: i64) -> String {
    keyed(r::SNAPSHOT, &id.to_string())
}

/// A commit by sha (7–64 lowercase hex).
pub fn commit_uri(sha: &str) -> String {
    keyed(r::COMMIT, sha)
}

pub fn branch_uri(name: &str) -> String {
    keyed(r::BRANCH, name)
}

/// A file by its workspace-relative path, in the working tree.
pub fn file_uri(path: &str) -> String {
    of(r::FILE, &[("path", path)])
}

/// A file at a revision (`git:HEAD`, `snap:01J9…`), optionally lines of
/// it (`10-20`, the view).
pub fn file_uri_at(path: &str, rev: Option<&str>, lines: Option<&str>) -> String {
    let mut params = vec![("path", path)];
    params.extend(rev.map(|rev| ("rev", rev)));
    let view: Vec<(&str, &str)> = lines.map(|l| ("lines", l)).into_iter().collect();
    of_with_view(r::FILE, &params, &view)
}

/// A file URI's path and revision (`None`: the working tree), with its
/// lines when its view names them.
pub fn file_of_uri(text: &str) -> Result<(String, Option<String>, Option<String>), NotA> {
    let u = parse_resource(text, r::FILE)?;
    let path = u
        .param("path")
        .ok_or_else(|| not_a(text, r::FILE))?
        .to_string();
    Ok((
        path,
        u.param("rev").map(str::to_string),
        u.view().get("lines").cloned(),
    ))
}

pub fn dir_uri(path: &str) -> String {
    of(r::DIR, &[("path", path)])
}

/// A symbol: `id` its symbol id within the file at `path`
/// (`oxplow_code_metrics::Definition::id`: `Parser::parse`, `main~2`).
/// The current symbol; pin a version with [`symbol_uri_at`].
pub fn symbol_uri(path: &str, id: &str) -> String {
    symbol_uri_at(path, id, None)
}

/// A symbol at a revision (`snap:<id>`, `git:<sha>`).
pub fn symbol_uri_at(path: &str, id: &str, rev: Option<&str>) -> String {
    let mut params = vec![("path", path), ("symbol", id)];
    params.extend(rev.map(|rev| ("rev", rev)));
    of(r::SYMBOL, &params)
}

pub fn wiki_uri(slug: &str) -> String {
    keyed(r::WIKI, slug)
}

pub fn comment_uri(id: CommentId) -> String {
    keyed(r::COMMENT, &id.to_string())
}

/// A logged event, by its public id (UUIDv7).
pub fn event_uri(id: &str) -> String {
    keyed(r::EVENT, id)
}

/// A page (`tasks`, `<extension>/<id>`) with its params.
pub fn page_uri(page: &str, params: &[(&str, &str)]) -> String {
    let mut all = vec![("page", page)];
    all.extend_from_slice(params);
    of(r::PAGE, &all)
}

/// An ephemeral page an agent showed, by its id.
pub fn view_uri(id: &str) -> String {
    keyed(r::VIEW, id)
}

/// An extension's page (`<extension>/<id>`).
pub fn extension_page_uri(extension: &str, page: &str) -> String {
    page_uri(&format!("{extension}/{page}"), &[])
}

/// A metric by its key.
pub fn metric_uri(key: &str) -> String {
    keyed(r::METRIC, key)
}

/// One of core's published models by its name (`effort`, published as
/// `v_effort`); an extension's is `<extension>/<name>`.
pub fn model_uri(name: &str) -> String {
    keyed(r::MODEL, name)
}

/// One run of a declared job (`v_job_run.id`).
pub fn job_run_uri(id: &str) -> String {
    keyed(r::JOB_RUN, id)
}

/// One recorded model call (`v_ai_call.id`).
pub fn ai_call_uri(id: i64) -> String {
    keyed(r::AI_CALL, &id.to_string())
}

/// A language model by the name its harness or provider reports
/// (`claude-opus-4-8`); not a `model`, a published view.
pub fn ai_model_uri(name: &str) -> String {
    keyed(r::AI_MODEL, name)
}

/// An extension, by its name.
pub fn extension_uri(name: &str) -> String {
    keyed(r::EXTENSION, name)
}

/// A command by name (`oxplow.config.set`).
pub fn command_uri(name: &str) -> String {
    keyed(r::COMMAND, name)
}

/// A `.oxplow/project.yaml` key (`zones`, `metricRetentionDays`).
pub fn config_uri(key: &str) -> String {
    keyed(r::CONFIG, key)
}

/// A code-quality finding.
pub fn finding_uri(id: &str) -> String {
    keyed(r::FINDING, id)
}

/// Work handed from one thread to another (`handoff.id`).
pub fn handoff_uri(id: HandoffId) -> String {
    keyed(r::HANDOFF, &id.to_string())
}

/// An agent slot on a thread (`agent_session.id`).
pub fn agent_session_uri(id: AgentSessionId) -> String {
    keyed(r::AGENT_SESSION, &id.to_string())
}

/// A subagent of an agent session, by the harness's id for it.
pub fn subagent_uri(session: AgentSessionId, id: &str) -> String {
    of(
        r::SUBAGENT,
        &[("session", &session.to_string()), ("agent", id)],
    )
}

/// A terminal (`terminal.id`).
pub fn terminal_uri(id: crate::TerminalId) -> String {
    keyed(r::TERMINAL, &id.to_string())
}

/// A test or analysis run (`metric_capture.id`).
pub fn run_uri(id: i64) -> String {
    keyed(r::RUN, &id.to_string())
}

/// A command waiting for a person's decision (`command_proposal.id`).
pub fn proposal_uri(id: i64) -> String {
    keyed(r::PROPOSAL, &id.to_string())
}

/// A claim an agent made about its work (`claim.id`).
pub fn claim_uri(id: i64) -> String {
    keyed(r::CLAIM, &id.to_string())
}

/// A decision an agent recorded or oxplow inferred (`decision.id`).
pub fn decision_uri(id: i64) -> String {
    keyed(r::DECISION, &id.to_string())
}

/// A collector, by its owner and id (`collector_run.owner`/`id`).
pub fn collector_uri(owner: &str, id: &str) -> String {
    keyed(r::COLLECTOR, &format!("{owner}/{id}"))
}

/// One of core's event consumers (`change.analyze`), as an `after:` names
/// it.
pub fn consumer_uri(name: &str) -> String {
    keyed(r::CONSUMER, name)
}

/// A code block (`<start>-<end>` lines of a file), a fact's subject.
pub fn block_uri(path: &str, lines: &str) -> String {
    of(r::BLOCK, &[("path", path), ("lines", lines)])
}

/// A test case, a fact's subject.
pub fn test_uri(class: &str, name: &str) -> String {
    of(r::TEST, &[("class", class), ("name", name)])
}

/// A work list's item: its provider's id and the item's.
pub fn work_item_uri(provider: &str, id: &str) -> String {
    of(r::WORK_ITEM, &[("provider", provider), ("id", id)])
}

/// The provider and the provider's id a work item's URI names.
pub fn work_item_of_uri(text: &str) -> Option<(String, String)> {
    let u = parse_resource(text, r::WORK_ITEM).ok()?;
    Some((u.param("provider")?.to_string(), u.param("id")?.to_string()))
}

/// A work item's URI in its one canonical spelling, or `Invalid` naming
/// what's wrong with it. Efforts key on the string (one open effort per
/// work item is a unique index), so an alias — a view, an extra param, an
/// escaped spelling — is refused rather than treated as a different item.
/// What a list's own ids look like is that list's to say.
pub fn validate_work_item_uri(text: &str) -> Result<(), crate::DomainError> {
    let invalid = |why: &str| Err(crate::DomainError::Invalid(format!("`{text}` {why}")));
    const SHAPE: &str =
        "a work item's URI is `oxplow:oxplow/work_item?id=<id>&provider=<provider>`";
    let parsed = OxplowUri::parse(text).map_err(|e| {
        crate::DomainError::Invalid(format!("`{text}` is not a URI: {}; {SHAPE}", e.reason()))
    })?;
    if parsed.resource().to_string() != r::WORK_ITEM {
        return invalid(&format!("is not a work item's; {SHAPE}"));
    }
    let provider_ok = parsed.param("provider").is_some_and(|provider| {
        provider.starts_with(|c: char| c.is_ascii_lowercase())
            && provider
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    });
    if !provider_ok || parsed.param("id").is_none_or(str::is_empty) {
        return invalid(&format!("names no provider and id; {SHAPE}"));
    }
    if parsed.params().len() != 2 || !parsed.view().is_empty() {
        return invalid("names a work item with other params or a view");
    }
    if parsed.to_string() != text {
        return invalid(&format!("is not canonical; write `{parsed}`"));
    }
    Ok(())
}

/// The `source` of an event a system component emits on its own behalf
/// (`system:snapshot_capture`); an actor's runs use `Actor::source()`.
///
/// A source says who emitted an event. It is its own vocabulary —
/// `system:…`, a provider instance, an effort policy are no URIs — so the
/// sources are built here, beside the URIs, and nowhere else.
pub fn system_source(component: &str) -> String {
    format!("system:{component}")
}

/// An effect's runs (`effect:<extension>/<id>`) or an effort policy's
/// (`effect:effort_policy:<id>`).
pub fn effect_source(effect: &str) -> String {
    format!("effect:{effect}")
}

/// The effect a source names, when an effect emitted it.
pub fn effect_of_source(source: &str) -> Option<&str> {
    source.strip_prefix("effect:")
}

/// A provider instance's events (`provider:oxplow`, `provider:github/acme`).
pub fn provider_source(instance: &str) -> String {
    format!("provider:{instance}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An effort payload carries the URI, and every consumer reads it back
    /// through this one inverse.
    #[test]
    fn an_effort_uri_reads_back_to_its_effort() {
        let id = EffortId::try_from_str("eff12").unwrap();
        assert_eq!(effort_uri(id), "oxplow:oxplow/effort?id=eff12");
        assert_eq!(effort_of_uri(&effort_uri(id)), Some(id));
        assert_eq!(effort_of_uri("eff12"), None, "a bare id isn't a URI");
        assert_eq!(effort_of_uri(&turn_uri(AgentTurnId::new(12))), None);
    }

    #[test]
    fn a_page_carries_its_own_params() {
        assert_eq!(
            page_uri("settings", &[("area", "ai")]),
            "oxplow:oxplow/page?area=ai&page=settings"
        );
        assert_eq!(
            extension_page_uri("github", "pr"),
            "oxplow:oxplow/page?page=github/pr"
        );
    }

    #[test]
    fn a_shaped_uri_names_its_parts() {
        assert_eq!(file_uri("src/a@b.rs"), "oxplow:oxplow/file?path=src/a@b.rs");
        assert_eq!(
            file_uri_at("src/a.rs", Some("git:HEAD"), Some("10-20")),
            "oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-20"
        );
        assert_eq!(
            file_of_uri("oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-20").unwrap(),
            (
                "src/a.rs".to_string(),
                Some("git:HEAD".to_string()),
                Some("10-20".to_string())
            )
        );
        assert_eq!(
            symbol_uri_at("src/a.rs", "Parser::parse", Some("snap:812")),
            "oxplow:oxplow/symbol?path=src/a.rs&rev=snap:812&symbol=Parser::parse"
        );
        assert_eq!(
            subagent_uri(AgentSessionId::new(3), "agent-7f"),
            "oxplow:oxplow/subagent?agent=agent-7f&session=ses3"
        );
        assert_eq!(
            collector_uri("triage", "todos"),
            "oxplow:oxplow/collector?collector=triage/todos"
        );
        assert_eq!(commit_uri("4c44d495"), "oxplow:oxplow/commit?sha=4c44d495");
        assert_eq!(wiki_uri("auth-flow"), "oxplow:oxplow/wiki?slug=auth-flow");
    }

    /// A reader gets the decoded key, typed, or says which resource it
    /// wanted.
    #[test]
    fn a_uri_reads_back_through_the_grammar() {
        assert_eq!(id_of(&wiki_uri("a b"), r::WIKI).unwrap(), "a b");
        assert_eq!(parse_as::<i64>(&decision_uri(3), r::DECISION).unwrap(), 3);
        assert_eq!(
            parse_as::<ThreadId>(&thread_uri(ThreadId::new(3)), r::THREAD).unwrap(),
            ThreadId::new(3)
        );
        let e = parse_as::<i64>(&claim_uri(3), r::DECISION).unwrap_err();
        assert_eq!(
            e.to_string(),
            "`oxplow:oxplow/claim?id=3` isn't an oxplow/decision URI (oxplow:oxplow/decision?…)"
        );
        assert!(
            id_of(&file_uri("a.rs"), r::FILE).is_err(),
            "a file has no single key"
        );
        assert!(
            id_of("oxplow:oxplow/wiki?slug=a#part=x", r::WIKI).is_err(),
            "a view needs parse_resource"
        );
        assert_eq!(resource_of(&wiki_uri("x")).as_deref(), Some("oxplow/wiki"));
        assert_eq!(resource_of("nope"), None);
        assert_eq!(key_param("acme/pr").as_deref(), Some("id"));
        assert_eq!(key_param("oxplow/lens").as_deref(), Some("lens"));
    }

    #[test]
    fn the_work_item_inverses() {
        assert_eq!(system_source("hook_ingest"), "system:hook_ingest");
        let item = work_item_uri("issues", "ENG-1");
        assert_eq!(item, "oxplow:oxplow/work_item?id=ENG-1&provider=issues");
        assert_eq!(
            work_item_of_uri(&item),
            Some(("issues".into(), "ENG-1".into()))
        );
        assert_eq!(work_item_of_uri(&effort_uri(EffortId::new(1))), None);
        assert!(validate_work_item_uri(&item).is_ok());
        for bad in ["", "tsk1", "oxplow:tsk1097", "oxplow:oxplow/work_item?id=x"] {
            let e = validate_work_item_uri(bad).unwrap_err().to_string();
            assert!(
                e.contains("oxplow:oxplow/work_item?id=<id>&provider=<provider>"),
                "{bad}: {e}"
            );
        }
        // Only the canonical spelling is a key: the open-effort index
        // compares strings, so a view, an extra param or an escaped
        // spelling would slip a second open effort past it.
        for alias in [
            "oxplow:oxplow/work_item?id=tsk1&provider=oxplow#x=1",
            "oxplow:oxplow/work_item?id=tsk1&provider=oxplow&v=2",
            "oxplow:oxplow/work_item?provider=oxplow&id=tsk1",
            "oxplow:oxplow/work_item?id=ts%6B1&provider=oxplow",
        ] {
            assert!(validate_work_item_uri(alias).is_err(), "{alias}");
        }
    }
}
