//! What the person's devices have open (`.context/pages-and-tabs.md` "What
//! agents see of the tabs"): each device's stored tabs (`tab_session`),
//! joined with what its window reports live (`ClientReports`):
//!
//! - `pages.read/open { thread_id, tab? }`: the page a thread has open —
//!   on the device the person used last, with the other devices' beside
//!   it — or the named tab; for a page, the window's own open run of it
//!   (`pageRun`), as text, with the node they are on.
//! - `pages.read/list_tabs { scope?, thread? }`: the tabs of a thread, its
//!   stream's threads or every thread, on every device with some (connected
//!   or not), each with whether it is selected and on screen.

use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::uri::build as refs;
use oxplow_domain::{Actor, CommandError, DeviceId, StreamId, ThreadId, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use crate::client_reports::stored_tabs::{self, StoredTab, StoredThread};
use crate::client_reports::LiveWindow;
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

/// One device's stored tabs, with what is known of the device now.
struct DeviceTabs {
    device: DeviceId,
    /// Its name, while it is connected.
    name: Option<String>,
    /// Its page load, while it is connected.
    connection: Option<String>,
    /// What its window shows, when it has reported.
    live: Option<LiveWindow>,
    threads: Vec<StoredThread>,
}

impl DeviceTabs {
    /// `tab` is on screen: selected in `thread`, which the window shows.
    fn visible(&self, thread: ThreadId, tab: &StoredTab) -> bool {
        let selected = self
            .threads
            .iter()
            .find(|t| t.thread == thread)
            .and_then(StoredThread::selected);
        selected.is_some_and(|s| s.slot == tab.slot)
            && self.live.as_ref().is_some_and(|l| l.shows(thread))
    }

    /// The facts of the device an agent is told beside a tab.
    fn facts(&self) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        out.insert("device".into(), json!(self.device.as_str()));
        out.insert("deviceName".into(), json!(self.name));
        out.insert("connected".into(), json!(self.connection.is_some()));
        if let Some(c) = &self.connection {
            out.insert("client".into(), json!(c));
        }
        out
    }

    fn report_facts(&self, out: &mut serde_json::Map<String, Value>) {
        if let Some(l) = &self.live {
            out.insert("reportedAt".into(), json!(l.reported_at));
            out.insert("stale".into(), json!(l.stale));
        }
    }
}

/// Every device with tabs stored for `wanted` (all threads when `None`):
/// the one the person used last first, then the other connected devices by
/// their windows' order, then the rest by id. A row that doesn't parse is
/// skipped.
async fn devices_with_tabs(
    svc: &Services,
    wanted: Option<&[ThreadId]>,
    now: Timestamp,
) -> Result<Vec<DeviceTabs>, CommandError> {
    let rows = svc.tab_session_store.list_all().await.map_err(failed)?;
    let mut by_device: Vec<DeviceTabs> = Vec::new();
    let windows = svc.client_reports.windows(now);
    for (device, row) in rows {
        if wanted.is_some_and(|w| !w.contains(&row.thread_id)) {
            continue;
        }
        let stored = match stored_tabs::parse(device.clone(), row.thread_id, &row.session_json) {
            Ok(s) => s,
            Err(error) => {
                tracing::warn!(%error, device = %device.as_str(), thread = %row.thread_id,
                    "skipping stored tabs that don't parse");
                continue;
            }
        };
        match by_device.iter_mut().find(|d| d.device == device) {
            Some(d) => d.threads.push(stored),
            None => by_device.push(DeviceTabs {
                name: svc.client_devices.get(&device).map(|d| d.name),
                connection: svc
                    .client_devices
                    .connection_of(&device)
                    .map(|c| c.as_str().to_string()),
                live: windows
                    .iter()
                    .find(|w| w.device == device.as_str())
                    .cloned(),
                device,
                threads: vec![stored],
            }),
        }
    }
    let used = svc.client_devices.last_used().map(|(d, _)| d);
    let rank = |d: &DeviceTabs| {
        if Some(&d.device) == used.as_ref() {
            return (0, 0);
        }
        match windows.iter().position(|w| w.device == d.device.as_str()) {
            Some(i) => (1, i),
            None => (2, 0),
        }
    };
    by_device.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.device.as_str().cmp(b.device.as_str()))
    });
    Ok(by_device)
}

