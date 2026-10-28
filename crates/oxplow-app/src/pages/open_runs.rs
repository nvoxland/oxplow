//! The page runs windows have open: each run kept with the request that
//! made it, re-run when a dataset one of its loaders read changes (or its
//! params change), so a window hears which loaders' rows changed
//! (`OxplowEvent::PageRunChanged`) and fetches only those. The change loop
//! (`datasets_changed::spawn`) tells it what changed; nothing here listens to
//! the event bus.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::{DomainError, Timestamp};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use super::runner::{PageRun, PageRunner, RunNode, RunRequest};

/// What a re-run changed: the loaders whose rows differ (embedded pages'
/// as `<embed id>/<loader>`), and whether its tree did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RunChange {
    pub run_id: String,
    /// Counts up with each change, so a window drops a fetch older than
    /// one it applied.
    #[specta(type = oxplow_domain::JsNumber)]
    pub seq: u64,
    pub loaders: Vec<String>,
    pub tree: bool,
}

/// A run and the sequence of that exact snapshot, captured under one lock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub run: PageRun,
    #[specta(type = oxplow_domain::JsNumber)]
    pub seq: u64,
}

/// What the change loop tells the open runs.
#[derive(Debug, Clone)]
enum Signal {
    Datasets(Vec<String>),
    Measures(Vec<String>),
    Everything,
}

/// Latest invalidation per dependency. Opening a page may itself record
/// command runs: unrelated writes must not keep its first read retrying.
#[derive(Default)]
struct Invalidations {
    serial: u64,
    everything: u64,
    datasets: HashMap<String, u64>,
    measures: HashMap<String, u64>,
}

impl Invalidations {
    fn record(&mut self, signal: &Signal) {
        self.serial += 1;
        match signal {
            Signal::Everything => self.everything = self.serial,
            Signal::Datasets(names) => {
                for name in names {
                    self.datasets.insert(name.clone(), self.serial);
                }
            }
            Signal::Measures(names) => {
                for name in names {
                    self.measures.insert(name.clone(), self.serial);
                }
            }
        }
    }

    fn reaches(&self, run: &PageRun, since: u64) -> bool {
        self.everything > since
            || reads_any(
                run,
                &self
                    .datasets
                    .iter()
                    .filter(|(_, serial)| **serial > since)
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>(),
                &self
                    .measures
                    .iter()
                    .filter(|(_, serial)| **serial > since)
                    .map(|(name, _)| name.clone())
                    .collect::<Vec<_>>(),
            )
    }
}

struct OpenRun {
    req: RunRequest,
    run: PageRun,
    client: String,
    seq: u64,
    /// A later refresh/parameter request supersedes earlier async work.
    revision: u64,
    opened_at: Timestamp,
    reported: bool,
}

/// An ephemeral page an agent showed (`oxplow.page.show { spec }`): its
/// YAML, kept in memory so a window can open it by id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub title: String,
    /// The thread that showed it.
    pub thread: Option<String>,
    pub spec_yaml: String,
}

/// The open runs.
#[derive(Clone, Default)]
pub struct PageRuns {
    runs: Arc<Mutex<HashMap<String, OpenRun>>>,
    views: Arc<Mutex<HashMap<String, View>>>,
    /// Where the change loop's signals go once [`PageRuns::start`] ran.
    signals: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedSender<Signal>>>>,
    connections: Arc<Mutex<HashMap<String, uuid::Uuid>>>,
    /// Register interest before the first read. Publishing a new run and
    /// invalidation use this same fence, so neither can miss the other.
    revision: Arc<Mutex<Invalidations>>,
}

/// One event socket's ownership of a window. A late drop from a socket
/// replaced on reconnect cannot close the new socket's runs.
pub struct PageClient {
    runs: PageRuns,
    client: String,
    connection: uuid::Uuid,
}

impl Drop for PageClient {
    fn drop(&mut self) {
        let mut connections = self.runs.connections.lock();
        if connections.get(&self.client) != Some(&self.connection) {
            return;
        }
        connections.remove(&self.client);
        self.runs.runs.lock().retain(|_, r| r.client != self.client);
    }
}

/// How long signals gather before the open runs re-run: a burst of
/// commits is one re-run.
const COALESCE: std::time::Duration = std::time::Duration::from_millis(100);
/// A change's re-run of one open run taking this long is logged: the
/// window waits on it, so it shows up as lag.
const SLOW_RERUN: std::time::Duration = std::time::Duration::from_millis(250);

