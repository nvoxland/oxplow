//! A device's tabs as the daemon stores them (`tab_session`): the window's
//! `Session` (`apps/desktop/src/tabs/session.ts`), read here for the little an
//! agent needs of it — each tab's current page, its label, where its Back and
//! Forward lead, where it was opened from, and which tab is selected.
//! Read-only: only the window writes a session.

use oxplow_domain::uri::build as refs;
use oxplow_domain::uri::oxplow_uri::OxplowUri;
use oxplow_domain::uri::resources as res;
use oxplow_domain::{DeviceId, ThreadId};
use serde::Deserialize;

/// One tab of a stored session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredTab {
    pub slot: String,
    /// The page it shows now: a canonical URI (or a web URL).
    pub id: String,
    /// Its page kind (`task`, `settings`, `acme/pr`, `https`).
    pub kind: String,
    /// The title the strip shows; the kind when the entry has none.
    pub title: String,
    /// The ids of the entries before the one it shows, oldest first.
    pub back: Vec<String>,
    /// The ids of the entries after it.
    pub forward: Vec<String>,
    /// The collection it was opened from (its title).
    pub origin: Option<String>,
}

/// One device's tabs of one thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredThread {
    pub device: DeviceId,
    pub thread: ThreadId,
    pub tabs: Vec<StoredTab>,
    /// The slot selected; none when none is.
    pub active: Option<String>,
}

impl StoredThread {
    /// The tab selected in the thread.
    pub fn selected(&self) -> Option<&StoredTab> {
        let active = self.active.as_deref()?;
        self.tabs.iter().find(|t| t.slot == active)
    }
}

#[derive(Deserialize)]
struct Session {
    #[serde(default)]
    tabs: Vec<serde_json::Value>,
    #[serde(default)]
    active: Option<String>,
}

/// One tab; read on its own, so a bad one loses only itself.
#[derive(Deserialize)]
struct Tab {
    slot: String,
    entries: Vec<Entry>,
    #[serde(default)]
    at: usize,
}

#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "ref")]
    page: Ref,
    #[serde(default)]
    title: Option<String>,
    /// The strip's label, stored beside the title (which stays what the page
    /// registered).
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    from: Option<Origin>,
}

#[derive(Deserialize)]
struct Ref {
    id: String,
}

#[derive(Deserialize)]
struct Origin {
    #[serde(default)]
    title: Option<String>,
}

/// What a tab shows, as visits record it (`recordedPageKind` in
/// `apps/desktop/src/tabs/pageRefs.ts`): a route's page (`tasks`, `acme/pr`),
/// else the resource its URI names, a web page's scheme, else `link`.
pub fn page_kind(id: &str) -> String {
    if refs::resource_of(id).as_deref() == Some(res::PAGE) {
        if let Some(head) = OxplowUri::parse(id)
            .ok()
            .and_then(|u| u.param("page").map(str::to_string))
        {
            // `<own namespace>/<id>` is oxplow's own page too, as a bare id
            // names it; another's is `<namespace>/<id>`.
            let own = res::PAGE.split_once('/').map_or("", |(ns, _)| ns);
            match head.split_once('/') {
                None if !head.is_empty() => return head,
                Some((ns, page)) if !ns.is_empty() && !page.is_empty() && !page.contains('/') => {
                    return if ns == own { page.to_string() } else { head };
                }
                _ => {}
            }
        }
    }
    if let Some(resource) = refs::resource_of(id) {
        return resource;
    }
    let lower = id.to_ascii_lowercase();
    for scheme in ["https", "http"] {
        if lower.starts_with(&format!("{scheme}://")) {
            return scheme.to_string();
        }
    }
    "link".to_string()
}

