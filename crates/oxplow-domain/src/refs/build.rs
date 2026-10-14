//! Canonical ref builders and readers (`.context/refs.md` "Using refs").
//!
//! Every Rust producer that names a thing — in an event subject, a
//! payload, a `page_ref` row, a command's input — builds the ref here, and
//! every reader reads one back through [`parse_as`] / [`id_of`] /
//! [`parse_kind`], so a kind's prefix is spelled once
//! (`refs::kind::{EFFORT, FILE, …}`) and an id is always escaped and
//! decoded by the grammar. A work list builds its own items' refs
//! (`oxplow_tasks::work_item_ref`). The `refs_are_built_and_parsed_through_the_grammar`
//! source guard refuses a hand-built or hand-parsed ref anywhere else.

use std::str::FromStr;

use crate::ids::{AgentSessionId, AgentTurnId, CommentId, EffortId, NoteId, StreamId, ThreadId};
use crate::refs::grammar::{CanonicalRef, RefParseError};
use crate::refs::kind as k;

/// `<kind>:<id>`, the id escaped as the grammar needs: what every builder
/// below is. A producer calls the typed builder for its kind; this is
/// for a kind it only knows at run time (an extension's).
///
/// # Panics
/// On an invalid kind name or an empty id: a builder is called with a
/// kind constant and a real id, so either is a bug at the call site.
pub fn of(kind: &str, id: &str) -> String {
    at(kind, id, None, None)
}

/// [`of`] with a revision (`git:HEAD`, `snap:01J9…`) and a fragment
/// (`L10-20`).
pub fn at(kind: &str, id: &str, rev: Option<&str>, frag: Option<&str>) -> String {
    CanonicalRef::new(kind, id, rev, frag)
        .unwrap_or_else(|e| panic!("building a `{kind}` ref of `{id}`: {e}"))
        .to_string()
}

/// Why a string isn't a ref of the kind its reader expects.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{text}` isn't a {kind} ref ({kind}:<id>)")]
pub struct NotA {
    pub text: String,
    pub kind: String,
}

/// `r` parsed, when it is a ref of `kind`: revision and fragment kept.
pub fn parse_kind(r: &str, kind: &str) -> Result<CanonicalRef, NotA> {
    CanonicalRef::parse(r)
        .ok()
        .filter(|c| c.kind == kind)
        .ok_or_else(|| NotA {
            text: r.to_string(),
            kind: kind.to_string(),
        })
}

/// The decoded id of a `kind` ref (`file:a%40b.rs` → `a@b.rs`). A
/// revision or fragment is refused: a reader that wants them calls
/// [`parse_kind`].
pub fn id_of(r: &str, kind: &str) -> Result<String, NotA> {
    let parsed = parse_kind(r, kind)?;
    if parsed.rev.is_some() || parsed.frag.is_some() {
        return Err(NotA {
            text: r.to_string(),
            kind: kind.to_string(),
        });
    }
    Ok(parsed.id)
}

/// The id of a `kind` ref, parsed as `T` (`effort:eff12` → `EffortId(12)`,
/// `decision:3` → `3`).
pub fn parse_as<T: FromStr>(r: &str, kind: &str) -> Result<T, NotA> {
    id_of(r, kind)?.parse().map_err(|_| NotA {
        text: r.to_string(),
        kind: kind.to_string(),
    })
}

/// The kind a ref names, or `None` for a string that isn't a ref.
pub fn kind_of(r: &str) -> Option<String> {
    CanonicalRef::parse(r).ok().map(|c| c.kind)
}

pub fn project_ref(name: &str) -> String {
    of(k::PROJECT, name)
}

pub fn stream_ref(id: StreamId) -> String {
    of(k::STREAM, &id.to_string())
}

pub fn thread_ref(id: ThreadId) -> String {
    of(k::THREAD, &id.to_string())
}

pub fn effort_ref(id: EffortId) -> String {
    of(k::EFFORT, &id.to_string())
}

/// The effort behind an `effort:` ref (`effort:eff12`), the inverse of
/// [`effort_ref`]; `None` for any other string, a bare id included.
pub fn effort_of_ref(r: &str) -> Option<EffortId> {
    parse_as(r, k::EFFORT).ok()
}

pub fn turn_ref(id: AgentTurnId) -> String {
    of(k::TURN, &id.to_string())
}