impl PageRuns {
    pub fn connect_client(&self, client: &str) -> PageClient {
        let connection = uuid::Uuid::now_v7();
        let mut connections = self.connections.lock();
        connections.insert(client.to_string(), connection);
        self.runs.lock().retain(|_, r| r.client != client);
        PageClient {
            runs: self.clone(),
            client: client.to_string(),
            connection,
        }
    }

    /// Re-run open runs as the change loop says datasets change, and tell
    /// the windows (`PageRunChanged`), for the life of `svc`.
    pub fn start(&self, svc: &Arc<crate::Services>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Signal>();
        *self.signals.lock() = Some(tx);
        let weak = Arc::downgrade(svc);
        let runs = self.clone();
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                tokio::time::sleep(COALESCE).await;
                let mut signals = vec![first];
                while let Ok(more) = rx.try_recv() {
                    signals.push(more);
                }
                let Some(svc) = weak.upgrade() else { return };
                let changes = if signals.iter().any(|s| matches!(s, Signal::Everything)) {
                    runs.all_changed(&svc).await
                } else {
                    let mut datasets = Vec::new();
                    let mut measures = Vec::new();
                    for s in signals {
                        match s {
                            Signal::Datasets(m) => datasets.extend(m),
                            Signal::Measures(m) => measures.extend(m),
                            Signal::Everything => {}
                        }
                    }
                    runs.changed(&svc, &datasets, &measures).await
                };
                for change in changes {
                    svc.events
                        .emit(crate::events::OxplowEvent::PageRunChanged(change));
                }
            }
        });
    }

    fn invalidate(&self, signal: &Signal) {
        self.revision.lock().record(signal);
    }

    fn signal(&self, signal: Signal) {
        self.invalidate(&signal);
        if let Some(tx) = self.signals.lock().as_ref() {
            let _ = tx.send(signal);
        }
    }

    /// The change loop: these published datasets changed.
    pub fn notify_datasets(&self, datasets: &[String]) {
        if !datasets.is_empty() {
            self.signal(Signal::Datasets(datasets.to_vec()));
        }
    }

    /// The change loop: facts landed for these measures (empty: unknown).
    pub fn notify_measures(&self, measures: &[String]) {
        self.signal(match measures {
            [] => Signal::Everything,
            m => Signal::Measures(m.to_vec()),
        });
    }

    /// The change loop missed something, or the catalog changed: anything
    /// may have.
    pub fn notify_everything(&self) {
        self.signal(Signal::Everything);
    }

    /// Run `req` and keep it open for `client` (a window).
    pub async fn open(
        &self,
        svc: &crate::Services,
        req: RunRequest,
        client: &str,
    ) -> Result<(String, PageRun), DomainError> {
        let connection = self.connections.lock().get(client).copied();
        loop {
            let observed = self.revision.lock().serial;
            let run = PageRunner::of(svc).run(req.clone()).await?;
            // Keep the invalidation fence until insertion: a notification
            // either forces another read here or finds the published run.
            let revision = self.revision.lock();
            let connections = self.connections.lock();
            if connections.get(client).copied() != connection {
                return Err(DomainError::Invalid(
                    "the window disconnected while its page opened".into(),
                ));
            }
            if revision.reaches(&run, observed) {
                continue;
            }
            let id = uuid::Uuid::now_v7().to_string();
            self.runs.lock().insert(
                id.clone(),
                OpenRun {
                    req,
                    run: run.clone(),
                    client: client.to_string(),
                    seq: 0,
                    revision: 0,
                    opened_at: Timestamp::now(),
                    reported: false,
                },
            );
            return Ok((id, run));
        }
    }

    /// Keep an inline page an agent showed; its id is what its uri names.
    pub fn keep_view(&self, title: &str, thread: Option<String>, spec_yaml: &str) -> String {
        let id = uuid::Uuid::now_v7().simple().to_string();
        self.views.lock().insert(
            id.clone(),
            View {
                id: id.clone(),
                title: title.to_string(),
                thread,
                spec_yaml: spec_yaml.to_string(),
            },
        );
        id
    }

    /// The view kept as `id`.
    pub fn view(&self, id: &str) -> Option<View> {
        self.views.lock().get(id).cloned()
    }

    pub fn close(&self, id: &str) {
        self.runs.lock().remove(id);
    }

    pub fn get(&self, id: &str) -> Option<PageRun> {
        self.runs.lock().get(id).map(|r| r.run.clone())
    }

    pub fn snapshot(&self, id: &str) -> Option<RunSnapshot> {
        self.runs.lock().get(id).map(|r| RunSnapshot {
            run: r.run.clone(),
            seq: r.seq,
        })
    }

    /// The open runs' ids.
    pub fn ids(&self) -> Vec<String> {
        self.runs.lock().keys().cloned().collect()
    }

    /// Drop every run whose window isn't among `clients`.
    pub fn keep_only_clients(&self, clients: &[String]) {
        self.runs.lock().retain(|_, r| clients.contains(&r.client));
    }

    /// Reconcile an ordered report from one window. A new run whose
    /// response has not reached the renderer yet has never been reported
    /// and must survive an older roster already in flight.
    pub fn report_client_runs(&self, client: &str, ids: &[String]) {
        self.runs.lock().retain(|id, run| {
            if run.client != client {
                return true;
            }
            if ids.contains(id) {
                run.reported = true;
                return true;
            }
            !run.reported
        });
    }

    /// Forget runs of expired windows. Give never-reported new opens the
    /// same first-report grace as a window's heartbeat expiry.
    pub fn reap_expired_clients(&self, clients: &[String], now: Timestamp) {
        self.runs.lock().retain(|_, r| {
            (r.reported && clients.contains(&r.client))
                || now.unix_ms() - r.opened_at.unix_ms() <= crate::client_reports::FORGET_AFTER_MS
        });
    }

    /// Re-run `id` with `params` over its own (a pick, a filter, a switch's
    /// param).
    pub async fn set_params(
        &self,
        svc: &crate::Services,
        id: &str,
        params: BTreeMap<String, SqlCell>,
    ) -> Result<(PageRun, RunChange), DomainError> {
        self.rerun_with(svc, id, params, None).await
    }

    /// Re-run `id` as it is (Refresh).
    pub async fn rerun(
        &self,
        svc: &crate::Services,
        id: &str,
    ) -> Result<(PageRun, RunChange), DomainError> {
        self.rerun_with(svc, id, BTreeMap::new(), None).await
    }

    async fn rerun_with(
        &self,
        svc: &crate::Services,
        id: &str,
        params: BTreeMap<String, SqlCell>,
        changed: Option<(&[String], &[String])>,
    ) -> Result<(PageRun, RunChange), DomainError> {
        let (revision, req, seed) = {
            let mut runs = self.runs.lock();
            let open = runs
                .get_mut(id)
                .ok_or_else(|| DomainError::Invalid(format!("no open page run `{id}`")))?;
            // Merge desired params, reserve this request and capture its
            // memo under one lock. A concurrent refresh cannot reserve a
            // newer ticket with a stale request copied before this update.
            open.req.params.extend(params);
            open.revision = open.revision.wrapping_add(1);
            let seed = match changed {
                Some((datasets, measures)) => {
                    super::runner::unreached(&open.run, datasets, measures)
                }
                None => HashMap::new(),
            };
            (open.revision, open.req.clone(), seed)
        };
        let fresh = PageRunner::of(svc).run_seeded(req.clone(), seed).await?;
        let mut runs = self.runs.lock();
        let open = runs
            .get_mut(id)
            .ok_or_else(|| DomainError::Invalid(format!("no open page run `{id}`")))?;
        if open.revision != revision {
            // A newer request owns this run. A stale completion neither
            // publishes its rows nor restores its older params.
            return Ok((
                open.run.clone(),
                RunChange {
                    run_id: id.to_string(),
                    seq: open.seq,
                    loaders: Vec::new(),
                    tree: false,
                },
            ));
        }
        let mut change = diff(&open.run, &fresh);
        open.seq += 1;
        change.run_id = id.to_string();
        change.seq = open.seq;
        open.req = req;
        open.run = fresh.clone();
        Ok((fresh, change))
    }

    /// Re-run each open run one of whose loaders read one of `datasets`
    /// (embedded pages' included). What changed, per run that changed.
    pub async fn datasets_changed(
        &self,
        svc: &crate::Services,
        datasets: &[String],
    ) -> Vec<RunChange> {
        self.changed(svc, datasets, &[]).await
    }

    /// Re-run each open run that read one of `datasets` or `measures`.
    pub async fn changed(
        &self,
        svc: &crate::Services,
        datasets: &[String],
        measures: &[String],
    ) -> Vec<RunChange> {
        self.invalidate(&Signal::Datasets(datasets.to_vec()));
        self.invalidate(&Signal::Measures(measures.to_vec()));
        let ids: Vec<String> = self
            .runs
            .lock()
            .iter()
            .filter(|(_, r)| reads_any(&r.run, datasets, measures))
            .map(|(id, _)| id.clone())
            .collect();
        self.rerun_changed(svc, ids, Some((datasets, measures)))
            .await
    }

    /// Re-run every open run (the catalog changed, or the change loop
    /// missed something).
    pub async fn all_changed(&self, svc: &crate::Services) -> Vec<RunChange> {
        self.invalidate(&Signal::Everything);
        let ids = self.ids();
        self.rerun_all(svc, ids).await
    }

    async fn rerun_all(&self, svc: &crate::Services, ids: Vec<String>) -> Vec<RunChange> {
        self.rerun_changed(svc, ids, None).await
    }

    /// Re-run `ids`; with `changed` (datasets, measures), keeping the loaders
    /// the change didn't reach.
    async fn rerun_changed(
        &self,
        svc: &crate::Services,
        ids: Vec<String>,
        changed: Option<(&[String], &[String])>,
    ) -> Vec<RunChange> {
        let mut out = Vec::new();
        for id in ids {
            let started = std::time::Instant::now();
            let outcome = self.rerun_with(svc, &id, BTreeMap::new(), changed).await;
            let took = started.elapsed();
            if took >= SLOW_RERUN {
                let loaders = match &outcome {
                    Ok((_, change)) => change.loaders.join(","),
                    Err(_) => String::new(),
                };
                tracing::warn!(
                    run = %id,
                    page = %self.get(&id).map(|r| r.page.id.clone()).unwrap_or_default(),
                    %loaders,
                    took_ms = took.as_millis() as u64,
                    "slow page re-run"
                );
            }
            match outcome {
                Ok((_, change)) if !change.loaders.is_empty() || change.tree => out.push(change),
                Ok(_) => {}
                // Its page went (a deploy removed it): it stays as it was;
                // the window's next fetch says why.
                Err(error) => tracing::debug!(%error, run = %id, "an open page run didn't re-run"),
            }
        }
        out
    }
}

