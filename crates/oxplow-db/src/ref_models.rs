//! What core declares of each of its ref kinds (`.context/refs.md` "The
//! kind registry"), the way an extension's `ref_kinds:` declares its own:
//!
//! - **owner** — the one published model whose SQL spells the kind's
//!   prefix, in its `ref` column, and the column that names one of its
//!   rows. Everyone else selects or joins on that `ref`: a lens, a script,
//!   another model never rebuilds a ref from an id. The resolver reads a
//!   kind's title here, and the source guard over SQL exempts these owners.
//! - **page** — where a ref of the kind opens: a tab-id template
//!   (`{ref}`, or `{<column>}` of the owner's row for that ref, first
//!   non-empty of `{a|b}`). The vocabulary pass writes it to
//!   `v_ref_kind.page` beside an extension's, and the desktop opens every
//!   ref through it.
//! - **searchable** — the model whose rows (`ref`, `title`, `body`,
//!   `stream_id`, optional `version`) site search indexes under the kind.

use oxplow_domain::uri::types as k;

/// A core kind's owning model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefModel {
    /// The ref kind (`uri::types::EFFORT`).
    pub kind: &'static str,
    /// Its published view (`v_effort`), which has a `ref` column.
    pub view: &'static str,
    /// The column that names a row to a person.
    pub title: &'static str,
}

/// Where a core kind's ref opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// The ref is its own tab id.
    Itself,
    /// The first non-empty of these columns of its owning row (a ref).
    Column(&'static [&'static str]),
    /// One of the shell's pages (`page:<name>`).
    Route(&'static str),
}

impl Page {
    /// As `v_ref_kind.page` says it: `{ref}`, `{a|b}`, or a page's ref.
    pub fn template(&self) -> String {
        match self {
            Page::Itself => "{ref}".to_string(),
            Page::Column(columns) => format!("{{{}}}", columns.join("|")),
            Page::Route(name) => oxplow_domain::uri::build::page_uri(name, None),
        }
    }
}

/// What core declares of one of its kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreKind {
    pub kind: &'static str,
    /// Its owning model's view and title column; `None` for a kind no
    /// model holds (a file, a page).
    pub owner: Option<(&'static str, &'static str)>,
    /// Where one opens (a tab-id template); `None` for a kind with no page.
    pub page: Option<Page>,
    /// The view search indexes it from; `None` when it isn't searched.
    pub searchable: Option<&'static str>,
}

const fn kind(kind: &'static str) -> CoreKind {
    CoreKind {
        kind,
        owner: None,
        page: None,
        searchable: None,
    }
}

impl CoreKind {
    const fn owned(mut self, view: &'static str, title: &'static str) -> Self {
        self.owner = Some((view, title));
        self
    }
    const fn page(mut self, page: Page) -> Self {
        self.page = Some(page);
        self
    }
    /// It opens as itself: its ref is its tab id.
    const fn opens(self) -> Self {
        self.page(Page::Itself)
    }
    const fn searched(mut self, view: &'static str) -> Self {
        self.searchable = Some(view);
        self
    }
}

/// Every core kind core declares anything of.
pub const CORE_KINDS: &[CoreKind] = &[
    kind(k::STREAM).owned("v_stream", "title").opens(),
    kind(k::THREAD)
        .owned("v_thread", "title")
        .opens()
        .searched("v_search_thread"),
    kind(k::EFFORT)
        .owned("v_effort", "title")
        .opens()
        .searched("v_search_effort"),
    // A turn opens its session's transcript at it, else (no session) its diff.
    kind(k::TURN)
        .owned("v_agent_turn", "prompt")
        .page(Page::Column(&["transcript_page", "ref"]))
        .searched("v_search_turn"),
    kind(k::SNAPSHOT).owned("v_snapshot", "created_at").opens(),
    kind(k::COMMIT)
        .owned("v_commit", "subject")
        .opens()
        .searched("v_search_commit"),
    kind(k::SYMBOL).owned("v_symbol", "name").opens(),
    kind(k::WORK_ITEM)
        .owned("v_work_item", "title")
        .opens()
        .searched("v_search_work_item"),
    kind(k::WIKI)
        .owned("v_knowledge_page", "title")
        .opens()
        .searched("v_search_wiki"),
    // A comment shows among the others on the Comments page.
    kind(k::COMMENT)
        .owned("v_comment", "body")
        .page(Page::Route("comments"))
        .searched("v_search_comment"),
    kind(k::EVENT).owned("v_event", "type"),
    kind(k::ANSWER).owned("v_thread_answer", "title"),
    // A note lives in its thread's notes.
    kind(k::THREAD_NOTE)
        .owned("v_thread_note", "body")
        .page(Page::Column(&["thread_ref"]))
        .searched("v_search_thread_note"),
    // A handoff lives in its recipient thread's handoffs.
    kind(k::HANDOFF)
        .owned("v_handoff", "body")
        .page(Page::Column(&["to_thread_ref"]))
        .searched("v_search_handoff"),
    kind(k::AGENT_SESSION)
        .owned("v_agent_session", "display_name")
        .opens()
        .searched("v_search_agent_session"),
    kind(k::PROPOSAL).owned("v_command_proposal", "command"),
    kind(k::CLAIM).owned("v_claim", "statement"),
    // A decision is reviewed with its effort, else in its thread.
    kind(k::DECISION)
        .owned("v_decision", "question")
        .page(Page::Column(&["effort_ref", "thread_ref"]))
        .searched("v_search_decision"),
    kind(k::COLLECTOR).owned("v_collector_run", "id"),
    kind(k::JOB_RUN).owned("v_job_run", "intent_key"),
    kind(k::AI_CALL).owned("v_ai_call", "role"),
    kind(k::DASHBOARD)
        .owned("v_dashboard", "title")
        .opens()
        .searched("v_search_dashboard"),
    // A subagent opens its session for now.
    kind(k::SUBAGENT)
        .owned("v_subagent", "id")
        .page(Page::Column(&["session_ref"])),
    kind(k::LENS)
        .owned("v_lens", "title")
        .opens()
        .searched("v_search_lens"),
    kind(k::EXTENSION).owned("v_extension", "name"),
    kind(k::FILE).opens(),
    kind(k::DIR).opens(),
    kind(k::METRIC).opens(),
    kind(k::PAGE).opens(),
];

