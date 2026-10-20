//! What the person's windows show (`.context/pages-and-tabs.md` "What
//! agents see of the tabs"): each window (a client, by the id its renderer
//! keeps for its life) reports its tabs per thread, which one is selected
//! in each, the thread it shows, whether it is focused and visible, and the
//! detail its active page publishes (a lens's params). An agent reads them
//! with `oxplow_list_tabs` and `oxplow_get_open_page`.
//!
//! Held in memory: nothing here outlives the process, and a client that
//! stops reporting (a closed window) is forgotten after [`FORGET_AFTER`]. A
//! report older than [`STALE_AFTER`] is still listed, marked stale.

use std::collections::HashMap;
use std::sync::Mutex;

use oxplow_domain::{ThreadId, Timestamp};
use serde::{Deserialize, Serialize};

/// A client not heard from for this long is gone.
pub const FORGET_AFTER_MS: i64 = 5 * 60 * 1000;
/// A report older than this is listed as stale.
pub const STALE_AFTER_MS: i64 = 60 * 1000;

/// One tab, as its window shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ReportedTab {
    /// Its tab id: a canonical ref (`work_item:oxplow:tsk3`,
    /// `page:settings`, `agent_session:ses2`).
    pub id: String,
    /// Its page kind (`work_item`, `lens`, `agent_session`).
    pub kind: String,
    /// The title on the tab.
    pub title: String,
    /// Where its Back leads: the ids of the entries before the one it
    /// shows, oldest first.
    #[serde(default)]
    pub back: Vec<String>,
    /// Where its Forward leads (the ids of the entries after it).
    #[serde(default)]
    pub forward: Vec<String>,
    /// The collection the page was opened from (its title), whose items
    /// Up and Down step through.
    #[serde(default)]
    pub origin: Option<String>,
}

/// A thread's tabs in one window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ThreadTabs {
    pub thread: ThreadId,
    pub tabs: Vec<ReportedTab>,
    /// The tab selected in the thread (its id); none when none is.
    pub active: Option<String>,
}

/// What one window shows, as its renderer reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct TabReport {
    /// The window's renderer, for as long as it lives.
    pub client: String,
    pub focused: bool,
    pub visible: bool,
    /// The thread the window shows.
    pub shown_thread: Option<ThreadId>,
    pub threads: Vec<ThreadTabs>,
    /// The detail the shown thread's active page publishes (JSON text: a
    /// lens's `{lensId, params}`, an editor's `{path, revision, selection}`).
    pub detail_json: Option<String>,
}

#[derive(Debug, Clone)]
struct Client {
    report: TabReport,
    reported_at: Timestamp,
    /// When it last reported being focused: which window the person is in.
    focused_at: Option<Timestamp>,
}

/// One tab, with where and how it is shown, for an agent.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListedTab {
    pub tab: String,
    pub kind: String,
    pub title: String,
    pub thread: ThreadId,
    /// Selected in its thread in this window.
    pub selected: bool,
    /// On screen: selected, in the thread its window shows, in a visible
    /// window.
    pub visible: bool,
    pub client: String,
    #[serde(rename = "reportedAt")]
    pub reported_at: Timestamp,
    pub stale: bool,
}

/// The page a thread has open in one window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenPage {
    #[serde(rename = "pageId")]
    pub page_id: String,
    pub kind: String,
    /// The window's published detail, when this is the page it shows.
    #[serde(skip)]
    pub detail_json: Option<String>,
    pub client: String,
    /// It is on screen (the window shows its thread and is visible).
    pub visible: bool,
    #[serde(rename = "reportedAt")]
    pub reported_at: Timestamp,
    pub stale: bool,
    /// Its tab's history and origin (`ReportedTab`).
    pub back: Vec<String>,
    pub forward: Vec<String>,
    pub origin: Option<String>,
}

#[derive(Debug, Default)]
pub struct ClientReports {
    clients: Mutex<HashMap<String, Client>>,
}

impl ClientReports {
    pub fn new() -> Self {
        Self::default()
    }