/// A snapshot (`snapshot.id`).
pub fn snapshot_ref(id: i64) -> String {
    of(k::SNAPSHOT, &id.to_string())
}

/// A commit by sha (7–64 lowercase hex).
pub fn commit_ref(sha: &str) -> String {
    of(k::COMMIT, sha)
}

pub fn branch_ref(name: &str) -> String {
    of(k::BRANCH, name)
}

/// A file by its workspace-relative path, in the working tree.
pub fn file_ref(path: &str) -> String {
    of(k::FILE, path)
}

/// A file at a revision (`git:HEAD`, `snap:01J9…`), optionally lines of
/// it (`L10-20`).
pub fn file_ref_at(path: &str, rev: Option<&str>, lines: Option<&str>) -> String {
    at(k::FILE, path, rev, lines)
}

pub fn dir_ref(path: &str) -> String {
    of(k::DIR, path)
}

/// A symbol: `<path>/<id>`, `id` its symbol id within the file
/// (`oxplow_code_metrics::Definition::id`: `Parser::parse`, `main~2`).
/// The current symbol; pin a version with [`at`] (`@snap:<id>`,
/// `@git:<sha>`).
pub fn symbol_ref(path: &str, id: &str) -> String {
    of(k::SYMBOL, &format!("{path}/{id}"))
}

pub fn wiki_ref(slug: &str) -> String {
    of(k::WIKI, slug)
}

pub fn comment_ref(id: CommentId) -> String {
    of(k::COMMENT, &id.to_string())
}

/// A logged event, by its public id (UUIDv7).
pub fn event_ref(id: &str) -> String {
    of(k::EVENT, id)
}

/// A lens (`<extension>/<slug>`).
pub fn lens_ref(id: &str) -> String {
    of(k::LENS, id)
}

/// A lens ref carrying params (`lens:acme/blocked?stream_id=2`): the
/// query follows the ref raw, each key and value escaped like a URL
/// query (the desktop's `withParams`), so `splitParams` reads it back.
/// A param with no value is left out.
///
/// # Errors
/// On an empty id.
pub fn lens_ref_with(id: &str, params: &[(&str, Option<&str>)]) -> Result<String, RefParseError> {
    CanonicalRef::new(k::LENS, id, None, None)?;
    let head = lens_ref(id);
    let query: Vec<String> = params
        .iter()
        .filter_map(|(key, value)| {
            Some(format!("{}={}", query_escape(key), query_escape((*value)?)))
        })
        .collect();
    Ok(if query.is_empty() {
        head
    } else {
        format!("{head}?{}", query.join("&"))
    })
}

/// `encodeURIComponent`, keeping `/ : ? ,` and writing a space as `+`.
fn query_escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => out.push(b as char),
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            b'/' | b':' | b'?' | b',' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// An agent's answer in a thread (`thread_answer.id`).
pub fn answer_ref(id: i64) -> String {
    of(k::ANSWER, &id.to_string())
}

/// A shell route (`settings`), optionally with its query string.
pub fn page_ref(name: &str, query: Option<&str>) -> String {
    match query {
        Some(q) if !q.is_empty() => of(k::PAGE, &format!("{name}?{q}")),
        _ => of(k::PAGE, name),
    }
}

/// An extension's page (the contribution `page:<extension>/<id>`).
pub fn extension_page_ref(extension: &str, page: &str) -> String {
    of(k::PAGE, &format!("{extension}/{page}"))
}

/// A metric by its key.
pub fn metric_ref(key: &str) -> String {
    of(k::METRIC, key)
}

/// One of core's published models by its name (`effort`, published as
/// `v_effort`); an extension's is the contribution
/// `model:<extension>/<name>`.
pub fn model_ref(name: &str) -> String {
    of(k::MODEL, name)
}

/// One run of a declared job (`v_job_run.id`).
pub fn job_run_ref(id: &str) -> String {
    of(k::JOB_RUN, id)
}

/// A language model by the name its harness or provider reports
/// (`ai_model:claude-opus-4-8`); not the `model` kind, a published view.
pub fn ai_model_ref(name: &str) -> String {
    of(k::AI_MODEL, name)
}

/// An extension, by its name.
pub fn extension_ref(name: &str) -> String {
    of(k::EXTENSION, name)
}