/// Whether a loader of `run` (or of a page embedded in it) read one of
/// `datasets`: a query's reads, a command loader's declared `reads`.
fn reads_any(run: &PageRun, datasets: &[String], measures: &[String]) -> bool {
    let own = run.page.loaders.iter().any(|decl| {
        decl.reads.iter().any(|m| datasets.contains(m))
            || decl.reads_params.iter().any(|p| {
                super::runner::context_reads(&run.page, p)
                    .iter()
                    .any(|m| datasets.iter().any(|x| x == m))
            })
            || run.loaders.get(&decl.name).is_some_and(|r| {
                r.result.reads.datasets.iter().any(|m| datasets.contains(m))
                    || r.result.reads.measures.iter().any(|m| measures.contains(m))
            })
    });
    own || embeds(&run.tree).any(|e| reads_any(e, datasets, measures))
        || run
            .rail
            .as_ref()
            .is_some_and(|r| embeds(r).any(|e| reads_any(e, datasets, measures)))
}

fn embeds(node: &RunNode) -> Box<dyn Iterator<Item = &PageRun> + '_> {
    Box::new(
        node.embed
            .as_deref()
            .into_iter()
            .chain(node.children.iter().flat_map(embeds)),
    )
}

/// The loaders whose result or error differs between `old` and `new`
/// (embedded pages' as `<embed id>/<loader>`), and whether either root's
/// tree or embedded page changed.
fn diff(old: &PageRun, new: &PageRun) -> RunChange {
    let mut names: Vec<&str> = new
        .page
        .loaders
        .iter()
        .map(|decl| decl.name.as_str())
        .collect();
    for decl in &old.page.loaders {
        if !names.contains(&decl.name.as_str()) {
            names.push(&decl.name);
        }
    }
    let mut loaders: Vec<String> = names
        .into_iter()
        .filter(|name| {
            let (a, b) = (old.loaders.get(*name), new.loaders.get(*name));
            // Completeness and provenance can change while the retained
            // cells stay identical (for example a newly truncated result).
            a.map(|run| (&run.result, &run.error)) != b.map(|run| (&run.result, &run.error))
        })
        .map(str::to_string)
        .collect();
    let mut tree = shape(&old.tree) != shape(&new.tree)
        || old.rail.as_ref().map(shape) != new.rail.as_ref().map(shape)
        || old.page != new.page;
    let mut old_embeds = embed_runs(&old.tree, "");
    let mut new_embeds = embed_runs(&new.tree, "");
    // Rail nodes share the page's node-id namespace: their action/fetch
    // paths have no additional `rail/` segment.
    if let Some(rail) = &old.rail {
        old_embeds.extend(embed_runs(rail, ""));
    }
    if let Some(rail) = &new.rail {
        new_embeds.extend(embed_runs(rail, ""));
    }
    tree |= old_embeds
        .iter()
        .any(|(path, _)| !new_embeds.iter().any(|(new_path, _)| new_path == path));
    for (path, run) in &new_embeds {
        match old_embeds.iter().find(|(p, _)| p == path) {
            Some((_, before)) => {
                let inner = diff(before, run);
                loaders.extend(inner.loaders.into_iter().map(|l| format!("{path}/{l}")));
                tree |= inner.tree;
            }
            None => tree = true,
        }
    }
    RunChange {
        run_id: String::new(),
        seq: 0,
        loaders,
        tree,
    }
}