    /// Take `report` as what its window shows at `now`.
    pub fn report(&self, report: TabReport, now: Timestamp) {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let focused_at = if report.focused {
            Some(now)
        } else {
            clients.get(&report.client).and_then(|c| c.focused_at)
        };
        clients.insert(
            report.client.clone(),
            Client {
                report,
                reported_at: now,
                focused_at,
            },
        );
    }

    /// The live clients at `now`, the one the person was in most recently
    /// first; the gone ones are forgotten.
    fn live(&self, now: Timestamp) -> Vec<Client> {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.retain(|_, c| now.unix_ms() - c.reported_at.unix_ms() <= FORGET_AFTER_MS);
        let mut out: Vec<Client> = clients.values().cloned().collect();
        out.sort_by(|a, b| {
            b.focused_at
                .cmp(&a.focused_at)
                .then(b.reported_at.cmp(&a.reported_at))
                .then(a.report.client.cmp(&b.report.client))
        });
        out
    }

    /// Every tab of `threads` in every window at `now`, the person's
    /// latest window first.
    pub fn tabs(&self, threads: &[ThreadId], now: Timestamp) -> Vec<ListedTab> {
        let mut out = Vec::new();
        for c in self.live(now) {
            let stale = now.unix_ms() - c.reported_at.unix_ms() > STALE_AFTER_MS;
            for t in c
                .report
                .threads
                .iter()
                .filter(|t| threads.contains(&t.thread))
            {
                for tab in &t.tabs {
                    let selected = t.active.as_deref() == Some(tab.id.as_str());
                    out.push(ListedTab {
                        tab: tab.id.clone(),
                        kind: tab.kind.clone(),
                        title: tab.title.clone(),
                        thread: t.thread,
                        selected,
                        visible: selected
                            && c.report.visible
                            && c.report.shown_thread == Some(t.thread),
                        client: c.report.client.clone(),
                        reported_at: c.reported_at,
                        stale,
                    });
                }
            }
        }
        out
    }

    /// The page `thread` has selected in each window at `now`, the
    /// person's latest window first.
    pub fn open_pages(&self, thread: ThreadId, now: Timestamp) -> Vec<OpenPage> {
        let mut out = Vec::new();
        for c in self.live(now) {
            let Some(t) = c.report.threads.iter().find(|t| t.thread == thread) else {
                continue;
            };
            let Some(active) = &t.active else { continue };
            let Some(tab) = t.tabs.iter().find(|tab| &tab.id == active) else {
                continue;
            };
            let shown = c.report.shown_thread == Some(thread);
            out.push(OpenPage {
                page_id: tab.id.clone(),
                kind: tab.kind.clone(),
                detail_json: shown.then(|| c.report.detail_json.clone()).flatten(),
                client: c.report.client.clone(),
                visible: shown && c.report.visible,
                reported_at: c.reported_at,
                stale: now.unix_ms() - c.reported_at.unix_ms() > STALE_AFTER_MS,
                back: tab.back.clone(),
                forward: tab.forward.clone(),
                origin: tab.origin.clone(),
            });
        }
        out
    }

    /// The threads any live window reported at `now`.
    pub fn reported_threads(&self, now: Timestamp) -> Vec<ThreadId> {
        let mut out: Vec<ThreadId> = self
            .live(now)
            .iter()
            .flat_map(|c| c.report.threads.iter().map(|t| t.thread))
            .collect();
        out.sort_by_key(|t| t.value());
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: i64) -> Timestamp {
        Timestamp::from_unix_ms(ms)
    }

    fn tab(id: &str, kind: &str) -> ReportedTab {
        ReportedTab {
            id: id.into(),
            kind: kind.into(),
            title: id.into(),
            back: Vec::new(),
            forward: Vec::new(),
            origin: None,
        }
    }

    fn report(client: &str, focused: bool, shown: i64, threads: Vec<ThreadTabs>) -> TabReport {
        TabReport {
            client: client.into(),
            focused,
            visible: true,
            shown_thread: Some(ThreadId::new(shown)),
            threads,
            detail_json: Some(format!("{{\"from\":\"{client}\"}}")),
        }
    }

