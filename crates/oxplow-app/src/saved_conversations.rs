//! Where the harnesses' saved conversations are read (`v_agent_conversation`):
//! the machine's, each harness asked, or a stand-in holding only what it was
//! given. A throwaway oxplow (`oxplow extension test`) uses the stand-in, as
//! it uses a memory keychain: the machine's conversations, and its `codex` and
//! `opencode`, are not the test's data.

use std::collections::HashMap;
use std::sync::RwLock;

use oxplow_domain::agent::harness::{AgentHarness, HarnessError, NativeConversation};
use oxplow_domain::Stream;

/// What one harness lists in the stand-in: its conversations, or why it can't.
type Given = Result<Vec<NativeConversation>, String>;

#[derive(Default)]
pub struct SavedConversations {
    /// `None`: the machine's. `Some`: the stand-in, by harness key.
    stand_in: RwLock<Option<HashMap<String, Given>>>,
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

    /// The saved conversations of `harness` begun in `stream`'s worktree.
    pub async fn list(
        &self,
        harness: &dyn AgentHarness,
        stream: &Stream,
    ) -> Result<Vec<NativeConversation>, HarnessError> {
        let given = self
            .stand_in
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|held| held.get(harness.id()).cloned());
        match given {
            None => crate::live_datasets::harness_conversations(harness, stream).await,
            Some(None) => Ok(Vec::new()),
            Some(Some(given)) => given.map_err(HarnessError::Unavailable),
        }
    }
}