/// `json` (the window's session) as `device`'s tabs of `thread`. A session
/// that doesn't parse is an error: the caller skips it. A tab that doesn't
/// read, or has no entry at its position, is left out.
pub fn parse(
    device: DeviceId,
    thread: ThreadId,
    json: &str,
) -> Result<StoredThread, serde_json::Error> {
    let session: Session = serde_json::from_str(json)?;
    let tabs = session
        .tabs
        .into_iter()
        .filter_map(|tab| {
            let tab: Tab = serde_json::from_value(tab).ok()?;
            let now = tab.entries.get(tab.at)?;
            let id = now.page.id.clone();
            let kind = page_kind(&id);
            Some(StoredTab {
                title: now
                    .label
                    .clone()
                    .filter(|t| !t.is_empty())
                    .or_else(|| now.title.clone().filter(|t| !t.is_empty()))
                    .unwrap_or_else(|| kind.clone()),
                back: tab.entries[..tab.at]
                    .iter()
                    .map(|e| e.page.id.clone())
                    .collect(),
                forward: tab.entries[tab.at + 1..]
                    .iter()
                    .map(|e| e.page.id.clone())
                    .collect(),
                origin: now.from.as_ref().and_then(|f| f.title.clone()),
                slot: tab.slot,
                id,
                kind,
            })
        })
        .collect();
    Ok(StoredThread {
        device,
        thread,
        tabs,
        active: session.active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parsed(session: serde_json::Value) -> StoredThread {
        parse(DeviceId::generate(), ThreadId::new(1), &session.to_string()).unwrap()
    }

    /// The kind is what the window records: the route's page, else the
    /// resource, else the web scheme, else `link`.
    #[test]
    fn a_tabs_kind_is_the_pages_resource_scheme_or_link() {
        assert_eq!(page_kind("oxplow:oxplow/page?page=settings"), "settings");
        assert_eq!(page_kind("oxplow:oxplow/page?page=oxplow/tasks"), "tasks");
        assert_eq!(
            page_kind("oxplow:oxplow/page?page=acme/commits#q=1"),
            "acme/commits"
        );
        assert_eq!(page_kind("oxplow:oxplow/file?path=src/a.rs"), "oxplow/file");
        assert_eq!(
            page_kind("oxplow:oxplow/agent_session?id=ses2"),
            "oxplow/agent_session"
        );
        assert_eq!(page_kind("https://example.com/x"), "https");
        assert_eq!(page_kind("HTTP://example.com/x"), "http");
        assert_eq!(page_kind("something else"), "link");
    }

    /// A tab reads as its current entry: id, title (else the kind), the ids
    /// before and after it, and where it was opened from; the active slot is
    /// the selected one.
    #[test]
    fn a_stored_session_reads_as_tabs_with_history_and_selection() {
        let s = parsed(json!({
            "tabs": [
                { "slot": "t1", "at": 1, "entries": [
                    { "ref": { "id": "oxplow:oxplow/page?page=a" } },
                    { "ref": { "id": "oxplow:oxplow/page?page=b" }, "title": "B page",
                      "from": { "title": "Pages", "items": [], "index": 0 }, "historyState": { "x": 1 } },
                    { "ref": { "id": "oxplow:oxplow/page?page=c" } } ] },
                { "slot": "t2", "entries": [ { "ref": { "id": "oxplow:oxplow/file?path=a.rs" } } ] },
                { "slot": "t3", "at": 4, "entries": [ { "ref": { "id": "x" } } ] }
            ],
            "active": "t2", "mru": ["t2"], "next": 4
        }));
        assert_eq!(s.tabs.len(), 2, "a tab with no entry at `at` is left out");
        let t1 = &s.tabs[0];
        assert_eq!(
            (t1.id.as_str(), t1.title.as_str()),
            ("oxplow:oxplow/page?page=b", "B page")
        );
        assert_eq!(t1.back, ["oxplow:oxplow/page?page=a"]);
        assert_eq!(t1.forward, ["oxplow:oxplow/page?page=c"]);
        assert_eq!(t1.origin.as_deref(), Some("Pages"));
        assert_eq!(s.tabs[1].title, "oxplow/file", "no title: the kind");
        assert_eq!(s.selected().unwrap().slot, "t2");
    }

    /// The window drops one bad tab and keeps the others, and an origin
    /// needs no title: Rust reads a session as loosely.
    #[test]
    fn a_bad_tab_or_an_origin_with_no_title_loses_only_itself() {
        let s = parsed(json!({
            "tabs": [
                { "slot": "t1", "entries": [
                    { "ref": { "id": "oxplow:oxplow/page?page=a" }, "from": { "items": [] } } ] },
                { "entries": [ { "ref": { "id": "oxplow:oxplow/page?page=b" } } ] },
                { "slot": "t3", "entries": [ { "ref": {} } ] },
                { "slot": "t4", "at": -1, "entries": [ { "ref": { "id": "x" } } ] },
                { "slot": "t5", "entries": [ { "ref": { "id": "oxplow:oxplow/page?page=e" } } ] }
            ]
        }));
        let slots: Vec<_> = s.tabs.iter().map(|t| t.slot.as_str()).collect();
        assert_eq!(slots, ["t1", "t5"]);
        assert_eq!(s.tabs[0].origin, None);
    }

    /// The strip's label is its own field, which the strip never reads
    /// back: it wins over the page's registered title.
    #[test]
    fn the_strip_label_wins_over_the_registered_title() {
        let s = parsed(json!({
            "tabs": [
                { "slot": "t1", "entries": [
                    { "ref": { "id": "oxplow:oxplow/page?page=a" }, "title": "Page title", "label": "Strip label" } ] },
                { "slot": "t2", "entries": [
                    { "ref": { "id": "oxplow:oxplow/page?page=b" }, "title": "Page title", "label": "" } ] }
            ]
        }));
        assert_eq!(s.tabs[0].title, "Strip label");
        assert_eq!(s.tabs[1].title, "Page title");
    }

    #[test]
    fn a_session_that_does_not_parse_is_an_error() {
        assert!(parse(DeviceId::generate(), ThreadId::new(1), "{\"tabs\":3}").is_err());
        assert!(parse(DeviceId::generate(), ThreadId::new(1), "nope").is_err());
    }
}
