//! What the person's windows show (`.context/pages-and-tabs.md` "What
//! agents see of the tabs"), read from `ClientReports`:
//!
//! - `pages.read/open { thread_id, tab? }`: the page a thread has open —
//!   in the window the person was in last, with the other windows' beside
//!   it — or the named tab; for a lens, that lens re-run with the person's
//!   params, as text.
//! - `pages.read/list_tabs { scope?, thread? }`: the tabs of a thread, its
//!   stream's threads or every thread, in every window, each with whether
//!   it is selected and on screen and how fresh its report is.

use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::uri::build as refs;
use oxplow_domain::{Actor, CommandError, StreamId, ThreadId, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use crate::Services;

/// The commands its operations are declared as.
pub const PAGE_OPEN: &str = "oxplow.page.open";
pub const PAGE_LIST_TABS: &str = "oxplow.page.list_tabs";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenPageInput {
    /// The thread (`thr…`).
    pub thread_id: String,
    /// One of its tabs (an id `oxplow_list_tabs` listed) instead of the
    /// page it has open.
    #[serde(default)]
    pub tab: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TabScope {
    /// One thread's tabs.
    #[default]
    Thread,
    /// Every thread's in that thread's stream.
    Stream,
    /// Every thread's.
    Project,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTabsInput {
    /// Whose tabs: `thread` (default), `stream` or `project`.
    #[serde(default)]
    pub scope: TabScope,
    /// The thread (`thr3`); default your own.
    #[serde(default)]
    pub thread: Option<String>,
}

/// `value` as a thread: its bare id.
fn thread_of(field: &str, value: &str) -> Result<ThreadId, CommandError> {
    ThreadId::try_from_str(value).ok_or_else(|| {
        invalid(
            field,
            format!("expected a thread id (`thr3`), got `{value}`"),
        )
    })
}

/// Every thread with its stream.
async fn threads(svc: &Services) -> Result<Vec<(ThreadId, StreamId)>, CommandError> {
    let out = svc
        .sql
        .query_sql("SELECT id, stream_id FROM v_thread", vec![], Some(100_000))
        .await
        .map_err(failed)?;
    Ok(out
        .rows
        .into_iter()
        .filter_map(|r| match (&r[0], &r[1]) {
            (SqlCell::Int(t), SqlCell::Int(s)) => Some((ThreadId::new(*t), StreamId::new(*s))),
            _ => None,
        })
        .collect())
}

fn lens_params(detail: &Value) -> std::collections::BTreeMap<String, SqlCell> {
    detail
        .get("params")
        .and_then(|p| p.as_object())
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default()
}

/// A lens page re-run with `detail`'s params, as text; null for any other page.
async fn lens_text(
    svc: &Services,
    thread: ThreadId,
    page_id: &str,
    detail: &Value,
) -> Result<Value, CommandError> {
    let Ok(lens_id) = refs::id_of(page_id, oxplow_domain::uri::resources::LENS) else {
        return Ok(Value::Null);
    };
    // The app shows the main worktree's lenses, whatever the thread's stream.
    let root = svc.worktrees.project_dir();
    let ctx = crate::extensions::lens_context(svc, None, Some(thread)).await;
    let read = async {
        let run = crate::extensions::run_lens(
            &svc.sql,
            &svc.extension_catalog,
            root,
            &lens_id,
            lens_params(detail),
            &ctx,
        )
        .await?;
        let text = crate::lens_text::text_run(svc, root, &run, &ctx).await?;
        Ok::<_, oxplow_domain::DomainError>((run.lens, text))
    };
    Ok(match read.await {
        Ok((lens, text)) => {
            let mut out = serde_json::to_value(text).map_err(failed)?;
            // Where the page and each of its parts come from: the files an
            // agent edits to change what the person sees.
            out["origin"] = lens.origin.to_json(root);
            out["parts"] = lens
                .parts
                .iter()
                .map(|p| {
                    let part = p
                        .lens
                        .as_deref()
                        .and_then(|id| svc.extension_catalog.find_lens(root, id).ok());
                    json!({
                        "id": p.id,
                        "lens": p.lens,
                        "title": p.title.clone().or_else(|| part.as_ref().map(|l| l.title.clone())),
                        "source": part.and_then(|l| l.origin.source),
                    })
                })
                .collect();
            out
        }
        Err(e) => json!({ "error": e.to_string() }),
    })
}

async fn open(svc: &Services, input: OpenPageInput) -> Result<Value, CommandError> {
    let thread = ThreadId::try_from_str(&input.thread_id).ok_or_else(|| {
        invalid(
            "/thread_id",
            format!("expected a thread id (thr…), got `{}`", input.thread_id),
        )
    })?;
    let now = Timestamp::now();
    let pages = svc.client_reports.open_pages(thread, now);
    let (page_id, kind, detail) = match &input.tab {
        Some(tab) => {
            let listed = svc.client_reports.tabs(&[thread], now);
            let Some(found) = listed.iter().find(|t| &t.tab == tab) else {
                return Err(invalid(
                    "/tab",
                    format!(
                        "no window shows `{tab}` in this thread; `oxplow_list_tabs` lists them"
                    ),
                ));
            };
            let detail = pages
                .iter()
                .find(|p| &p.page_id == tab)
                .and_then(|p| p.detail_json.clone());
            (found.tab.clone(), found.kind.clone(), detail)
        }
        None => match pages.first() {
            Some(p) => (p.page_id.clone(), p.kind.clone(), p.detail_json.clone()),
            None => return Ok(json!({ "open": null, "others": [] })),
        },
    };
    let detail: Value = detail
        .as_deref()
        .and_then(|d| serde_json::from_str(d).ok())
        .unwrap_or(Value::Null);
    let lens = if kind == "lens" {
        lens_text(svc, thread, &page_id, &detail).await?
    } else {
        Value::Null
    };
    let this = pages.iter().find(|p| p.page_id == page_id);
    // A page or lens apart from what it's opened with; anything else (a
    // file, a commit) is its URI as it is.
    let (page, view) = oxplow_domain::uri::build::params_of(&page_id)
        .unwrap_or_else(|| (page_id.clone(), Default::default()));
    let params: serde_json::Map<String, Value> = view
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    let others: Vec<&crate::client_reports::OpenPage> = pages
        .iter()
        .filter(|p| this.is_none_or(|t| t.client != p.client))
        .collect();
    Ok(json!({
        "open": {
            "pageId": page_id,
            // What it is and how it's viewed, apart: the id is only their
            // string form.
            "page": page,
            "params": params,
            "kind": kind,
            "detail": detail,
            "back": this.map(|p| p.back.clone()).unwrap_or_default(),
            "forward": this.map(|p| p.forward.clone()).unwrap_or_default(),
            "origin": this.and_then(|p| p.origin.clone()),
            "client": this.map(|p| p.client.clone()),
            "visible": this.is_some_and(|p| p.visible),
            "reportedAt": this.map(|p| p.reported_at),
            "stale": this.is_some_and(|p| p.stale),
        },
        "others": serde_json::to_value(others).map_err(failed)?,
        "lens": lens,
    }))
}

async fn list_tabs(
    svc: &Services,
    actor: &Actor,
    input: ListTabsInput,
) -> Result<Value, CommandError> {
    let thread = match &input.thread {
        Some(t) => Some(thread_of("/thread", t)?),
        None => match actor {
            Actor::Agent { thread_id, .. } => *thread_id,
            _ => None,
        },
    };
    let all = threads(svc).await?;
    let stream_of = |t: ThreadId| all.iter().find(|(id, _)| *id == t).map(|(_, s)| *s);
    let wanted: Vec<ThreadId> = match (input.scope, thread) {
        (TabScope::Project, _) => all.iter().map(|(t, _)| *t).collect(),
        (TabScope::Thread, Some(t)) => vec![t],
        (TabScope::Stream, Some(t)) => {
            let stream =
                stream_of(t).ok_or_else(|| invalid("/thread", format!("no thread `{t}`")))?;
            all.iter()
                .filter(|(_, s)| *s == stream)
                .map(|(t, _)| *t)
                .collect()
        }
        (_, None) => {
            return Err(invalid(
                "/thread",
                "name the thread (`oxplow:oxplow/thread?id=thr3`): you aren't on one",
            ))
        }
    };
    let now = Timestamp::now();
    let reported = svc.client_reports.reported_threads(now);
    let tabs: Vec<Value> = svc
        .client_reports
        .tabs(&wanted, now)
        .into_iter()
        .map(|t| {
            json!({
                "tab": t.tab,
                "kind": t.kind,
                "title": t.title,
                "thread": t.thread.to_string(),
                "stream": stream_of(t.thread).map(|s| s.to_string()),
                // A session's tab is its session: what the tab names, not
                // what the window called its kind.
                "session": (oxplow_domain::uri::build::resource_of(&t.tab).as_deref()
                    == Some(oxplow_domain::uri::resources::AGENT_SESSION))
                .then(|| oxplow_domain::uri::build::id_of(&t.tab, oxplow_domain::uri::resources::AGENT_SESSION).ok()).flatten(),
                "selected": t.selected,
                "visible": t.visible,
                "client": t.client,
                "reportedAt": t.reported_at,
                "stale": t.stale,
            })
        })
        .collect();
    let unreported: Vec<String> = wanted
        .iter()
        .filter(|t| !reported.contains(t))
        .map(|t| refs::thread_uri(*t))
        .collect();
    Ok(json!({ "tabs": tabs, "unreported": unreported }))
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let for_list = services.clone();
    vec![
        Op::new(
            "pages.read",
            "open",
            schema::<OpenPageInput>(),
            false,
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: OpenPageInput = parse(input)?;
                    let svc = services.get()?;
                    Ok(HandlerOutput {
                        result: open(&svc, input).await?,
                        ..HandlerOutput::default()
                    })
                })
            })),
        ),
        Op::new(
            "pages.read",
            "list_tabs",
            schema::<ListTabsInput>(),
            false,
            Handler::External(Arc::new(move |inv: Invocation, input| {
                let services = for_list.clone();
                Box::pin(async move {
                    let input: ListTabsInput = parse(input)?;
                    let svc = services.get()?;
                    let result = list_tabs(&svc, inv.actor(), input).await?;
                    Ok(HandlerOutput {
                        result,
                        ..HandlerOutput::default()
                    })
                })
            })),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_reports::{ReportedTab, TabReport, ThreadTabs};
    use crate::test_fixtures::services_with_effort;

    fn tab(id: &str, kind: &str) -> ReportedTab {
        ReportedTab {
            id: id.into(),
            kind: kind.into(),
            title: format!("{id} title"),
            back: Vec::new(),
            forward: Vec::new(),
            origin: None,
        }
    }

    /// The open page reads as what it is and how it's viewed: its identity
    /// and its params apart, and where its tab's history leads — so an
    /// agent never parses a tab id.
    #[tokio::test]
    async fn the_open_page_says_its_page_its_params_and_its_history() {
        let fx = services_with_effort().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let id = "oxplow:oxplow/lens?lens=acme/commits&q=numbat%20bay&range=7d";
        report(
            &fx.svc,
            "a",
            true,
            fx.thread,
            vec![ReportedTab {
                back: vec!["oxplow:oxplow/wiki?slug=home".into()],
                forward: vec![
                    "oxplow:oxplow/lens?commit=abc&lens=acme/commits&q=numbat%20bay&range=7d"
                        .into(),
                ],
                origin: Some("Recent commits".into()),
                ..tab(id, "lens")
            }],
            id,
        );
        let open = fx
            .svc
            .commands
            .run(
                &actor,
                PAGE_OPEN,
                json!({ "thread_id": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap()
            .result;
        assert_eq!(open["open"]["pageId"], id);
        assert_eq!(open["open"]["page"], "oxplow:oxplow/lens?lens=acme/commits");
        assert_eq!(
            open["open"]["params"],
            json!({ "range": "7d", "q": "numbat bay" })
        );
        assert_eq!(
            open["open"]["back"],
            json!(["oxplow:oxplow/wiki?slug=home"])
        );
        assert_eq!(
            open["open"]["forward"][0],
            "oxplow:oxplow/lens?commit=abc&lens=acme/commits&q=numbat%20bay&range=7d"
        );
        assert_eq!(open["open"]["origin"], "Recent commits");
    }

    fn report(
        svc: &Services,
        client: &str,
        focused: bool,
        thread: ThreadId,
        tabs: Vec<ReportedTab>,
        active: &str,
    ) {
        svc.client_reports.report(
            TabReport {
                client: client.into(),
                focused,
                visible: true,
                shown_thread: Some(thread),
                threads: vec![ThreadTabs {
                    thread,
                    tabs,
                    active: Some(active.into()),
                }],
                detail_json: Some(r#"{"line":3}"#.into()),
            },
            Timestamp::now(),
        );
    }

    /// What a window reported open in a thread reads back through the bus
    /// (null while nothing has been reported), naming its window, with the
    /// other windows' pages beside it; a listed tab reads by its id; a
    /// non-thread id is refused at its field.
    #[tokio::test]
    async fn the_open_page_reads_through_the_bus() {
        let fx = services_with_effort().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let tid = fx.thread.to_string();
        let run = |input: Value| {
            let svc = fx.svc.clone();
            let actor = actor.clone();
            async move { svc.commands.run(&actor, PAGE_OPEN, input, false).await }
        };
        let none = run(json!({ "thread_id": tid })).await.unwrap();
        assert_eq!(none.result["open"], Value::Null);
        assert!(none.audit_id.is_none(), "a read leaves no record");

        report(
            &fx.svc,
            "b",
            false,
            fx.thread,
            vec![tab("oxplow:oxplow/wiki?slug=home", "wiki")],
            "oxplow:oxplow/wiki?slug=home",
        );
        report(
            &fx.svc,
            "a",
            true,
            fx.thread,
            vec![
                tab("oxplow:oxplow/file?path=src/a.rs", "file"),
                tab("oxplow:oxplow/page?page=settings", "settings"),
            ],
            "oxplow:oxplow/file?path=src/a.rs",
        );
        let open = run(json!({ "thread_id": tid })).await.unwrap().result;
        assert_eq!(open["open"]["pageId"], "oxplow:oxplow/file?path=src/a.rs");
        assert_eq!(open["open"]["client"], "a");
        assert_eq!(open["open"]["detail"]["line"], 3);
        assert_eq!(open["others"][0]["pageId"], "oxplow:oxplow/wiki?slug=home");
        assert_eq!(open["lens"], Value::Null);

        let by_tab = run(json!({ "thread_id": tid, "tab": "oxplow:oxplow/page?page=settings" }))
            .await
            .unwrap()
            .result;
        assert_eq!(by_tab["open"]["pageId"], "oxplow:oxplow/page?page=settings");
        assert_eq!(
            by_tab["open"]["detail"],
            Value::Null,
            "only the shown page publishes detail"
        );
        let err = run(json!({ "thread_id": tid, "tab": "oxplow:oxplow/wiki?slug=gone" }))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/tab"),
            "{err:?}"
        );

        let err = run(json!({ "thread_id": "str1" })).await.unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/thread_id"),
            "{err:?}"
        );
    }

    /// The tabs of the caller's thread by default, of its stream or the
    /// project on request; a thread no window reported is named as such.
    #[tokio::test]
    async fn tabs_are_listed_by_scope() {
        let fx = services_with_effort().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let run = |input: Value| {
            let svc = fx.svc.clone();
            let actor = actor.clone();
            async move {
                svc.commands
                    .run(&actor, PAGE_LIST_TABS, input, false)
                    .await
                    .unwrap()
                    .result
            }
        };
        let empty = run(json!({})).await;
        assert_eq!(empty["tabs"], json!([]));
        assert_eq!(empty["unreported"], json!([refs::thread_uri(fx.thread)]));

        report(
            &fx.svc,
            "a",
            true,
            fx.thread,
            vec![
                tab("oxplow:oxplow/wiki?slug=home", "wiki"),
                tab("oxplow:oxplow/agent_session?id=ses1", "agent_session"),
            ],
            "oxplow:oxplow/wiki?slug=home",
        );
        let listed = run(json!({ "scope": "stream" })).await;
        let rows = listed["tabs"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["tab"], "oxplow:oxplow/wiki?slug=home");
        assert_eq!(rows[0]["title"], "oxplow:oxplow/wiki?slug=home title");
        assert_eq!(rows[0]["thread"], fx.thread.to_string());
        assert_eq!(
            (rows[0]["selected"].clone(), rows[0]["visible"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(rows[1]["session"], "ses1");
        assert_eq!(rows[1]["selected"], false);
        assert_eq!(listed["unreported"], json!([]));
        assert_eq!(
            run(json!({ "scope": "project" })).await["tabs"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let named = run(json!({ "thread": fx.thread.to_string() })).await;
        assert_eq!(named["tabs"].as_array().unwrap().len(), 2);
    }
}
