//! An ACP session's conversation, held in memory (a `session/load` replay
//! rebuilds it after a restart; there is no table).
//!
//! Each item has a stable `id` and a `seq` that is bumped every time the
//! item changes, from one counter for the whole transcript. A client that
//! has seen everything up to `seq` N asks for `since(N)` and upserts what
//! comes back by `id`: new items and changed ones (a streamed chunk
//! appended, a tool call updated, a permission answered) arrive the same
//! way. The ring is capped; the oldest items fall off.

use std::collections::VecDeque;

use serde::Serialize;

use super::model::{
    AcpUpdate, ContextUsage, PermissionAnswer, PermissionOption, PlanEntry, ToolCall,
};

pub const DEFAULT_CAP: usize = 2000;

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemBody {
    /// What the human sent. `context` is the oxplow block attached to it,
    /// shown behind a disclosure.
    User {
        text: String,
        context: Option<String>,
    },
    Agent {
        text: String,
    },
    Thought {
        text: String,
    },
    Tool {
        call: ToolCall,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    /// A permission request waiting on (or answered by) the human.
    Permission {
        request_id: String,
        tool_call_id: String,
        title: String,
        options: Vec<PermissionOption>,
        answer: Option<PermissionAnswer>,
    },
    /// oxplow's policy rejected the call without asking the human.
    PolicyDenied {
        tool_call_id: String,
        label: String,
        reason: String,
    },
    /// A write the policy would deny ran without asking first.
    Bypass {
        tool_call_id: String,
        label: String,
        reason: String,
    },
    /// The turn-end directive, shown to the human. Never sent.
    Directive {
        text: String,
    },
    /// Something failed: the prompt, the agent process, the protocol.
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptItem {
    pub id: u64,
    pub seq: u64,
    #[serde(flatten)]
    pub body: ItemBody,
}

#[derive(Debug)]
pub struct Transcript {
    items: VecDeque<TranscriptItem>,
    cap: usize,
    next_id: u64,
    seq: u64,
    usage: Option<ContextUsage>,
}

impl Default for Transcript {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

impl Transcript {
    pub fn new(cap: usize) -> Self {
        Self {
            items: VecDeque::new(),
            cap: cap.max(1),
            next_id: 1,
            seq: 0,
            usage: None,
        }
    }

    /// The highest `seq` handed out so far.
    pub fn head_seq(&self) -> u64 {
        self.seq
    }

    pub fn usage(&self) -> Option<&ContextUsage> {
        self.usage.as_ref()
    }

    /// Items created or changed after `seq`, oldest first.
    pub fn since(&self, seq: u64) -> Vec<TranscriptItem> {
        let mut out: Vec<TranscriptItem> =
            self.items.iter().filter(|i| i.seq > seq).cloned().collect();
        out.sort_by_key(|i| i.seq);
        out
    }

    pub fn push(&mut self, body: ItemBody) -> TranscriptItem {
        self.seq += 1;
        let item = TranscriptItem {
            id: self.next_id,
            seq: self.seq,
            body,
        };
        self.next_id += 1;
        self.items.push_back(item.clone());
        while self.items.len() > self.cap {
            self.items.pop_front();
        }
        item
    }

    /// Change item `id` in place; `None` when it has fallen off the ring.
    pub fn update(&mut self, id: u64, f: impl FnOnce(&mut ItemBody)) -> Option<TranscriptItem> {
        let item = self.items.iter_mut().find(|i| i.id == id)?;
        f(&mut item.body);
        self.seq += 1;
        item.seq = self.seq;
        Some(item.clone())
    }

    /// The current state of tool call `tool_call_id`.
    pub fn tool(&self, tool_call_id: &str) -> Option<&ToolCall> {
        self.items.iter().rev().find_map(|i| match &i.body {
            ItemBody::Tool { call } if call.id == tool_call_id => Some(call),
            _ => None,
        })
    }

    /// Fold one `session/update` in. Returns the item created or changed;
    /// `None` for updates that aren't items (usage, ignored kinds).
    pub fn apply(&mut self, u: AcpUpdate) -> Option<TranscriptItem> {
        match u {
            AcpUpdate::UserChunk(t) => Some(self.chunk(t, Chunk::User)),
            AcpUpdate::AgentChunk(t) => Some(self.chunk(t, Chunk::Agent)),
            AcpUpdate::ThoughtChunk(t) => Some(self.chunk(t, Chunk::Thought)),
            AcpUpdate::ToolCall(call) => match self.tool_item_id(&call.id) {
                Some(id) => self.update(id, |b| *b = ItemBody::Tool { call }),
                None => Some(self.push(ItemBody::Tool { call })),
            },
            AcpUpdate::ToolCallUpdate(p) => match self.tool_item_id(&p.id) {
                Some(id) => self.update(id, |b| {
                    if let ItemBody::Tool { call } = b {
                        call.apply(&p);
                    }
                }),
                None => Some(self.push(ItemBody::Tool {
                    call: ToolCall::from_patch(&p),
                })),
            },
            AcpUpdate::Plan(entries) => match self.current_plan_id() {
                Some(id) => self.update(id, |b| *b = ItemBody::Plan { entries }),
                None => Some(self.push(ItemBody::Plan { entries })),
            },
            AcpUpdate::Usage(u) => {
                self.usage = Some(u);
                None
            }
            AcpUpdate::Ignored => None,
        }
    }

    fn chunk(&mut self, text: String, kind: Chunk) -> TranscriptItem {
        let append = match (self.items.back().map(|i| &i.body), kind) {
            (Some(ItemBody::Agent { .. }), Chunk::Agent)
            | (Some(ItemBody::Thought { .. }), Chunk::Thought)
            | (Some(ItemBody::User { .. }), Chunk::User) => self.items.back().map(|i| i.id),
            _ => None,
        };
        let appended = append.and_then(|id| {
            self.update(id, |b| match b {
                ItemBody::Agent { text: t }
                | ItemBody::Thought { text: t }
                | ItemBody::User { text: t, .. } => t.push_str(&text),
                _ => {}
            })
        });
        if let Some(item) = appended {
            return item;
        }
        self.push(match kind {
            Chunk::User => ItemBody::User {
                text,
                context: None,
            },
            Chunk::Agent => ItemBody::Agent { text },
            Chunk::Thought => ItemBody::Thought { text },
        })
    }

    fn tool_item_id(&self, tool_call_id: &str) -> Option<u64> {
        self.items.iter().rev().find_map(|i| match &i.body {
            ItemBody::Tool { call } if call.id == tool_call_id => Some(i.id),
            _ => None,
        })
    }

    /// The plan of the current turn: the latest plan after the latest
    /// user message. A new turn starts a new plan item.
    fn current_plan_id(&self) -> Option<u64> {
        for i in self.items.iter().rev() {
            match &i.body {
                ItemBody::Plan { .. } => return Some(i.id),
                ItemBody::User { .. } => return None,
                _ => {}
            }
        }
        None
    }
}

#[derive(Clone, Copy)]
enum Chunk {
    User,
    Agent,
    Thought,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::model::{PlanStatus, ToolCallPatch, ToolKind, ToolStatus};

    fn tool(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            title: "t".into(),
            kind: ToolKind::Edit,
            ..Default::default()
        }
    }

    #[test]
    fn chunks_coalesce_into_one_item_with_a_bumped_seq() {
        let mut t = Transcript::default();
        let a = t.apply(AcpUpdate::AgentChunk("hel".into())).unwrap();
        let b = t.apply(AcpUpdate::AgentChunk("lo".into())).unwrap();
        assert_eq!(a.id, b.id);
        assert!(b.seq > a.seq);
        assert_eq!(
            b.body,
            ItemBody::Agent {
                text: "hello".into()
            }
        );
        // A thought between breaks the run.
        t.apply(AcpUpdate::ThoughtChunk("hm".into()));
        let c = t.apply(AcpUpdate::AgentChunk("again".into())).unwrap();
        assert_ne!(c.id, a.id);
        assert_eq!(t.since(0).len(), 3);
    }

    #[test]
    fn since_returns_new_and_changed_items() {
        let mut t = Transcript::default();
        t.push(ItemBody::User {
            text: "go".into(),
            context: None,
        });
        let first = t.apply(AcpUpdate::AgentChunk("a".into())).unwrap();
        let mark = t.head_seq();
        assert!(t.since(mark).is_empty());
        t.apply(AcpUpdate::AgentChunk("b".into()));
        let changed = t.since(mark);
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].id, first.id);
        assert_eq!(changed[0].body, ItemBody::Agent { text: "ab".into() });
    }

    #[test]
    fn tool_updates_merge_by_id() {
        let mut t = Transcript::default();
        let first = t.apply(AcpUpdate::ToolCall(tool("c1"))).unwrap();
        t.apply(AcpUpdate::AgentChunk("x".into()));
        let merged = t
            .apply(AcpUpdate::ToolCallUpdate(ToolCallPatch {
                id: "c1".into(),
                status: Some(ToolStatus::Completed),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(merged.id, first.id);
        assert_eq!(t.tool("c1").unwrap().status, ToolStatus::Completed);
        assert_eq!(t.tool("c1").unwrap().title, "t");
        // An update for a call never announced creates it.
        let orphan = t
            .apply(AcpUpdate::ToolCallUpdate(ToolCallPatch {
                id: "c2".into(),
                title: Some("late".into()),
                ..Default::default()
            }))
            .unwrap();
        assert_ne!(orphan.id, first.id);
        assert_eq!(t.tool("c2").unwrap().title, "late");
    }

    #[test]
    fn plan_replaces_within_a_turn_and_restarts_after_a_user_message() {
        let mut t = Transcript::default();
        let entry = |s: PlanStatus| {
            vec![PlanEntry {
                content: "a".into(),
                status: s,
            }]
        };
        let p1 = t
            .apply(AcpUpdate::Plan(entry(PlanStatus::Pending)))
            .unwrap();
        let p2 = t
            .apply(AcpUpdate::Plan(entry(PlanStatus::Completed)))
            .unwrap();
        assert_eq!(p1.id, p2.id);
        t.push(ItemBody::User {
            text: "next".into(),
            context: None,
        });
        let p3 = t
            .apply(AcpUpdate::Plan(entry(PlanStatus::Pending)))
            .unwrap();
        assert_ne!(p3.id, p1.id);
    }

    #[test]
    fn usage_is_state_not_an_item() {
        let mut t = Transcript::default();
        let u = ContextUsage {
            used: 1,
            size: 10,
            cost_amount: None,
            cost_currency: None,
        };
        assert!(t.apply(AcpUpdate::Usage(u.clone())).is_none());
        assert_eq!(t.usage(), Some(&u));
        assert!(t.apply(AcpUpdate::Ignored).is_none());
        assert_eq!(t.head_seq(), 0);
    }

    #[test]
    fn ring_drops_the_oldest() {
        let mut t = Transcript::new(2);
        for s in ["a", "b", "c"] {
            t.push(ItemBody::Directive { text: s.into() });
        }
        let items = t.since(0);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].body, ItemBody::Directive { text: "b".into() });
        assert!(t.update(1, |_| {}).is_none());
    }

    #[test]
    fn permission_answer_updates_in_place() {
        let mut t = Transcript::default();
        let p = t.push(ItemBody::Permission {
            request_id: "r1".into(),
            tool_call_id: "c1".into(),
            title: "Write".into(),
            options: vec![],
            answer: None,
        });
        let answered = t
            .update(p.id, |b| {
                if let ItemBody::Permission { answer, .. } = b {
                    *answer = Some(PermissionAnswer::Cancelled);
                }
            })
            .unwrap();
        assert!(answered.seq > p.seq);
        assert!(matches!(
            answered.body,
            ItemBody::Permission {
                answer: Some(PermissionAnswer::Cancelled),
                ..
            }
        ));
    }

    #[test]
    fn items_serialize_flat_with_a_type_tag() {
        let mut t = Transcript::default();
        let item = t.push(ItemBody::Directive { text: "d".into() });
        assert_eq!(
            serde_json::to_value(item).unwrap(),
            serde_json::json!({"id": 1, "seq": 1, "type": "directive", "text": "d"})
        );
    }
}
