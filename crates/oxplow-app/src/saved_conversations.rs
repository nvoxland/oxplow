//! Where the harnesses' saved conversations are read (`v_agent_conversation`):
//! the machine's, each harness asked, or a stand-in holding only what it was
//! given. A throwaway oxplow (`oxplow extension test`) uses the stand-in, as
//! it uses a memory keychain: the machine's conversations, and its `codex` and
//! `opencode`, are not the test's data.
//!
//! The machine's lists are kept a while: listing can start a program
//! (`opencode session list`, ~0.5 s) and every page showing them re-reads
//! them as turns land. A list is asked again when a conversation was bound
//! since (oxplow started, resumed or forked one: the `generation` the reader
//! passes, the newest binding) or after [`KEEP_FOR`] (one begun outside
//! oxplow).

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use oxplow_domain::agent::harness::{AgentHarness, HarnessError, NativeConversation};
use oxplow_domain::Stream;

/// What one harness lists in the stand-in: its conversations, or why it can't.
type Given = Result<Vec<NativeConversation>, String>;

/// How long a machine's list is kept while no conversation is bound.
const KEEP_FOR: Duration = Duration::from_secs(30);

/// A harness's list for a stream, as last read.
struct Kept {
    at: Instant,
    generation: i64,
    found: Vec<NativeConversation>,
}

pub struct SavedConversations {
    /// `None`: the machine's. `Some`: the stand-in, by harness key.
    stand_in: RwLock<Option<HashMap<String, Given>>>,
    /// The machine's lists, by harness key and stream.
    kept: std::sync::Mutex<HashMap<(String, oxplow_domain::StreamId), Kept>>,
    keep_for: RwLock<Duration>,
}

impl Default for SavedConversations {
    fn default() -> Self {
        Self {
            stand_in: RwLock::default(),
            kept: std::sync::Mutex::default(),
            keep_for: RwLock::new(KEEP_FOR),
        }
    }
}

impl SavedConversations {
    /// From now on, list only what [`Self::give`] and [`Self::unavailable`]
    /// say: a harness given nothing lists nothing.
    pub fn stand_in(&self) {
        let mut held = self.stand_in.write().unwrap_or_else(|e| e.into_inner());
        held.get_or_insert_with(HashMap::new);
    }

    /// `harness` lists `conversation` too (a stand-in from now on).
    pub fn give(&self, harness: &str, conversation: NativeConversation) {
        let mut held = self.stand_in.write().unwrap_or_else(|e| e.into_inner());
        let entry = held
            .get_or_insert_with(HashMap::new)
            .entry(harness.to_string())
            .or_insert_with(|| Ok(Vec::new()));
        match entry {
            Ok(found) => found.push(conversation),
            Err(_) => *entry = Ok(vec![conversation]),
        }
    }

    /// `harness` can't list, for `why` (a stand-in from now on).
    pub fn unavailable(&self, harness: &str, why: &str) {
        let mut held = self.stand_in.write().unwrap_or_else(|e| e.into_inner());
        held.get_or_insert_with(HashMap::new)
            .insert(harness.to_string(), Err(why.to_string()));
    }

    /// Keep a list this long from now on (a test's ageing).
    pub fn keep_for(&self, d: Duration) {
        *self.keep_for.write().unwrap_or_else(|e| e.into_inner()) = d;
    }

    /// The saved conversations of `harness` begun in `stream`'s worktree;
    /// `generation` is the newest conversation binding (a kept list read at
    /// another is asked again).
    pub async fn list(
        &self,
        harness: &dyn AgentHarness,
        stream: &Stream,
        generation: i64,
    ) -> Result<Vec<NativeConversation>, HarnessError> {
        let given = self
            .stand_in
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|held| held.get(harness.id()).cloned());
        match given {
            None => self.machines(harness, stream, generation).await,
            Some(None) => Ok(Vec::new()),
            Some(Some(given)) => given.map_err(HarnessError::Unavailable),
        }
    }

    /// The machine's list: kept, or asked of the harness and kept. A failure
    /// isn't kept, so the next read asks again.
    async fn machines(
        &self,
        harness: &dyn AgentHarness,
        stream: &Stream,
        generation: i64,
    ) -> Result<Vec<NativeConversation>, HarnessError> {
        let key = (harness.id().to_string(), stream.id);
        let keep_for = *self.keep_for.read().unwrap_or_else(|e| e.into_inner());
        {
            let kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(k) = kept.get(&key) {
                if k.generation == generation && k.at.elapsed() < keep_for {
                    return Ok(k.found.clone());
                }
            }
        }
        let found = crate::live_datasets::harness_conversations(harness, stream).await?;
        self.kept.lock().unwrap_or_else(|e| e.into_inner()).insert(
            key,
            Kept {
                at: Instant::now(),
                generation,
                found: found.clone(),
            },
        );
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::agent::harness::{ConversationQuery, Launch, LaunchInput};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A harness that counts how often it is asked to list.
    struct Counting(AtomicUsize);

    #[async_trait::async_trait]
    impl AgentHarness for Counting {
        fn id(&self) -> &str {
            "counting"
        }
        fn title(&self) -> &str {
            "Counting"
        }
        async fn launch(&self, _: &LaunchInput) -> Result<Launch, HarnessError> {
            Err(HarnessError::Config("a fake".into()))
        }
        async fn tool_use(
            &self,
            _: &serde_json::Value,
        ) -> Option<oxplow_domain::agent::tool::ToolUse> {
            None
        }
        async fn render(&self, _: &oxplow_domain::agent::observe::HookAnswer) -> serde_json::Value {
            serde_json::Value::Null
        }
        async fn conversations(
            &self,
            _: &ConversationQuery,
        ) -> Result<Vec<NativeConversation>, HarnessError> {
            let n = self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![NativeConversation {
                id: format!("c{n}"),
                title: None,
                first_prompt: None,
                started_at: None,
                updated_at: None,
                size_bytes: None,
                parent: None,
            }])
        }
    }

    fn stream() -> Stream {
        let at = oxplow_domain::Timestamp::from_unix_ms(0);
        Stream {
            id: oxplow_domain::StreamId::new(1),
            kind: oxplow_domain::StreamKind::Primary,
            title: "s".into(),
            branch: "main".into(),
            branch_ref: String::new(),
            branch_source: String::new(),
            worktree_path: "/nowhere".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            host: oxplow_domain::HostId::LOCAL,
            custom_prompt: None,
            created_at: at,
            updated_at: at,
            archived_at: None,
        }
    }

    /// Listing can start a program (opencode) and every page that shows the
    /// conversations re-reads them as turns land: a list is kept while the
    /// bindings stay as they were and it is young, and asked again when a
    /// conversation is bound (oxplow started or resumed one) or it is old.
    #[tokio::test]
    async fn a_list_is_kept_until_a_binding_changes_or_it_ages() {
        let saved = SavedConversations::default();
        let harness = Counting(AtomicUsize::new(0));
        let s = stream();
        let ids = |l: Vec<NativeConversation>| l.into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids(saved.list(&harness, &s, 7).await.unwrap()), ["c0"]);
        assert_eq!(
            ids(saved.list(&harness, &s, 7).await.unwrap()),
            ["c0"],
            "kept"
        );
        assert_eq!(
            ids(saved.list(&harness, &s, 8).await.unwrap()),
            ["c1"],
            "a binding changed"
        );
        saved.keep_for(std::time::Duration::ZERO);
        assert_eq!(
            ids(saved.list(&harness, &s, 8).await.unwrap()),
            ["c2"],
            "aged"
        );
    }
}