    fn thread(id: i64, tabs: Vec<ReportedTab>, active: Option<&str>) -> ThreadTabs {
        ThreadTabs {
            thread: ThreadId::new(id),
            tabs,
            active: active.map(str::to_string),
        }
    }

    /// Two windows don't collapse into one "open page": each says what it
    /// has selected, the window the person was in last first, and only the
    /// window showing the thread carries its detail.
    #[test]
    fn two_windows_each_report_their_own_page() {
        let r = ClientReports::new();
        r.report(
            report(
                "a",
                true,
                1,
                vec![thread(1, vec![tab("wiki:home", "wiki")], Some("wiki:home"))],
            ),
            at(1_000),
        );
        r.report(
            report(
                "b",
                false,
                2,
                vec![
                    thread(1, vec![tab("file:src/a.rs", "file")], Some("file:src/a.rs")),
                    thread(
                        2,
                        vec![tab("page:settings", "settings")],
                        Some("page:settings"),
                    ),
                ],
            ),
            at(2_000),
        );
        let pages = r.open_pages(ThreadId::new(1), at(3_000));
        assert_eq!(
            pages
                .iter()
                .map(|p| (p.client.as_str(), p.page_id.as_str(), p.visible))
                .collect::<Vec<_>>(),
            [("a", "wiki:home", true), ("b", "file:src/a.rs", false)]
        );
        assert_eq!(pages[0].detail_json.as_deref(), Some("{\"from\":\"a\"}"));
        assert_eq!(pages[1].detail_json, None, "b shows another thread");
        // The person moves to b: it leads.
        r.report(
            report(
                "b",
                true,
                1,
                vec![thread(
                    1,
                    vec![tab("file:src/a.rs", "file")],
                    Some("file:src/a.rs"),
                )],
            ),
            at(4_000),
        );
        assert_eq!(r.open_pages(ThreadId::new(1), at(4_500))[0].client, "b");
    }

    #[test]
    fn tabs_are_listed_per_thread_with_selection_visibility_and_staleness() {
        let r = ClientReports::new();
        r.report(
            report(
                "a",
                true,
                1,
                vec![
                    thread(
                        1,
                        vec![
                            tab("wiki:home", "wiki"),
                            tab("agent_session:ses2", "agent_session"),
                        ],
                        Some("wiki:home"),
                    ),
                    thread(
                        2,
                        vec![tab("page:settings", "settings")],
                        Some("page:settings"),
                    ),
                ],
            ),
            at(0),
        );
        let listed = r.tabs(&[ThreadId::new(1)], at(1_000));
        assert_eq!(
            listed
                .iter()
                .map(|t| (t.tab.as_str(), t.selected, t.visible, t.stale))
                .collect::<Vec<_>>(),
            [
                ("wiki:home", true, true, false),
                ("agent_session:ses2", false, false, false)
            ]
        );
        // Thread 2's tab is selected there but not on screen.
        let other = r.tabs(&[ThreadId::new(2)], at(1_000));
        assert_eq!((other[0].selected, other[0].visible), (true, false));
        assert!(r.tabs(&[ThreadId::new(1)], at(STALE_AFTER_MS + 1))[0].stale);
        assert_eq!(
            r.reported_threads(at(1_000)),
            [ThreadId::new(1), ThreadId::new(2)]
        );
    }

    #[test]
    fn a_window_that_stops_reporting_is_forgotten() {
        let r = ClientReports::new();
        r.report(
            report(
                "a",
                true,
                1,
                vec![thread(1, vec![tab("wiki:home", "wiki")], Some("wiki:home"))],
            ),
            at(0),
        );
        assert_eq!(r.open_pages(ThreadId::new(1), at(FORGET_AFTER_MS)).len(), 1);
        assert!(r
            .open_pages(ThreadId::new(1), at(FORGET_AFTER_MS + 1))
            .is_empty());
        assert!(r
            .tabs(&[ThreadId::new(1)], at(FORGET_AFTER_MS + 1))
            .is_empty());
    }
}