/// What core declares of `kind`, if anything.
pub fn core_kind(kind: &str) -> Option<&'static CoreKind> {
    CORE_KINDS.iter().find(|c| c.kind == kind)
}

/// Every core kind's owning model.
pub fn owners() -> impl Iterator<Item = RefModel> {
    CORE_KINDS.iter().filter_map(|c| {
        c.owner.map(|(view, title)| RefModel {
            kind: c.kind,
            view,
            title,
        })
    })
}

/// The owning model of `kind`, when a core model owns it.
pub fn owner_of(kind: &str) -> Option<RefModel> {
    owners().find(|m| m.kind == kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The columns a page template reads from its kind's owning row, in
    /// order: none for `{ref}` or a literal. `{a|b}` reads `a` then `b`.
    fn template_columns(template: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            for column in rest[open + 1..open + close].split('|') {
                if column != "ref" && !out.contains(&column) {
                    out.push(column);
                }
            }
            rest = &rest[open + close + 1..];
        }
        out
    }

    /// Each owner is a published view with a `ref` column of its kind and
    /// its title column: the resolver's query compiles on a fresh
    /// database, and every row's ref names the kind.
    #[tokio::test]
    async fn every_owner_has_a_ref_of_its_kind_and_a_title() {
        let db = crate::Database::in_memory();
        db.read(|c| {
            for m in owners() {
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
        let registered: Vec<String> = oxplow_domain::uri::types::core_types()
            .kinds()
            .map(|k| k.kind.clone())
            .collect();
        for c in CORE_KINDS {
            assert!(registered.contains(&c.kind.to_string()), "{}", c.kind);
        }
    }

    /// A page template's columns are its owner's, so the desktop's one
    /// query for them compiles; a template that reads a column needs an
    /// owner to read it from.
    #[tokio::test]
    async fn every_page_template_reads_its_owners_columns() {
        let db = crate::Database::in_memory();
        db.read(|c| {
            for kind in CORE_KINDS {
                let Some(page) = kind.page else { continue };
                let page = page.template();
                let columns = template_columns(&page);
                if columns.is_empty() {
                    continue;
                }
                let (view, _) = kind.owner.unwrap_or_else(|| {
                    panic!("{}: `{page}` reads a row, and no model owns it", kind.kind)
                });
                c.prepare(&format!(
                    "SELECT {} FROM {view} WHERE ref = ?1",
                    columns.join(", ")
                ))
                .unwrap_or_else(|e| panic!("{}: {e}", kind.kind));
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    /// A search feed has the columns the index reads.
    #[tokio::test]
    async fn every_search_feed_has_the_columns_search_reads() {
        let db = crate::Database::in_memory();
        db.read(|c| {
            for kind in CORE_KINDS {
                let Some(view) = kind.searchable else {
                    continue;
                };
                c.prepare(&format!("SELECT ref, title, body, stream_id FROM {view}"))
                    .unwrap_or_else(|e| panic!("{}: {e}", kind.kind));
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    #[test]
    fn a_templates_columns_are_read_in_order_and_ref_is_the_ref() {
        assert!(template_columns("{ref}").is_empty());
        assert!(template_columns("page:comments").is_empty());
        assert_eq!(template_columns("{thread_ref}"), ["thread_ref"]);
        assert_eq!(
            template_columns("{effort_ref|thread_ref}"),
            ["effort_ref", "thread_ref"]
        );
        assert_eq!(template_columns("page:acme/pr?ref={ref}&n={n}"), ["n"]);
    }
}
