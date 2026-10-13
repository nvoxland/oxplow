//! Which published model owns each core ref kind (`.context/refs.md`
//! "Using refs"): the one model whose SQL spells the kind's prefix, in
//! its `ref` column, and the column that names one of its rows. Everyone
//! else selects or joins on that `ref` — a lens, a script, another model
//! never rebuilds a ref from an id. The resolver reads a kind's title
//! here (an extension's kind names its own `resolve` view in
//! `v_ref_kind`), and the source guard over SQL exempts these owners.

/// A core kind's owning model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefModel {
    /// The ref kind (`refs::kind::EFFORT`).
    pub kind: &'static str,
    /// Its published view (`v_effort`), which has a `ref` column.
    pub view: &'static str,
    /// The column that names a row to a person.
    pub title: &'static str,
}

use oxplow_domain::refs::kind as k;

const fn owner(kind: &'static str, view: &'static str, title: &'static str) -> RefModel {
    RefModel { kind, view, title }
}

/// Every core kind whose rows are a published model's.
pub const REF_MODELS: &[RefModel] = &[
    owner(k::STREAM, "v_stream", "title"),
    owner(k::THREAD, "v_thread", "title"),
    owner(k::EFFORT, "v_effort", "title"),
    owner(k::TURN, "v_agent_turn", "prompt"),
    owner(k::SNAPSHOT, "v_snapshot", "created_at"),
    owner(k::COMMIT, "v_commit", "subject"),
    owner(k::SYMBOL, "v_symbol", "name"),
    owner(k::WORK_ITEM, "v_work_item", "title"),
    owner(k::WIKI, "v_knowledge_page", "title"),
    owner(k::COMMENT, "v_comment", "body"),
    owner(k::EVENT, "v_event", "type"),
    owner(k::ANSWER, "v_thread_answer", "title"),
    owner(k::THREAD_NOTE, "v_thread_note", "body"),
    owner(k::AGENT_SESSION, "v_agent_session", "title"),
    owner(k::PROPOSAL, "v_command_proposal", "command"),
    owner(k::CLAIM, "v_claim", "statement"),
    owner(k::DECISION, "v_decision", "question"),
    owner(k::COLLECTOR, "v_collector_run", "id"),
    owner(k::JOB_RUN, "v_job_run", "intent_key"),
];

/// The owning model of `kind`, when a core model owns it.
pub fn owner_of(kind: &str) -> Option<&'static RefModel> {
    REF_MODELS.iter().find(|m| m.kind == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each owner is a published view with a `ref` column of its kind and
    /// its title column: the resolver's query compiles on a fresh
    /// database, and every row's ref names the kind.
    #[tokio::test]
    async fn every_owner_has_a_ref_of_its_kind_and_a_title() {
        let db = crate::Database::in_memory();
        db.read(|c| {
            for m in REF_MODELS {
                let sql = format!(
                    "SELECT {title} FROM {view} WHERE ref = ?1
                     UNION ALL SELECT count(*) FROM {view}
                       WHERE substr(ref, 1, length(?2) + 1) <> ?2 || ':'",
                    title = m.title,
                    view = m.view
                );
                c.prepare(&sql)
                    .unwrap_or_else(|e| panic!("{}: {e}", m.view));
            }
            Ok(())
        })
        .await
        .unwrap();
        let registered: Vec<String> = oxplow_domain::refs::kind::core_kinds()
            .kinds()
            .map(|k| k.kind.clone())
            .collect();
        for m in REF_MODELS {
            assert!(registered.contains(&m.kind.to_string()), "{}", m.kind);
        }
    }
}
