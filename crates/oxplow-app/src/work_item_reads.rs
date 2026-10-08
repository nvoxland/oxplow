//! The agent's reads of the work-item interface (`.context/work-items.md`):
//! `v_work_item` and its views, whichever list is active — oxplow's tasks,
//! an extension's tracker, or none (nothing). What the MCP tools
//! `list_work_items`, `get_work_item` and `next_work_item` answer; no
//! read knows which list it is.

use std::collections::{HashMap, HashSet};

use oxplow_db::SqlCell;
use oxplow_domain::{DomainError, ThreadId};
use serde::Serialize;
use serde_json::Value;

use crate::sql_gateway::SqlGateway;

/// The longest body a list returns; `get_work_item` has the whole.
const LIST_BODY_CHARS: usize = 500;

/// One item, as `v_work_item` holds it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkItemRow {
    #[serde(rename = "ref")]
    pub item_ref: String,
    pub provider: String,
    pub title: String,
    pub body: String,
    /// `todo`, `in_progress`, `blocked`, `done` or `canceled`.
    pub state: String,
    /// The list's own state.
    pub native_state: String,
    pub parent_ref: Option<String>,
    /// The thread whose list it's on (`thr3`); `None` on the backlog.
    pub thread_id: Option<String>,
    pub rank: Option<f64>,
    pub closed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// The list's own fields (its declared fields).
    pub native: Value,
}

/// One item with its links and comments (`get_work_item`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkItemDetail {
    #[serde(flatten)]
    pub item: WorkItemRow,
    pub links: Vec<Link>,
    pub comments: Vec<Comment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Link {
    pub from_ref: String,
    pub to_ref: String,
    pub link_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comment {
    pub body: String,
    pub author: String,
    pub created_at: String,
}

/// What to work on next on a thread's list.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NextWork {
    /// Nothing ready.
    Empty,
    /// The first ready item is an epic: it and its ready descendants, as
    /// one unit.
    Epic {
        epic: Box<WorkItemRow>,
        children: Vec<WorkItemRow>,
    },
    /// Every ready item that isn't an epic, to pick one or a cluster.
    Standalone { items: Vec<WorkItemRow> },
}

/// Which list `list_work_items` reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Thread(ThreadId),
    Backlog,
}

const COLUMNS: &str = "ref, provider, title, body, state, native_state, parent_ref, thread_id, \
                       rank, closed_at, created_at, updated_at, native";
const ORDER: &str = "ORDER BY rank IS NULL, rank, created_at";

fn text(cell: &SqlCell) -> Option<String> {
    match cell {
        SqlCell::Null(()) => None,
        SqlCell::Text(s) => Some(s.clone()),
        SqlCell::Int(i) => Some(i.to_string()),
        SqlCell::Real(r) => Some(r.to_string()),
        SqlCell::Bool(b) => Some(b.to_string()),
    }
}

fn row_of(cells: &[SqlCell]) -> WorkItemRow {
    let s = |i: usize| text(&cells[i]).unwrap_or_default();
    let o = |i: usize| text(&cells[i]);
    WorkItemRow {
        item_ref: s(0),
        provider: s(1),
        title: s(2),
        body: s(3),
        state: s(4),
        native_state: s(5),
        parent_ref: o(6),
        thread_id: match &cells[7] {
            SqlCell::Int(id) => Some(ThreadId::new(*id).to_string()),
            _ => None,
        },
        rank: match &cells[8] {
            SqlCell::Int(r) => Some(*r as f64),
            SqlCell::Real(r) => Some(*r),
            _ => None,
        },
        closed_at: o(9),
        created_at: s(10),
        updated_at: s(11),
        native: o(12)
            .and_then(|n| serde_json::from_str(&n).ok())
            .unwrap_or(Value::Null),
    }
}

async fn items(
    sql: &SqlGateway,
    filter: &str,
    params: Vec<SqlCell>,
) -> Result<Vec<WorkItemRow>, DomainError> {
    let out = sql
        .query_sql(
            &format!("SELECT {COLUMNS} FROM v_work_item WHERE {filter} {ORDER}"),
            params,
            None,
        )
        .await?;
    Ok(out.rows.iter().map(|r| row_of(r)).collect())
}

fn thread_cell(thread: &ThreadId) -> SqlCell {
    SqlCell::Int(thread.value())
}

