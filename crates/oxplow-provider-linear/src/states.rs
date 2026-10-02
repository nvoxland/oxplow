//! A team's workflow states and how they map onto oxplow's canonical
//! states: by the state's type (`triage` / `backlog` / `unstarted` →
//! todo, `started` → in_progress, `completed` → done, `canceled` →
//! canceled), except the state the instance names as its blocked one
//! (`config.blocked_state`, default "Blocked"), which is blocked.

use oxplow_domain::work_items::CanonicalState;
use oxplow_provider_protocol::ProtocolError;
use serde_json::Value;

/// The blocked state's name when the config doesn't name one.
pub const DEFAULT_BLOCKED: &str = "Blocked";

/// One workflow state.
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    pub id: String,
    pub name: String,
    /// Linear's state type (`backlog`, `started`, …).
    pub kind: String,
    pub position: f64,
}

impl State {
    pub fn from_node(node: &Value) -> Option<State> {
        Some(State {
            id: node["id"].as_str()?.to_string(),
            name: node["name"].as_str()?.to_string(),
            kind: node["type"].as_str()?.to_string(),
            position: node["position"].as_f64().unwrap_or(0.0),
        })
    }
}

/// The canonical state of a Linear state type.
pub fn canonical_of_type(kind: &str) -> Option<CanonicalState> {
    match kind {
        "triage" | "backlog" | "unstarted" => Some(CanonicalState::Todo),
        "started" => Some(CanonicalState::InProgress),
        "completed" => Some(CanonicalState::Done),
        "canceled" => Some(CanonicalState::Canceled),
        _ => None,
    }
}

/// An instance's team: its id and key, its states, and which one is
/// blocked.
#[derive(Debug, Clone, PartialEq)]
pub struct Team {
    pub id: String,
    pub key: String,
    pub states: Vec<State>,
    pub blocked: String,
}

impl Team {
    /// The canonical state of a state named `name` of type `kind`.
    pub fn canonical(&self, name: &str, kind: &str) -> Option<CanonicalState> {
        if name.eq_ignore_ascii_case(&self.blocked) {
            return Some(CanonicalState::Blocked);
        }
        canonical_of_type(kind)
    }

    /// The state a move to `to` lands on: the one named `native` (which
    /// must be one of `to`'s), else `to`'s first state — for todo the
    /// first unstarted one, then backlog, then triage.
    pub fn state_for(
        &self,
        to: CanonicalState,
        native: Option<&str>,
    ) -> Result<&State, ProtocolError> {
        if let Some(name) = native {
            let state = self
                .states
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(name))
                .ok_or_else(|| ProtocolError::InvalidInput {
                    field: "/native_state".into(),
                    message: format!("team {} has no state `{name}`", self.key),
                })?;
            return match self.canonical(&state.name, &state.kind) {
                Some(c) if c == to => Ok(state),
                other => Err(ProtocolError::InvalidInput {
                    field: "/native_state".into(),
                    message: format!(
                        "`{}` is {}, not {}",
                        state.name,
                        other.map_or("unmapped", |c| c.as_str()),
                        to.as_str()
                    ),
                }),
            };
        }
        let kinds: &[&str] = match to {
            CanonicalState::Todo => &["unstarted", "backlog", "triage"],
            CanonicalState::InProgress => &["started"],
            CanonicalState::Done => &["completed"],
            CanonicalState::Canceled => &["canceled"],
            CanonicalState::Blocked => &[],
        };
        let candidates = |kind: &str| {
            let mut of: Vec<&State> = self
                .states
                .iter()
                .filter(|s| s.kind == kind && self.canonical(&s.name, &s.kind) == Some(to))
                .collect();
            of.sort_by(|a, b| a.position.total_cmp(&b.position));
            of.into_iter().next()
        };
        let found = match to {
            CanonicalState::Blocked => self
                .states
                .iter()
                .find(|s| s.name.eq_ignore_ascii_case(&self.blocked)),
            _ => kinds.iter().find_map(|k| candidates(k)),
        };
        found.ok_or_else(|| ProtocolError::InvalidInput {
            field: "/to".into(),
            message: format!("team {} has no {} state", self.key, to.as_str()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(id: &str, name: &str, kind: &str, position: f64) -> State {
        State {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            position,
        }
    }

    fn team() -> Team {
        Team {
            id: "t".into(),
            key: "ENG".into(),
            states: vec![
                state("s0", "Triage", "triage", 0.0),
                state("s1", "Backlog", "backlog", 1.0),
                state("s2", "Todo", "unstarted", 2.0),
                state("s3", "In Progress", "started", 3.0),
                state("s4", "Blocked", "started", 4.0),
                state("s5", "Done", "completed", 5.0),
                state("s6", "Canceled", "canceled", 6.0),
                state("s7", "Duplicate", "canceled", 7.0),
            ],
            blocked: DEFAULT_BLOCKED.into(),
        }
    }

    #[test]
    fn state_types_map_to_canonical_states_and_blocked_is_the_named_state() {
        let t = team();
        let got: Vec<(&str, Option<CanonicalState>)> = t
            .states
            .iter()
            .map(|s| (s.name.as_str(), t.canonical(&s.name, &s.kind)))
            .collect();
        use CanonicalState::*;
        assert_eq!(
            got,
            vec![
                ("Triage", Some(Todo)),
                ("Backlog", Some(Todo)),
                ("Todo", Some(Todo)),
                ("In Progress", Some(InProgress)),
                ("Blocked", Some(Blocked)),
                ("Done", Some(Done)),
                ("Canceled", Some(Canceled)),
                ("Duplicate", Some(Canceled)),
            ]
        );
    }

    #[test]
    fn a_move_lands_on_the_canonical_states_first_state_or_the_named_one() {
        let t = team();
        let land = |to, native| t.state_for(to, native).map(|s| s.name.clone());
        use CanonicalState::*;
        assert_eq!(land(Todo, None).unwrap(), "Todo");
        assert_eq!(land(InProgress, None).unwrap(), "In Progress");
        assert_eq!(land(Blocked, None).unwrap(), "Blocked");
        assert_eq!(land(Done, None).unwrap(), "Done");
        assert_eq!(land(Canceled, None).unwrap(), "Canceled");
        assert_eq!(land(Canceled, Some("duplicate")).unwrap(), "Duplicate");
        assert_eq!(land(Todo, Some("Backlog")).unwrap(), "Backlog");
        let wrong = t.state_for(Done, Some("Duplicate")).unwrap_err();
        assert!(
            matches!(&wrong, ProtocolError::InvalidInput { field, .. } if field == "/native_state"),
            "{wrong:?}"
        );
        let missing = Team {
            blocked: "Stuck".into(),
            ..team()
        };
        assert!(missing.state_for(Blocked, None).is_err());
        // Without its own state, "Blocked" is an ordinary started state.
        assert_eq!(missing.canonical("Blocked", "started"), Some(InProgress));
    }
}