/// A command by name (`oxplow.config.set`).
pub fn command_ref(name: &str) -> String {
    of(k::COMMAND, name)
}

/// A `.oxplow/project.yaml` key (`zones`, `metricRetentionDays`).
pub fn config_ref(key: &str) -> String {
    of(k::CONFIG, key)
}

/// A code-quality finding.
pub fn finding_ref(id: &str) -> String {
    of(k::FINDING, id)
}

pub fn thread_note_ref(id: NoteId) -> String {
    of(k::THREAD_NOTE, &id.to_string())
}

/// An agent slot on a thread (`agent_session.id`).
pub fn agent_session_ref(id: AgentSessionId) -> String {
    of(k::AGENT_SESSION, &id.to_string())
}

/// A test or analysis run (`metric_capture.id`).
pub fn run_ref(id: i64) -> String {
    of(k::RUN, &id.to_string())
}

/// A command waiting for a person's decision (`command_proposal.id`).
pub fn proposal_ref(id: i64) -> String {
    of(k::PROPOSAL, &id.to_string())
}

/// A claim an agent made about its work (`claim.id`).
pub fn claim_ref(id: i64) -> String {
    of(k::CLAIM, &id.to_string())
}

/// A decision an agent recorded or oxplow inferred (`decision.id`).
pub fn decision_ref(id: i64) -> String {
    of(k::DECISION, &id.to_string())
}

/// A collector, by its owner and id (`collector_run.owner`/`id`).
pub fn collector_ref(owner: &str, id: &str) -> String {
    of(k::COLLECTOR, &format!("{owner}/{id}"))
}

/// The provider-scoped id inside a `work_item` ref (`oxplow:tsk42`,
/// `issues:ENG-12`) — the `page_ref` id of the work item.
pub fn work_item_id_of_ref(r: &str) -> Option<&str> {
    r.strip_prefix("work_item:").filter(|id| !id.is_empty())
}

/// A `work_item` ref in its one canonical spelling, or `Invalid` naming
/// what's wrong with it. Efforts key on the string (one open effort per
/// work item is a unique index), so an alias — a revision, a fragment,
/// an escaped spelling — is refused rather than treated as a different
/// item. What a list's own ids look like is that list's to say.
pub fn validate_work_item_ref(r: &str) -> Result<(), crate::DomainError> {
    let invalid = |why: &str| Err(crate::DomainError::Invalid(format!("`{r}` {why}")));
    const SHAPE: &str = "a work item ref is `work_item:<provider>:<id>` (`work_item:oxplow:tsk42`)";
    // `work_item` is core's kind, the same in every vocabulary: its shape
    // is checked here, no registry needed.
    let parsed = crate::refs::CanonicalRef::parse(r).map_err(|e| {
        crate::DomainError::Invalid(format!(
            "`{r}` is not a canonical ref: {}; {SHAPE}",
            e.reason()
        ))
    })?;
    if parsed.kind != "work_item" {
        return invalid(&format!("is not a work_item ref; {SHAPE}"));
    }
    let provider_ok = parsed.id.split_once(':').is_some_and(|(provider, native)| {
        !native.is_empty()
            && provider.starts_with(|c: char| c.is_ascii_lowercase())
            && provider
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    });
    if !provider_ok {
        return invalid(&format!("names no `<provider>:<id>`; {SHAPE}"));
    }
    if parsed.rev.is_some() || parsed.frag.is_some() {
        return invalid("names a work item with a revision or fragment");
    }
    if parsed.to_string() != r {
        return invalid(&format!("is not canonical; write `{parsed}`"));
    }
    Ok(())
}

/// The `source` of an event a system component emits on its own behalf
/// (`system:snapshot_capture`); an actor's runs use `Actor::source()`.
///
/// A source says who emitted an event. It looks like a ref but is its own
/// vocabulary — `system:…`, a provider instance, an effort policy are no
/// refs — so the sources are built here, beside the refs, and nowhere
/// else.
pub fn system_source(component: &str) -> String {
    format!("system:{component}")
}