/// The tab selected in `thread` on each device that has one, in device order.
fn selected_in(devices: &[DeviceTabs], thread: ThreadId) -> Vec<(&DeviceTabs, &StoredTab)> {
    devices
        .iter()
        .filter_map(|d| {
            let t = d.threads.iter().find(|t| t.thread == thread)?;
            Some((d, t.selected()?))
        })
        .collect()
}

/// A page open on a device, as an agent reads it.
fn open_json(thread: ThreadId, device: &DeviceTabs, tab: &StoredTab) -> Value {
    let mut out = device.facts();
    out.insert("pageId".into(), json!(tab.id));
    out.insert("kind".into(), json!(tab.kind));
    out.insert("back".into(), json!(tab.back));
    out.insert("forward".into(), json!(tab.forward));
    out.insert("origin".into(), json!(tab.origin));
    out.insert("visible".into(), json!(device.visible(thread, tab)));
    out.insert("reportedAt".into(), Value::Null);
    out.insert("stale".into(), Value::Null);
    device.report_facts(&mut out);
    Value::Object(out)
}

async fn open(svc: &Services, input: OpenPageInput) -> Result<Value, CommandError> {
    let thread = ThreadId::try_from_str(&input.thread_id).ok_or_else(|| {
        invalid(
            "/thread_id",
            format!("expected a thread id (thr…), got `{}`", input.thread_id),
        )
    })?;
    let now = Timestamp::now();
    let devices = devices_with_tabs(svc, None, now).await?;
    let selected = selected_in(&devices, thread);
    // The page asked for, else the one open on the device used last.
    let this: Option<(&DeviceTabs, &StoredTab)> = match &input.tab {
        Some(tab) => {
            let found = devices.iter().find_map(|d| {
                let t = d.threads.iter().find(|t| t.thread == thread)?;
                Some((d, t.tabs.iter().find(|x| &x.id == tab)?))
            });
            if found.is_none() {
                return Err(invalid(
                    "/tab",
                    format!(
                        "no device has `{tab}` open in this thread; `oxplow_list_tabs` lists them"
                    ),
                ));
            }
            found
        }
        None => selected.first().copied(),
    };
    let Some((device, tab)) = this else {
        // Say why nothing is open, so a null isn't read as "the person has
        // nothing open": no device has tabs stored for this thread, or none
        // has one selected in it — and where the person is.
        let stored = devices
            .iter()
            .any(|d| d.threads.iter().any(|t| t.thread == thread));
        let shown: Vec<Value> = svc
            .client_reports
            .windows(now)
            .into_iter()
            .filter_map(|w| {
                let t = w.shown_thread?;
                let selected = devices
                    .iter()
                    .filter(|d| d.device.as_str() == w.device)
                    .find_map(|d| d.threads.iter().find(|x| x.thread == t))
                    .and_then(StoredThread::selected)
                    .map(|s| s.id.clone());
                Some(json!({ "thread": refs::thread_uri(t), "selected": selected }))
            })
            .collect();
        let reason = if !stored {
            "no device has tabs stored for this thread (none is open on it, or none has told oxplow since it started); `shown` lists the threads the person's windows are looking at"
        } else {
            "a device has this thread's tabs stored but none is selected in it"
        };
        return Ok(json!({ "open": null, "others": [], "reason": reason, "shown": shown }));
    };
    // The window publishes detail for the page it shows.
    let is_shown = device
        .threads
        .iter()
        .find(|t| t.thread == thread)
        .and_then(StoredThread::selected)
        .is_some_and(|s| s.slot == tab.slot);
    let detail: Value = device
        .live
        .as_ref()
        .filter(|l| is_shown && l.shown_thread == Some(thread))
        .and_then(|l| l.detail_json.as_deref())
        .and_then(|d| serde_json::from_str(d).ok())
        .unwrap_or(Value::Null);
    // A page: the window's own open run, as the person sees it.
    let page_run = match detail.get("runId").and_then(Value::as_str) {
        Some(run_id) => match svc.page_runs.get(run_id) {
            Some(run) => {
                let text = crate::pages::text::page_text(svc, svc.worktrees.project_dir(), &run)
                    .await
                    .map_err(failed)?;
                json!({
                    "page": run.page.id,
                    "title": run.page.title,
                    "params": run.params,
                    "focus": detail.get("focus").cloned().unwrap_or(Value::Null),
                    "source": run.page.origin.source,
                    "text": text,
                })
            }
            None => json!({ "error": format!("the window's run `{run_id}` isn't open now") }),
        },
        None => Value::Null,
    };
    // What it is (the URI without its `#`) and how it's viewed (the `#`'s
    // params: a filter, a pick, a line); a URI that isn't oxplow's is as it is.
    let (page, view) = match oxplow_domain::uri::oxplow_uri::OxplowUri::parse(&tab.id) {
        Ok(u) => (u.identity(), u.view().clone()),
        Err(_) => (tab.id.clone(), Default::default()),
    };
    let view: serde_json::Map<String, Value> = view
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    let others: Vec<Value> = selected
        .iter()
        .filter(|(d, _)| d.device != device.device)
        .map(|(d, t)| open_json(thread, d, t))
        .collect();
    let mut open = open_json(thread, device, tab);
    let fields = open.as_object_mut().expect("an object");
    // What it is and how it's viewed, apart: the id is only their string form.
    fields.insert("page".into(), json!(page));
    fields.insert("view".into(), Value::Object(view));
    fields.insert("detail".into(), detail);
    Ok(json!({ "open": open, "others": others, "pageRun": page_run }))
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
    let devices = devices_with_tabs(svc, Some(&wanted), Timestamp::now()).await?;
    let mut tabs: Vec<Value> = Vec::new();
    for device in &devices {
        for stored in &device.threads {
            let selected = stored.selected().map(|s| s.slot.as_str());
            for tab in &stored.tabs {
                let mut row = device.facts();
                row.insert("tab".into(), json!(tab.id));
                row.insert("kind".into(), json!(tab.kind));
                row.insert("title".into(), json!(tab.title));
                row.insert("thread".into(), json!(stored.thread.to_string()));
                row.insert(
                    "stream".into(),
                    json!(stream_of(stored.thread).map(|s| s.to_string())),
                );
                // A session's tab is its session: what the tab names, not
                // what the window called its kind.
                row.insert(
                    "session".into(),
                    json!((oxplow_domain::uri::build::resource_of(&tab.id).as_deref()
                        == Some(oxplow_domain::uri::resources::AGENT_SESSION))
                    .then(|| oxplow_domain::uri::build::id_of(
                        &tab.id,
                        oxplow_domain::uri::resources::AGENT_SESSION
                    )
                    .ok())
                    .flatten()),
                );
                row.insert(
                    "selected".into(),
                    json!(selected == Some(tab.slot.as_str())),
                );
                row.insert("visible".into(), json!(device.visible(stored.thread, tab)));
                device.report_facts(&mut row);
                tabs.push(Value::Object(row));
            }
        }
    }
    let unreported: Vec<String> = wanted
        .iter()
        .filter(|t| {
            !devices
                .iter()
                .any(|d| d.threads.iter().any(|s| s.thread == **t))
        })
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
    use crate::client_reports::TabReport;
    use crate::test_fixtures::services_with_thread;
    use oxplow_domain::{DeviceDescriptor, DeviceKind};

    /// The page load a test names `a`, `b`, `w1`…: a fixed `con-<uuid>` per
    /// name.
    pub(super) fn con(name: &str) -> String {
        let n: u32 = name.bytes().map(u32::from).sum();
        format!("con-00000000-0000-4000-8000-{n:012x}")
    }

    /// The device a test names: a fixed `dev-<uuid>` per name.
    pub(super) fn device(name: &str) -> DeviceId {
        let n: u32 = name.bytes().map(u32::from).sum();
        DeviceId::parse(&format!("dev-00000000-0000-4000-8000-{n:012x}")).unwrap()
    }

    pub(super) fn caller(name: &str) -> oxplow_domain::Caller {
        oxplow_domain::Caller {
            device: device(name),
            connection: oxplow_domain::ConnectionId::parse(&con(name)).unwrap(),
        }
    }

    /// The device `name` connects (its window is `con(name)`).
    pub(super) fn connect(svc: &Services, name: &str) {
        svc.client_devices
            .connected(
                DeviceDescriptor {
                    id: device(name),
                    name: format!("{name} device"),
                    kind: DeviceKind::Desktop,
                },
                caller(name).connection,
            )
            .unwrap();
    }

    /// `name`'s window reports showing `thread`.
    pub(super) fn report(svc: &Services, name: &str, focused: bool, thread: ThreadId) {
        svc.client_reports.report(
            &caller(name),
            TabReport {
                runs: Vec::new(),
                focused,
                visible: true,
                shown_thread: Some(thread),
                detail_json: Some(r#"{"line":3}"#.into()),
            },
            Timestamp::now(),
        );
    }

    /// `name`'s stored tabs of `thread`: one entry per tab (an id and its
    /// title), `active` selected.
    pub(super) async fn store(
        svc: &Services,
        name: &str,
        thread: ThreadId,
        tabs: &[(&str, &str)],
        active: usize,
    ) {
        let tabs: Vec<Value> = tabs
            .iter()
            .enumerate()
            .map(|(i, (id, title))| {
                json!({ "slot": format!("t{i}"), "at": 0,
                    "entries": [{ "ref": { "id": id }, "title": title }] })
            })
            .collect();
        let session = json!({ "tabs": tabs, "active": format!("t{active}"), "mru": [], "next": 9 });
        svc.tab_session_store
            .save(&device(name), &thread, session.to_string(), None)
            .await
            .unwrap();
    }

    /// The open page reads as what it is and how it's viewed: its identity
    /// (the URI without its `#`) and its view (the `#`'s params) apart, and
    /// where its tab's history leads — so an agent never parses a tab id.
    #[tokio::test]
    async fn the_open_page_says_its_page_its_view_and_its_history() {
        let fx = services_with_thread().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let id = "oxplow:oxplow/page?page=acme/commits#q=numbat%20bay&range=7d";
        let next = "oxplow:oxplow/page?page=acme/commits#commit=abc&q=numbat%20bay&range=7d";
        let home = "oxplow:oxplow/knowledge?id=home&store=oxwiki";
        let session = json!({
            "tabs": [{ "slot": "t1", "at": 1, "entries": [
                { "ref": { "id": home } },
                { "ref": { "id": id }, "title": "Commits",
                  "from": { "title": "Recent commits", "items": [], "index": 0 } },
                { "ref": { "id": next } },
            ] }],
            "active": "t1", "mru": ["t1"], "next": 2
        });
        fx.svc
            .tab_session_store
            .save(&device("a"), &fx.thread, session.to_string(), None)
            .await
            .unwrap();
        connect(&fx.svc, "a");
        report(&fx.svc, "a", true, fx.thread);
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
        assert_eq!(open["open"]["kind"], "acme/commits");
        assert_eq!(open["open"]["page"], "oxplow:oxplow/page?page=acme/commits");
        assert_eq!(
            open["open"]["view"],
            json!({ "range": "7d", "q": "numbat bay" })
        );
        assert_eq!(open["open"]["back"], json!([home]));
        assert_eq!(open["open"]["forward"], json!([next]));
        assert_eq!(open["open"]["origin"], "Recent commits");
    }

    /// What a device stored open in a thread reads back through the bus
    /// (null while nothing is stored), naming its device, with the other
    /// devices' pages beside it; a listed tab reads by its id; a non-thread
    /// id is refused at its field.
    #[tokio::test]
    async fn the_open_page_reads_through_the_bus() {
        let fx = services_with_thread().await;
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
        assert!(none.command_run_id.is_none(), "a read leaves no record");

        let home = "oxplow:oxplow/knowledge?id=home&store=oxwiki";
        let file = "oxplow:oxplow/file?path=src/a.rs";
        let settings = "oxplow:oxplow/page?page=settings";
        store(&fx.svc, "b", fx.thread, &[(home, "Home")], 0).await;
        store(
            &fx.svc,
            "a",
            fx.thread,
            &[(file, "a.rs"), (settings, "Settings")],
            0,
        )
        .await;
        connect(&fx.svc, "b");
        connect(&fx.svc, "a");
        report(&fx.svc, "b", false, fx.thread);
        report(&fx.svc, "a", true, fx.thread);
        // The person uses a last: it leads.
        fx.svc.client_devices.used(&device("a"));
        let open = run(json!({ "thread_id": tid })).await.unwrap().result;
        assert_eq!(open["open"]["pageId"], file);
        assert_eq!(open["open"]["client"], con("a"));
        assert_eq!(open["open"]["device"], device("a").to_string());
        assert_eq!(open["open"]["connected"], true);
        assert_eq!(open["open"]["visible"], true);
        assert_eq!(open["open"]["detail"]["line"], 3);
        assert_eq!(open["others"][0]["pageId"], home);
        assert_eq!(open["others"][0]["device"], device("b").to_string());

        let by_tab = run(json!({ "thread_id": tid, "tab": settings }))
            .await
            .unwrap()
            .result;
        assert_eq!(by_tab["open"]["pageId"], settings);
        assert_eq!(
            by_tab["open"]["detail"],
            Value::Null,
            "only the shown page publishes detail"
        );
        let err =
            run(json!({ "thread_id": tid, "tab": "oxplow:oxplow/knowledge?id=gone&store=oxwiki" }))
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
    /// project on request; a thread no device stored tabs for is named as
    /// such.
    #[tokio::test]
    async fn tabs_are_listed_by_scope() {
        let fx = services_with_thread().await;
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

        let home = "oxplow:oxplow/knowledge?id=home&store=oxwiki";
        store(
            &fx.svc,
            "a",
            fx.thread,
            &[
                (home, "Home"),
                ("oxplow:oxplow/agent_session?id=ses1", "Session"),
            ],
            0,
        )
        .await;
        connect(&fx.svc, "a");
        report(&fx.svc, "a", true, fx.thread);
        let listed = run(json!({ "scope": "stream" })).await;
        let rows = listed["tabs"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["tab"], home);
        assert_eq!(rows[0]["title"], "Home");
        assert_eq!(rows[0]["kind"], "oxplow/knowledge");
        assert_eq!(rows[0]["thread"], fx.thread.to_string());
        assert_eq!(rows[0]["client"], con("a"));
        assert_eq!(rows[0]["device"], device("a").to_string());
        assert_eq!(rows[0]["deviceName"], "a device");
        assert_eq!(rows[0]["connected"], true);
        assert_eq!(
            (rows[0]["selected"].clone(), rows[0]["visible"].clone()),
            (json!(true), json!(true))
        );
        assert_eq!(rows[0]["stale"], false);
        assert_eq!(rows[1]["session"], "ses1");
        assert_eq!(rows[1]["selected"], false);
        assert_eq!(rows[1]["visible"], false);
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

    /// A device that is not connected still lists its tabs: not connected,
    /// with no window to name, nothing on screen and no report to age.
    #[tokio::test]
    async fn a_device_that_is_away_still_lists_its_tabs() {
        let fx = services_with_thread().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let home = "oxplow:oxplow/knowledge?id=home&store=oxwiki";
        store(&fx.svc, "tablet", fx.thread, &[(home, "Home")], 0).await;
        let listed = fx
            .svc
            .commands
            .run(&actor, PAGE_LIST_TABS, json!({}), false)
            .await
            .unwrap()
            .result;
        let row = &listed["tabs"][0];
        assert_eq!(row["tab"], home);
        assert_eq!(row["connected"], false);
        assert_eq!(row["device"], device("tablet").to_string());
        assert_eq!(row["selected"], true);
        assert_eq!(row["visible"], false);
        assert!(row.get("client").is_none(), "{row}");
        assert!(row.get("stale").is_none(), "{row}");
        assert_eq!(listed["unreported"], json!([]));

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
        assert_eq!(open["open"]["pageId"], home);
        assert_eq!(open["open"]["connected"], false);
        assert_eq!(open["open"]["detail"], Value::Null);
    }

    /// The device the person used last leads, ahead of one that only
    /// reported more lately and of one that is away; a stored session that
    /// doesn't parse is skipped.
    #[tokio::test]
    async fn the_device_used_last_leads() {
        let fx = services_with_thread().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let page = |n: &str| format!("oxplow:oxplow/page?page={n}");
        for name in ["a", "b", "c"] {
            store(&fx.svc, name, fx.thread, &[(&page(name), name)], 0).await;
        }
        fx.svc
            .tab_session_store
            .save(&device("bad"), &fx.thread, "{\"tabs\":3}".into(), None)
            .await
            .unwrap();
        connect(&fx.svc, "a");
        connect(&fx.svc, "b");
        report(&fx.svc, "a", true, fx.thread);
        report(&fx.svc, "b", false, fx.thread);
        fx.svc.client_devices.used(&device("b"));
        let listed = fx
            .svc
            .commands
            .run(&actor, PAGE_LIST_TABS, json!({}), false)
            .await
            .unwrap()
            .result;
        let order: Vec<&str> = listed["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["title"].as_str().unwrap())
            .collect();
        assert_eq!(order, ["b", "a", "c"]);
    }
}

#[cfg(test)]
mod page_run_tests {
    use oxplow_domain::stores::StreamStore as _;
    use oxplow_domain::{Actor, Timestamp};
    use serde_json::json;

    use super::tests::{caller, connect, store};
    use super::PAGE_OPEN;
    use crate::pages::runner::{RunRequest, RunTarget};
    use crate::pages::test_support::fixture;

    /// What the person has open, when it is a page: the window's own open
    /// run as its text — what they see, with their params — and the node
    /// in focus.
    #[tokio::test]
    async fn the_open_page_is_the_windows_own_run_as_text() {
        let (_dir, svc) = fixture().await;
        let stream = svc.stream_store.primary().await.unwrap().unwrap().id;
        let thread = crate::test_fixtures::new_thread(&svc, stream, "t").await.id;
        let (run_id, _) = svc
            .page_runs
            .open(
                &svc,
                RunRequest {
                    target: RunTarget::Page("acme/numbers".into()),
                    params: [("n".to_string(), oxplow_db::SqlCell::Int(3))].into(),
                    ctx: crate::pages::spec::ViewContext::default(),
                    viewer: Actor::Human,
                },
                caller("w1").connection.as_str(),
            )
            .await
            .unwrap();
        let id = "oxplow:oxplow/page?page=acme/numbers#n=3&node=big";
        store(&svc, "w1", thread, &[(id, "Numbers")], 0).await;
        connect(&svc, "w1");
        svc.client_reports.report(
            &caller("w1"),
            crate::client_reports::TabReport {
                runs: Vec::new(),
                focused: true,
                visible: true,
                shown_thread: Some(thread),
                detail_json: Some(
                    json!({ "runId": run_id, "focus": { "node": "big" } }).to_string(),
                ),
            },
            Timestamp::now(),
        );
        let open = svc
            .commands
            .run(
                &Actor::Human,
                PAGE_OPEN,
                json!({ "thread_id": thread.to_string() }),
                false,
            )
            .await
            .unwrap()
            .result;
        let page = &open["pageRun"];
        assert_eq!(page["page"], json!("acme/numbers"), "{open}");
        assert_eq!(page["focus"], json!({ "node": "big" }), "{open}");
        let text = page["text"].as_str().unwrap();
        assert!(
            text.contains("[node big]") && text.contains("| 3 |"),
            "{text}"
        );
    }
}
