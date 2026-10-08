//! Canonical ref builders (P2.4b, tsk450; `.context/refs.md`).
//!
//! Every producer that names an entity in an event subject, a payload or
//! a `page_ref` row builds the ref here, so the shape (`stream:str1`,
//! `turn:trn3`, `work_item:oxplow:tsk42`) lives in one place instead of a
//! `format!` per call site. A work list builds its own items' refs.

use crate::ids::{AgentSessionId, AgentTurnId, EffortId, StreamId, ThreadId};

pub fn stream_ref(id: StreamId) -> String {
    format!("stream:{id}")
}

pub fn thread_ref(id: ThreadId) -> String {
    format!("thread:{id}")
}

/// A command waiting for a person's decision (`command_proposal.id`).
pub fn proposal_ref(id: i64) -> String {
    format!("proposal:{id}")
}

/// A claim an agent made about its work (`claim.id`).
pub fn claim_ref(id: i64) -> String {
    format!("claim:{id}")
}

/// A collector, by its owner and id (`collector_run.owner`/`id`).
pub fn collector_ref(owner: &str, id: &str) -> String {
    format!("collector:{owner}/{id}")
}

/// A decision an agent recorded or oxplow inferred (`decision.id`).
pub fn decision_ref(id: i64) -> String {
    format!("decision:{id}")
}

/// An agent's answer in a thread (`thread_answer.id`).
pub fn answer_ref(id: i64) -> String {
    format!("answer:{id}")
}

/// A lens (`<extension>/<slug>`).
pub fn lens_ref(id: &str) -> String {
    format!("lens:{id}")
}

/// An extension, by its name.
pub fn extension_ref(name: &str) -> String {
    format!("extension:{name}")
}

pub fn effort_ref(id: EffortId) -> String {
    format!("effort:{id}")
}

/// The effort behind an `effort:` ref (`effort:eff12`), the inverse of
/// [`effort_ref`]; `None` for any other string, a bare id included.
pub fn effort_of_ref(r: &str) -> Option<EffortId> {
    r.strip_prefix("effort:").and_then(EffortId::try_from_str)
}

/// An agent slot on a thread (`agent_session.id`).
pub fn agent_session_ref(id: AgentSessionId) -> String {
    format!("agent_session:{id}")
}

pub fn turn_ref(id: AgentTurnId) -> String {
    format!("turn:{id}")
}

/// A snapshot (`snapshot.id`).
pub fn snapshot_ref(id: i64) -> String {
    format!("snapshot:{id}")
}

/// A commit by sha (7–40 lowercase hex).
pub fn commit_ref(sha: &str) -> String {
    format!("commit:{sha}")
}

/// A command by name (`oxplow.config.set`).
pub fn command_ref(name: &str) -> String {
    format!("command:{name}")
}

/// A `.oxplow/project.yaml` key (`zones`, `metricRetentionDays`).
pub fn config_ref(key: &str) -> String {
    format!("config:{key}")
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
pub fn system_source(component: &str) -> String {
    format!("system:{component}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::grammar::CanonicalRef;
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
    fn every_builder_produces_a_valid_canonical_ref() {
        let reg = core_kinds();
        for r in [
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