/// An effect's runs (`effect:<extension>/<id>`, which is also its ref) or
/// an effort policy's (`effect:effort_policy:<id>`).
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
    use crate::refs::kind::core_kinds;

    /// tsk1025: an `effort.*` payload carries the ref, and every consumer
    /// reads it back through this one inverse.
    #[test]
    fn an_effort_ref_reads_back_to_its_effort() {
        let id = EffortId::try_from_str("eff12").unwrap();
        assert_eq!(effort_of_ref(&effort_ref(id)), Some(id));
        assert_eq!(effort_of_ref("eff12"), None, "a bare id isn't a ref");
        assert_eq!(effort_of_ref("turn:12"), None);
    }

    #[test]
    fn a_lens_ref_carries_its_params_escaped_like_a_query() {
        assert_eq!(
            lens_ref_with("acme/blocked", &[("stream_id", Some("2"))]).unwrap(),
            "lens:acme/blocked?stream_id=2"
        );
        assert_eq!(
            lens_ref_with(
                "acme/x",
                &[("a", Some("b c&d=e%")), ("gone", None), ("z", Some("1"))]
            )
            .unwrap(),
            "lens:acme/x?a=b+c%26d%3De%25&z=1"
        );
        assert_eq!(lens_ref_with("acme/x", &[]).unwrap(), "lens:acme/x");
        assert!(lens_ref_with("", &[]).is_err());
    }

    #[test]
    fn every_builder_produces_a_valid_canonical_ref() {
        let reg = core_kinds();
        for r in [
            project_ref("oxplow"),
            branch_ref("main"),
            file_ref("src/a@b.rs"),
            file_ref_at("src/a.rs", Some("git:HEAD"), Some("L10-20")),
            dir_ref("src/ui"),
            symbol_ref("src/a.rs", "parse"),
            symbol_ref("README.md", "Parser::parse(String,int)"),
            symbol_ref("Sources/App/Main.swift", "<Parser as Display>::fmt~2"),
            symbol_ref("src/core.clj", "area([:a :b])"),
            at(k::SYMBOL, "src/a.rs/Parser::parse", Some("snap:812"), None),
            at(
                k::SYMBOL,
                "src/a.rs/Parser::parse",
                Some("git:3f2a9c1"),
                None,
            ),
            wiki_ref("architecture"),
            comment_ref(CommentId::new(4)),
            event_ref("01a1369e-b063-76f8-968f-902d392f882d"),
            lens_ref("acme/blocked"),
            answer_ref(3),
            page_ref("settings", None),
            page_ref("dashboard", Some("variant=visits")),
            extension_page_ref("github", "pr"),
            metric_ref("oxplow.rust.unsafe_blocks"),
            model_ref("effort"),
            job_run_ref("01a1369e-b063-76f8-968f-902d392f882d"),
            extension_ref("effort-review"),
            finding_ref("clippy:1"),
            thread_note_ref(NoteId::new(5)),
            agent_session_ref(AgentSessionId::new(2)),
            run_ref(9),
            claim_ref(1),
            decision_ref(2),
            collector_ref("triage", "todos"),
            stream_ref(StreamId::new(1)),
            thread_ref(ThreadId::new(3)),
            effort_ref(EffortId::new(12)),
            turn_ref(AgentTurnId::new(7)),
            snapshot_ref(42),
            commit_ref("4c44d495"),
            command_ref("oxplow.config.set"),
            config_ref("metricRetentionDays"),
            "work_item:oxplow:tsk42".to_string(),
            proposal_ref(12),
        ] {
            let parsed = CanonicalRef::parse(&r).unwrap_or_else(|e| panic!("{r}: {}", e.reason()));
            reg.validate(&parsed).unwrap_or_else(|e| panic!("{r}: {e}"));
        }
        assert_eq!(stream_ref(StreamId::new(1)), "stream:str1");
        assert_eq!(turn_ref(AgentTurnId::new(7)), "turn:trn7");
        assert_eq!(proposal_ref(12), "proposal:12");
        assert_eq!(file_ref("src/a@b.rs"), "file:src/a%40b.rs", "escaped");
    }

    /// Every core kind has a builder: the test above names one ref per
    /// kind `core_kinds()` registers.
    #[test]
    fn every_core_kind_has_a_builder() {
        let built: std::collections::BTreeSet<String> = [
            project_ref("p"),
            stream_ref(StreamId::new(1)),
            thread_ref(ThreadId::new(1)),
            effort_ref(EffortId::new(1)),
            turn_ref(AgentTurnId::new(1)),
            snapshot_ref(1),
            commit_ref("4c44d495"),
            branch_ref("b"),
            file_ref("a"),
            dir_ref("a"),
            symbol_ref("a", "b"),
            "work_item:oxplow:tsk1".to_string(),
            wiki_ref("w"),
            comment_ref(CommentId::new(1)),
            event_ref("01a1369e-b063-76f8-968f-902d392f882d"),
            lens_ref("a/b"),
            answer_ref(1),
            page_ref("p", None),
            metric_ref("m"),
            model_ref("v_a"),
            job_run_ref("01a1369e-b063-76f8-968f-902d392f882d"),
            ai_model_ref("claude-opus-4-8"),
            extension_ref("e"),
            command_ref("a.b"),
            config_ref("c"),
            finding_ref("f"),
            thread_note_ref(NoteId::new(1)),
            agent_session_ref(AgentSessionId::new(1)),
            run_ref(1),
            proposal_ref(1),
            claim_ref(1),
            decision_ref(1),
            collector_ref("a", "b"),
        ]
        .iter()
        .map(|r| kind_of(r).unwrap())
        .collect();
        // A contribution's ref is its id's (`ContributionId::reference`).
        let contributions: std::collections::BTreeSet<String> = crate::contribution::KINDS
            .iter()
            .map(|k| k.kind.segment().to_string())
            .collect();
        let registered: std::collections::BTreeSet<String> =
            core_kinds().kinds().map(|k| k.kind.clone()).collect();
        let core: std::collections::BTreeSet<String> =
            registered.difference(&contributions).cloned().collect();
        let built_core: std::collections::BTreeSet<String> =
            built.difference(&contributions).cloned().collect();
        assert_eq!(built_core, core);
    }

    /// A reader gets the decoded id, typed, or says which kind it wanted.
    #[test]
    fn a_ref_reads_back_through_the_grammar() {
        assert_eq!(id_of("file:a%40b.rs", k::FILE).unwrap(), "a@b.rs");
        assert_eq!(parse_as::<i64>("decision:3", k::DECISION).unwrap(), 3);
        assert_eq!(
            parse_as::<ThreadId>("thread:thr3", k::THREAD).unwrap(),
            ThreadId::new(3)
        );
        let e = parse_as::<i64>("claim:3", k::DECISION).unwrap_err();
        assert_eq!(
            e.to_string(),
            "`claim:3` isn't a decision ref (decision:<id>)"
        );
        assert!(
            id_of("file:a.rs@git:HEAD", k::FILE).is_err(),
            "a revision needs parse_kind"
        );
        let at = parse_kind("file:a.rs@git:HEAD#L1", k::FILE).unwrap();
        assert_eq!(
            (at.rev.as_deref(), at.frag.as_deref()),
            (Some("git:HEAD"), Some("L1"))
        );
        assert_eq!(kind_of("wiki:x").as_deref(), Some("wiki"));
        assert_eq!(kind_of("nope"), None);
    }

    #[test]
    fn the_work_item_inverses() {
        assert_eq!(system_source("hook_ingest"), "system:hook_ingest");
        assert_eq!(
            work_item_id_of_ref("work_item:issues:ENG-1"),
            Some("issues:ENG-1")
        );
        assert_eq!(work_item_id_of_ref("work_item:"), None);
        assert_eq!(work_item_id_of_ref("effort:eff1"), None);
        assert!(validate_work_item_ref("work_item:issues:ENG-1").is_ok());
        for bad in ["", "tsk1", "effort:eff1", "work_item:"] {
            assert!(validate_work_item_ref(bad).is_err(), "{bad}");
        }
        // A wrong shape shows the right one.
        for bad in ["work_item:oxplow/tsk1097", "oxplow:tsk1097", "effort:eff1"] {
            let e = validate_work_item_ref(bad).unwrap_err().to_string();
            assert!(e.contains("`work_item:<provider>:<id>`"), "{e}");
        }
        // Only the canonical spelling is a key: the open-effort index
        // compares strings, so a fragment, a revision or an escaped
        // spelling would slip a second open effort past it. (What a list's
        // own ids look like is that list's to say, not core's.)
        for alias in [
            "work_item:oxplow:tsk1#x",
            "work_item:oxplow:tsk1@v2",
            "work_item:issues:ENG%2D1",
        ] {
            assert!(validate_work_item_ref(alias).is_err(), "{alias}");
        }
    }
}
