//! The page runs windows have open: each run kept with the request that
//! made it, re-run when a model one of its loaders read changes (or its
//! params change), so a window hears which loaders' rows changed
//! (`OxplowEvent::PageRunChanged`) and fetches only those. The change loop
//! (`models_changed::spawn`) tells it what changed; nothing here listens to
//! the event bus.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use oxplow_db::SqlCell;
use oxplow_domain::DomainError;
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

/// What the change loop tells the open runs.
#[derive(Debug, Clone)]
enum Signal {
    Models(Vec<String>),
    Measures(Vec<String>),
    Everything,
}

struct OpenRun {
    req: RunRequest,
    run: PageRun,
    client: String,
    seq: u64,
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
}

/// How long signals gather before the open runs re-run: a burst of
/// commits is one re-run.
const COALESCE: std::time::Duration = std::time::Duration::from_millis(100);

impl PageRuns {
    /// Re-run open runs as the change loop says models change, and tell
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
                    let mut models = Vec::new();
                    let mut measures = Vec::new();
                    for s in signals {
                        match s {
                            Signal::Models(m) => models.extend(m),
                            Signal::Measures(m) => measures.extend(m),
                            Signal::Everything => {}
                        }
                    }
                    runs.changed(&svc, &models, &measures).await
                };
                for change in changes {
                    svc.events
                        .emit(crate::events::OxplowEvent::PageRunChanged(change));
                }
            }
        });
    }

    fn signal(&self, signal: Signal) {
        if let Some(tx) = self.signals.lock().as_ref() {
            let _ = tx.send(signal);
        }
    }

    /// The change loop: these published models changed.
    pub fn notify_models(&self, models: &[String]) {
        if !models.is_empty() {
            self.signal(Signal::Models(models.to_vec()));
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
        let run = PageRunner::of(svc).run(req.clone()).await?;
        let id = uuid::Uuid::now_v7().to_string();
        self.runs.lock().insert(
            id.clone(),
            OpenRun {
                req,
                run: run.clone(),
                client: client.to_string(),
                seq: 0,
            },
        );
        Ok((id, run))
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

    /// The open runs' ids.
    pub fn ids(&self) -> Vec<String> {
        self.runs.lock().keys().cloned().collect()
    }

    /// Drop every run whose window isn't among `clients`.
    pub fn keep_only_clients(&self, clients: &[String]) {
        self.runs.lock().retain(|_, r| clients.contains(&r.client));
    }

    /// Re-run `id` with `params` over its own (a pick, a filter, a switch's
    /// param).
    pub async fn set_params(
        &self,
        svc: &crate::Services,
        id: &str,
        params: BTreeMap<String, SqlCell>,
    ) -> Result<(PageRun, RunChange), DomainError> {
        let mut req = self.request(id)?;
        for (k, v) in params {
            req.params.insert(k, v);
        }
        self.rerun_with(svc, id, req).await
    }

    /// Re-run `id` as it is (Refresh).
    pub async fn rerun(
        &self,
        svc: &crate::Services,
        id: &str,
    ) -> Result<(PageRun, RunChange), DomainError> {
        let req = self.request(id)?;
        self.rerun_with(svc, id, req).await
    }

    fn request(&self, id: &str) -> Result<RunRequest, DomainError> {
        self.runs
            .lock()
            .get(id)
            .map(|r| r.req.clone())
            .ok_or_else(|| DomainError::Invalid(format!("no open page run `{id}`")))
    }

    async fn rerun_with(
        &self,
        svc: &crate::Services,
        id: &str,
        req: RunRequest,
    ) -> Result<(PageRun, RunChange), DomainError> {
        let fresh = PageRunner::of(svc).run(req.clone()).await?;
        let mut runs = self.runs.lock();
        let open = runs
            .get_mut(id)
            .ok_or_else(|| DomainError::Invalid(format!("no open page run `{id}`")))?;
        let mut change = diff(&open.run, &fresh);
        open.seq += 1;
        change.run_id = id.to_string();
        change.seq = open.seq;
        open.req = req;
        open.run = fresh.clone();
        Ok((fresh, change))
    }

    /// Re-run each open run one of whose loaders read one of `models`
    /// (embedded pages' included). What changed, per run that changed.
    pub async fn models_changed(&self, svc: &crate::Services, models: &[String]) -> Vec<RunChange> {
        self.changed(svc, models, &[]).await
    }

    /// Re-run each open run that read one of `models` or `measures`.
    pub async fn changed(
        &self,
        svc: &crate::Services,
        models: &[String],
        measures: &[String],
    ) -> Vec<RunChange> {
        let ids: Vec<String> = self
            .runs
            .lock()
            .iter()
            .filter(|(_, r)| reads_any(&r.run, models, measures))
            .map(|(id, _)| id.clone())
            .collect();
        self.rerun_all(svc, ids).await
    }

    /// Re-run every open run (the catalog changed, or the change loop
    /// missed something).
    pub async fn all_changed(&self, svc: &crate::Services) -> Vec<RunChange> {
        let ids = self.ids();
        self.rerun_all(svc, ids).await
    }

    async fn rerun_all(&self, svc: &crate::Services, ids: Vec<String>) -> Vec<RunChange> {
        let mut out = Vec::new();
        for id in ids {
            match self.rerun(svc, &id).await {
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
/// `models`: a query's reads, a command loader's declared `reads`.
fn reads_any(run: &PageRun, models: &[String], measures: &[String]) -> bool {
    let own = run.page.loaders.iter().any(|decl| {
        decl.reads.iter().any(|m| models.contains(m))
            || run.loaders.get(&decl.name).is_some_and(|r| {
                r.result.reads.models.iter().any(|m| models.contains(m))
                    || r.result.reads.measures.iter().any(|m| measures.contains(m))
            })
    });
    own || embeds(&run.tree).any(|e| reads_any(e, models, measures))
        || run
            .rail
            .as_ref()
            .is_some_and(|r| embeds(r).any(|e| reads_any(e, models, measures)))
}

fn embeds(node: &RunNode) -> Box<dyn Iterator<Item = &PageRun> + '_> {
    Box::new(
        node.embed
            .as_deref()
            .into_iter()
            .chain(node.children.iter().flat_map(embeds)),
    )
}

/// The loaders whose rows differ between `old` and `new` (embedded
/// pages' as `<embed id>/<loader>`), and whether the tree changed.
fn diff(old: &PageRun, new: &PageRun) -> RunChange {
    let mut loaders: Vec<String> = new
        .page
        .loaders
        .iter()
        .filter(|d| {
            let (a, b) = (old.loaders.get(&d.name), new.loaders.get(&d.name));
            a.map(|r| (&r.result.columns, &r.result.rows, &r.error))
                != b.map(|r| (&r.result.columns, &r.result.rows, &r.error))
        })
        .map(|d| d.name.clone())
        .collect();
    let mut tree = shape(&old.tree) != shape(&new.tree) || old.page != new.page;
    let (old_embeds, new_embeds) = (embed_runs(&old.tree, ""), embed_runs(&new.tree, ""));
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