/// A list's items in list order, in `states` (every state when empty),
/// each body cut to [`LIST_BODY_CHARS`].
pub async fn list(
    sql: &SqlGateway,
    scope: &Scope,
    states: &[String],
) -> Result<Vec<WorkItemRow>, DomainError> {
    let (mut filter, mut params) = match scope {
        Scope::Thread(t) => ("thread_id = ?1".to_string(), vec![thread_cell(t)]),
        Scope::Backlog => ("thread_id IS NULL".to_string(), Vec::new()),
    };
    if !states.is_empty() {
        let at = params.len();
        filter.push_str(&format!(
            " AND state IN ({})",
            (1..=states.len())
                .map(|i| format!("?{}", at + i))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        params.extend(states.iter().cloned().map(SqlCell::Text));
    }
    let mut rows = items(sql, &filter, params).await?;
    for row in &mut rows {
        if row.body.chars().count() > LIST_BODY_CHARS {
            row.body = row.body.chars().take(LIST_BODY_CHARS).collect::<String>() + "…";
        }
    }
    Ok(rows)
}

/// One item with its links (both ways) and comments; `None` when it isn't
/// on the active list.
pub async fn get(sql: &SqlGateway, item_ref: &str) -> Result<Option<WorkItemDetail>, DomainError> {
    let found = items(sql, "ref = ?1", vec![SqlCell::Text(item_ref.into())]).await?;
    let Some(item) = found.into_iter().next() else {
        return Ok(None);
    };
    let links = sql
        .query_sql(
            "SELECT from_ref, to_ref, link_type FROM v_work_item_link
              WHERE from_ref = ?1 OR to_ref = ?1 ORDER BY created_at",
            vec![SqlCell::Text(item_ref.into())],
            None,
        )
        .await?
        .rows
        .iter()
        .map(|r| Link {
            from_ref: text(&r[0]).unwrap_or_default(),
            to_ref: text(&r[1]).unwrap_or_default(),
            link_type: text(&r[2]).unwrap_or_default(),
        })
        .collect();
    let comments = sql
        .query_sql(
            "SELECT body, author, created_at FROM v_work_item_comment
              WHERE ref = ?1 ORDER BY created_at",
            vec![SqlCell::Text(item_ref.into())],
            None,
        )
        .await?
        .rows
        .iter()
        .map(|r| Comment {
            body: text(&r[0]).unwrap_or_default(),
            author: text(&r[1]).unwrap_or_default(),
            created_at: text(&r[2]).unwrap_or_default(),
        })
        .collect();
    Ok(Some(WorkItemDetail {
        item,
        links,
        comments,
    }))
}

/// What to work on next on `thread`'s list ([`next_from`]).
pub async fn next(sql: &SqlGateway, thread: &ThreadId) -> Result<NextWork, DomainError> {
    let all = items(sql, "thread_id = ?1", vec![thread_cell(thread)]).await?;
    let blocks = sql
        .query_sql(
            "SELECT from_ref, to_ref FROM v_work_item_link WHERE link_type = 'blocks'",
            Vec::new(),
            None,
        )
        .await?
        .rows
        .iter()
        .map(|r| {
            (
                text(&r[0]).unwrap_or_default(),
                text(&r[1]).unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let closed = sql
        .query_sql(
            "SELECT ref FROM v_work_item WHERE state IN ('done', 'canceled')",
            Vec::new(),
            None,
        )
        .await?
        .rows
        .iter()
        .filter_map(|r| text(&r[0]))
        .collect::<HashSet<_>>();
    Ok(next_from(all, &blocks, &closed))
}

/// The choice itself, over a list in list order: the ready (`todo`) items
/// no open blocker holds; if the first is an epic (an item with
/// children), it and its ready, unblocked descendants; else every ready
/// item that isn't an epic. `blocks` are `(blocker, blocked)` refs;
/// `closed` the refs that no longer block.
pub fn next_from(
    all: Vec<WorkItemRow>,
    blocks: &[(String, String)],
    closed: &HashSet<String>,
) -> NextWork {
    let mut blockers: HashMap<&str, Vec<&str>> = HashMap::new();
    for (from, to) in blocks {
        blockers.entry(to.as_str()).or_default().push(from.as_str());
    }
    let blocked = |r: &str| {
        blockers
            .get(r)
            .is_some_and(|bs| bs.iter().any(|b| !closed.contains(*b)))
    };
    let parents: HashSet<&str> = all.iter().filter_map(|i| i.parent_ref.as_deref()).collect();
    let is_epic = |i: &WorkItemRow| parents.contains(i.item_ref.as_str());
    let ready: Vec<&WorkItemRow> = all
        .iter()
        .filter(|i| i.state == "todo" && !blocked(&i.item_ref))
        .collect();
    let Some(head) = ready.first() else {
        return NextWork::Empty;
    };
    if is_epic(head) {
        let mut children = Vec::new();
        let mut frontier = vec![head.item_ref.as_str()];
        while let Some(parent) = frontier.pop() {
            for it in all
                .iter()
                .filter(|i| i.parent_ref.as_deref() == Some(parent))
            {
                if it.state == "todo" && !blocked(&it.item_ref) {
                    children.push(it.clone());
                }
                frontier.push(it.item_ref.as_str());
            }
        }
        // In list order, as `all` is.
        let order: HashMap<&str, usize> = all
            .iter()
            .enumerate()
            .map(|(n, i)| (i.item_ref.as_str(), n))
            .collect();
        children.sort_by_key(|c| order.get(c.item_ref.as_str()).copied());
        return NextWork::Epic {
            epic: Box::new((*head).clone()),
            children,
        };
    }
    NextWork::Standalone {
        items: ready.into_iter().filter(|i| !is_epic(i)).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(r: &str, state: &str, parent: Option<&str>) -> WorkItemRow {
        WorkItemRow {
            item_ref: r.into(),
            provider: "issues".into(),
            title: r.into(),
            body: String::new(),
            state: state.into(),
            native_state: state.into(),
            parent_ref: parent.map(str::to_string),
            thread_id: Some("thr1".into()),
            rank: None,
            closed_at: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            native: Value::Null,
        }
    }

    fn refs(items: &[WorkItemRow]) -> Vec<&str> {
        items.iter().map(|i| i.item_ref.as_str()).collect()
    }

    #[test]
    fn nothing_ready_is_empty() {
        let all = vec![item("a", "in_progress", None), item("b", "done", None)];
        assert_eq!(next_from(all, &[], &HashSet::new()), NextWork::Empty);
    }

    #[test]
    fn ready_items_that_are_not_epics_are_offered_in_list_order() {
        let all = vec![
            item("a", "todo", None),
            item("b", "blocked", None),
            item("c", "todo", None),
        ];
        let NextWork::Standalone { items } = next_from(all, &[], &HashSet::new()) else {
            panic!("standalone");
        };
        assert_eq!(refs(&items), ["a", "c"]);
    }

    /// A ready epic first: it and its ready descendants, as one unit.
    #[test]
    fn a_ready_epic_first_comes_with_its_ready_descendants() {
        let all = vec![
            item("e", "todo", None),
            item("c1", "todo", Some("e")),
            item("c2", "done", Some("e")),
            item("g", "todo", Some("c1")),
            item("x", "todo", None),
        ];
        let NextWork::Epic { epic, children } = next_from(all, &[], &HashSet::new()) else {
            panic!("epic");
        };
        assert_eq!(epic.item_ref, "e");
        assert_eq!(refs(&children), ["c1", "g"]);
    }

    /// An item an open blocker holds isn't ready; a closed blocker doesn't
    /// hold it.
    #[test]
    fn blocks_hold_an_item_until_the_blocker_closes() {
        let all = vec![item("a", "todo", None), item("b", "todo", None)];
        let blocks = vec![("z".to_string(), "a".to_string())];
        let NextWork::Standalone { items } = next_from(all.clone(), &blocks, &HashSet::new())
        else {
            panic!("standalone");
        };
        assert_eq!(refs(&items), ["b"]);
        let closed = HashSet::from(["z".to_string()]);
        let NextWork::Standalone { items } = next_from(all, &blocks, &closed) else {
            panic!("standalone");
        };
        assert_eq!(refs(&items), ["a", "b"]);
    }

    /// The reads go through the interface: oxplow's tasks while they're the
    /// list, nothing with none.
    #[tokio::test]
    async fn the_reads_follow_the_active_list() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let run = |input: Value| {
            let svc = fx.svc.clone();
            async move {
                svc.commands
                    .run(
                        &oxplow_domain::Actor::Human,
                        "oxplow.work_item.create",
                        input,
                        false,
                    )
                    .await
                    .unwrap()
            }
        };
        let thread = fx.thread.to_string();
        let filed = run(
            serde_json::json!({ "title": "Next up", "thread": thread, "body": "x".repeat(600) }),
        )
        .await;
        let item_ref = filed.result["ref"].as_str().unwrap().to_string();
        run(serde_json::json!({ "title": "Later" })).await;

        let sql = &fx.svc.sql;
        let listed = list(sql, &Scope::Thread(fx.thread), &[]).await.unwrap();
        assert_eq!(refs(&listed), [item_ref.as_str()]);
        assert_eq!(
            listed[0].body.chars().count(),
            LIST_BODY_CHARS + 1,
            "cut, with an ellipsis"
        );
        assert_eq!(listed[0].thread_id.as_deref(), Some(thread.as_str()));
        let backlog = list(sql, &Scope::Backlog, &["todo".into()]).await.unwrap();
        assert_eq!(
            backlog.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(),
            ["Later"]
        );
        let detail = get(sql, &item_ref).await.unwrap().expect("on the list");
        assert_eq!(detail.item.body.len(), 600);
        assert!(
            matches!(next(sql, &fx.thread).await.unwrap(), NextWork::Standalone { items } if items.len() == 1)
        );

        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "none".into());
        let config = crate::config_service::read_config(&fx.svc.config);
        fx.svc
            .capabilities
            .publish_now(&config, &fx.svc.db)
            .unwrap();
        assert!(list(sql, &Scope::Thread(fx.thread), &[])
            .await
            .unwrap()
            .is_empty());
        assert!(get(sql, &item_ref).await.unwrap().is_none());
        assert_eq!(next(sql, &fx.thread).await.unwrap(), NextWork::Empty);
    }
}