/// A tree's node ids and errors, without its embeds' runs.
fn shape(node: &RunNode) -> Vec<(String, Option<String>)> {
    let mut out = vec![(node.id.clone(), node.error.clone())];
    for c in &node.children {
        out.extend(shape(c));
    }
    out
}

fn embed_runs<'r>(node: &'r RunNode, prefix: &str) -> Vec<(String, &'r PageRun)> {
    let path = format!("{prefix}{}", node.id);
    let mut out = Vec::new();
    if let Some(e) = &node.embed {
        out.push((path.clone(), e.as_ref()));
    }
    for c in &node.children {
        out.extend(embed_runs(c, prefix));
    }
    out
}

#[cfg(test)]
mod diff_tests {
    use super::*;
    use crate::pages::runner::LoaderRun;

    fn fixture(slug: &str) -> PageRun {
        let page = crate::pages::schema::parse_page(
            "acme",
            "acme",
            slug,
            "diff-test.yaml",
            "title: Diff test\nloaders:\n  value: { rows: [{ n: 1 }] }\nview: { viz: table, from: value }\nrail: { id: rail-embed, embed: { page: acme/child } }\n",
        )
        .unwrap();
        let tree = RunNode {
            id: page.view.id.clone(),
            children: Vec::new(),
            embed: None,
            error: None,
            replaced_by: None,
        };
        PageRun {
            inactive: None,
            page,
            params: BTreeMap::new(),
            loaders: BTreeMap::from([(
                "value".into(),
                LoaderRun {
                    result: oxplow_db::SqlQueryResult {
                        columns: vec!["n".into()],
                        rows: vec![vec![SqlCell::Int(1)]],
                        ..Default::default()
                    },
                    error: None,
                    key: "value-key".into(),
                },
            )]),
            tree,
            rail: None,
            alert: None,
            badge: None,
        }
    }

