//! Whether a commit lands an open effort's work (`.context/work-tracking.md`
//! rule 3). On `vcs.commit.indexed`, each open effort on the commit's
//! stream is compared with it, by content: the effort's files — its
//! recorded ones and what its thread's running turn has changed so far
//! (a commit mid-turn comes before that turn's files are observed) — as
//! they are in the worktree now, against the commit's version of each.
//!
//! The commit holds the effort when it touches at least one of those files
//! and has every one it touches as the effort left it; it holds all of it
//! (`complete`) when that's true of every file. Either way it logs
//! `effort.landed` and links a linked effort's item to the commit; an
//! effort policy decides what a landing means.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_domain::events::schema::{
    EffortLanded, EffortLandedV1, EventType as _, VcsCommitIndexed, VcsCommitIndexedV1,
};
use oxplow_domain::refs::build::{effort_ref, system_source};
use oxplow_domain::stores::AgentTurnStore as _;
use oxplow_domain::vcs::Revision;
use oxplow_domain::{DomainError, Envelope, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::Services;

/// The consumer's name (its checkpoint key; what callers settle on).
pub const NAME: &str = "effort.landing";

pub struct EffortLandingConsumer {
    services: Weak<Services>,
}

/// Register the consumer on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(EffortLandingConsumer {
            services: Arc::downgrade(svc),
        }));
}

/// An effort's files now: what's recorded, and what its thread's running
/// turn has changed since it began.
async fn files_now(
    svc: &Services,
    ws: &Path,
    effort: &oxplow_db::Effort,
) -> Result<BTreeSet<String>, DomainError> {
    let mut files: BTreeSet<String> = svc
        .effort_store
        .paths(&effort.id)
        .await?
        .into_iter()
        .collect();
    let running = svc.agent_turn_store.list_open(&effort.thread_id).await?;
    if let Some(start) = running.first().and_then(|t| t.start_snapshot_id) {
        for entry in svc
            .trees
            .diff(ws, Some(&Revision::Snapshot(start)), &Revision::Working)
            .await?
        {
            files.insert(entry.path);
        }
    }
    Ok(files)
}