    #[test]
    fn truncation_and_provenance_changes_notify_without_cell_changes() {
        let old = fixture("host");
        let changes: [fn(&mut oxplow_db::SqlQueryResult); 3] = [
            |result| result.truncated = true,
            |result| result.generation = Some("generation-b".into()),
            |result| result.reads.datasets.push("v_work_item".into()),
        ];
        for change in changes {
            let mut new = old.clone();
            change(&mut new.loaders.get_mut("value").unwrap().result);
            let changed = diff(&old, &new);
            assert_eq!(changed.loaders, ["value"]);
            assert!(!changed.tree);
        }
    }

    #[test]
    fn a_rail_embeds_changed_loader_uses_the_normal_node_path() {
        let mut old = fixture("host");
        old.rail = Some(RunNode {
            id: "rail-embed".into(),
            children: Vec::new(),
            embed: Some(Box::new(fixture("child"))),
            error: None,
            replaced_by: None,
        });
        let mut new = old.clone();
        new.rail
            .as_mut()
            .unwrap()
            .embed
            .as_mut()
            .unwrap()
            .loaders
            .get_mut("value")
            .unwrap()
            .result
            .rows[0][0] = SqlCell::Int(2);
        let changed = diff(&old, &new);
        assert_eq!(changed.loaders, ["rail-embed/value"]);
        assert!(!changed.tree);
    }

    #[test]
    fn removing_an_embed_or_rail_notifies_even_when_node_ids_are_unchanged() {
        let mut old = fixture("host");
        old.rail = Some(RunNode {
            id: "rail-embed".into(),
            children: Vec::new(),
            embed: Some(Box::new(fixture("child"))),
            error: None,
            replaced_by: None,
        });
        let mut new = old.clone();
        new.rail.as_mut().unwrap().embed = None;
        assert!(diff(&old, &new).tree);
        new.rail = None;
        assert!(diff(&old, &new).tree);
        // The same removal rule applies to the main tree's embeds.
        old.tree.embed = Some(Box::new(fixture("child")));
        new = old.clone();
        new.tree.embed = None;
        assert!(diff(&old, &new).tree);
    }

    #[test]
    fn removing_a_loader_names_its_missing_result() {
        let old = fixture("host");
        let mut new = old.clone();
        new.loaders.clear();
        new.page.loaders.clear();
        let changed = diff(&old, &new);
        assert_eq!(changed.loaders, ["value"]);
        assert!(changed.tree);
    }
}