/// Whether `sha` holds `files` as they are in `ws` now: `None` when it
/// touches none of them, else whether it has all of them so.
async fn landing(
    svc: &Services,
    ws: &Path,
    sha: &str,
    files: &BTreeSet<String>,
) -> Result<Option<bool>, DomainError> {
    let touched: BTreeSet<String> = svc.git_store.commit_paths(sha).await?.into_iter().collect();
    if files.is_disjoint(&touched) {
        return Ok(None);
    }
    let objects = svc.vcs.object_store(ws);
    let mut all = true;
    for path in files {
        let committed = svc
            .vcs
            .object_at(ws, sha, path)
            .await
            .map_err(|e| DomainError::Invalid(format!("{sha}:{path}: {e}")))?;
        let now = match std::fs::read(ws.join(path)) {
            Ok(bytes) => Some(objects.id_of(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(DomainError::Storage(format!("{path}: {e}"))),
        };
        if committed != now {
            if touched.contains(path) {
                // It touched this file, but not as the effort has it.
                return Ok(None);
            }
            all = false;
        }
    }
    Ok(Some(all))
}

#[async_trait]
impl AsyncEventConsumer for EffortLandingConsumer {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == VcsCommitIndexed::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        let indexed: VcsCommitIndexedV1 = serde_json::from_value(event.envelope.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("vcs.commit.indexed: {e}")))?;
        let Some(stream) = event.envelope.anchors.stream_id else {
            return Ok(());
        };
        let Some(sha) = indexed.commit.strip_prefix("commit:") else {
            return Ok(());
        };
        let ws = svc.worktrees.resolve(Some(&stream.to_string())).await;
        for effort in svc.effort_store.list_open_for_stream(stream).await? {
            let files = files_now(&svc, &ws, &effort).await?;
            let Some(complete) = landing(&svc, &ws, sha, &files).await? else {
                continue;
            };
            crate::commit_links::link(&svc, sha, &effort).await?;
            let env = Envelope::typed::<EffortLanded>(
                system_source(NAME),
                &EffortLandedV1 {
                    effort: effort_ref(effort.id),
                    commit: indexed.commit.clone(),
                    complete,
                },
            )
            .with_anchors(oxplow_domain::Anchors {
                stream_id: Some(stream),
                thread_id: Some(effort.thread_id),
                effort_id: Some(effort.id),
                ..Default::default()
            })
            .with_subject([effort_ref(effort.id), indexed.commit.clone()])
            .with_cause(event.envelope.id.clone())
            .with_dedupe_key(format!("effort.landed:{}:{sha}", effort.id));
            match svc.event_log_store.append(env).await {
                Ok(_) | Err(DomainError::Constraint(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{commit_all, EffortFixture};
    use oxplow_db::EffortFileChange;
    use oxplow_db::EffortStore as _;

    async fn claim(f: &EffortFixture, path: &str) {
        f.svc
            .effort_store
            .record_file(
                &f.effort,
                path,
                EffortFileChange::Updated,
                oxplow_db::effort_store::FileRefVersion {
                    local_snapshot_id: 0,
                    closest_vcs_rev: None,
                    vcs_rev_exact: false,
                },
            )
            .await
            .unwrap();
    }

    /// The fixture with a baseline snapshot, its repo indexed, and this
    /// consumer on its pump (boot registers it).
    async fn fixture() -> EffortFixture {
        let f = crate::thread_checkpoint::tests::with_baseline().await;
        register(&f.svc);
        index(&f).await;
        f
    }

    /// Index the repo's new commits and let the landing, the policy and
    /// the close's end snapshot run, as the app does within moments.
    async fn index(f: &EffortFixture) {
        crate::commit_indexer::refresh(&f.svc).await;
        for consumer in [
            NAME,
            crate::effort_policy::NAME,
            crate::effort_lifecycle::NAME,
        ] {
            f.svc
                .event_pump
                .settle(&[consumer], std::time::Duration::from_secs(10))
                .await;
        }
    }

    async fn landed(f: &EffortFixture) -> Vec<(String, bool)> {
        f.svc
            .event_log_store
            .read_after(0, 10_000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == EffortLanded::TYPE)
            .map(|e| {
                (
                    e.envelope.payload["effort"].as_str().unwrap().to_string(),
                    e.envelope.payload["complete"].as_bool().unwrap(),
                )
            })
            .collect()
    }

    async fn closed_by(f: &EffortFixture) -> Option<String> {
        f.svc
            .effort_store
            .get_effort(&f.effort)
            .await
            .unwrap()
            .unwrap()
            .closed_by
    }

    /// A commit of every file the effort changed, as it left them, lands
    /// it whole, and the default policy closes it as a commit.
    #[tokio::test]
    async fn a_commit_of_all_its_files_lands_it_and_closes_it() {
        let f = fixture().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("mine.rs"), "fn mine() {}\n").unwrap();
        claim(&f, "mine.rs").await;
        commit_all(&root, "mine");
        index(&f).await;
        assert_eq!(landed(&f).await, vec![(effort_ref(f.effort), true)]);
        assert_eq!(closed_by(&f).await.as_deref(), Some("commit"));
    }

    /// A commit of some of its files lands it in part: linked, still open.
    /// One that has a file of it but not as the effort has it holds none.
    #[tokio::test]
    async fn a_partial_commit_leaves_it_open() {
        let f = fixture().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        commit_all(&root, "a");
        std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
        claim(&f, "a.rs").await;
        claim(&f, "b.rs").await;
        index(&f).await;
        assert_eq!(landed(&f).await, vec![(effort_ref(f.effort), false)]);
        assert_eq!(closed_by(&f).await, None);

        std::fs::write(root.join("b.rs"), "fn b() { 1 }\n").unwrap();
        let repo = git2::Repository::open(&root).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("b.rs")).unwrap();
        idx.write().unwrap();
        std::fs::write(root.join("b.rs"), "fn b() { 2 }\n").unwrap();
        crate::test_fixtures::commit_index(&root, "stale b");
        index(&f).await;
        assert_eq!(
            landed(&f).await.len(),
            1,
            "a commit of an older b holds nothing"
        );
    }

    /// A commit made mid-turn holds what the running turn changed though
    /// it isn't observed yet.
    #[tokio::test]
    async fn a_mid_turn_commit_counts_the_running_turns_files() {
        let f = fixture().await;
        f.svc
            .hook_ingest
            .ingest(crate::hook_ingest::HookEnvelope {
                kind: oxplow_domain::hook::HookKind::UserPromptSubmit,
                thread_id: Some(f.thread),
                stream_id: None,
                agent_session_id: None,
                session_id: Some("s".into()),
                payload_json: "{}".into(),
                prompt: Some("go".into()),
                decision: None,
            })
            .await
            .unwrap();
        let root = f.svc.layout.project_dir.clone();
        std::fs::write(root.join("shell.rs"), "fn shell() {}\n").unwrap();
        commit_all(&root, "shell");
        index(&f).await;
        assert_eq!(landed(&f).await, vec![(effort_ref(f.effort), true)]);
    }

    /// A commit, then more edits in the same turn: the commit closes the
    /// first effort, and the effort the turn's end opens holds only what
    /// came after it.
    #[tokio::test]
    async fn a_commit_then_more_edits_splits_into_two_efforts() {
        let f = fixture().await;
        let root = f.svc.layout.project_dir.clone();
        let submit = |kind| crate::hook_ingest::HookEnvelope {
            kind,
            thread_id: Some(f.thread),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("s".into()),
            payload_json: "{}".into(),
            prompt: Some("go".into()),
            decision: None,
        };
        f.svc
            .hook_ingest
            .ingest(submit(oxplow_domain::hook::HookKind::UserPromptSubmit))
            .await
            .unwrap();
        let turn = f.svc.agent_turn_store.list_open(&f.thread).await.unwrap()[0].id;
        f.svc
            .tool_call_store
            .record(oxplow_db::NewToolCall {
                thread_id: f.thread.value(),
                turn_id: Some(turn.value()),
                tool: "Bash".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        let stream = f.svc.streams.list_streams().await.unwrap()[0].id;
        // What the file watcher does in the app: the capture sees a write.
        let watched = || async {
            f.svc
                .snapshot_captures
                .get(&stream)
                .unwrap()
                .enqueue_startup_diff()
                .await
                .unwrap();
        };
        std::fs::write(root.join("mine.rs"), "fn mine() {}\n").unwrap();
        watched().await;
        commit_all(&root, "mine");
        index(&f).await;
        assert_eq!(closed_by(&f).await.as_deref(), Some("commit"));
        std::fs::write(root.join("more.rs"), "fn more() {}\n").unwrap();
        watched().await;
        f.svc
            .hook_ingest
            .ingest(submit(oxplow_domain::hook::HookKind::Stop))
            .await
            .unwrap();
        for consumer in [
            crate::thread_checkpoint::NAME,
            crate::effort_policy::NAME,
            crate::effort_lifecycle::NAME,
            crate::effort_observation::NAME,
        ] {
            f.svc
                .event_pump
                .settle(&[consumer], std::time::Duration::from_secs(10))
                .await;
        }
        let next = f
            .svc
            .effort_store
            .find_open_for_thread(&f.thread)
            .await
            .unwrap()
            .expect("the turn's end opened the next effort");
        let files: Vec<String> = f
            .svc
            .effort_store
            .list_files(&next.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect();
        assert_eq!(files, vec!["more.rs".to_string()]);
    }
}
