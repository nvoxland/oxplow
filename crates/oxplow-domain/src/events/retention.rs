//! How long the event log keeps its bodies, per namespace (§5.4). The
//! envelopes are kept; payloads and large content expire. Core's windows
//! are below; a plugin's namespace is kept at most [`PLUGIN_DEFAULT`], or
//! the shorter window its extension declares; a project may set its own
//! (`eventRetention` in `.oxplow/project.yaml`, a person's key) — for a
//! plugin namespace never longer than the plugin's. The sweep is
//! `oxplow_db::event_retention`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::schema::CORE_NAMESPACES;

/// How long a namespace's event payloads and large content are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetentionWindow {
    /// Days an event's payload is kept; after it, `{}`.
    pub payload_days: i64,
    /// Days its large content (tool input and output) is kept.
    pub content_days: i64,
}

impl RetentionWindow {
    pub const fn new(payload_days: i64, content_days: i64) -> Self {
        Self {
            payload_days,
            content_days,
        }
    }

    /// Each of the two at most `cap`'s.
    pub fn at_most(self, cap: Self) -> Self {
        Self {
            payload_days: self.payload_days.min(cap.payload_days),
            content_days: self.content_days.min(cap.content_days),
        }
    }
}

/// Core's expiring namespaces and their default windows. A core
/// namespace not listed is state (`snapshot`, `vcs`, `effort`, …): its
/// events are kept whole.
pub const CORE_WINDOWS: &[(&str, RetentionWindow)] = &[
    ("agent", RetentionWindow::new(30, 14)),
    ("test", RetentionWindow::new(90, 30)),
    ("code", RetentionWindow::new(90, 30)),
    ("collector", RetentionWindow::new(90, 30)),
    ("effect", RetentionWindow::new(90, 30)),
    ("ui", RetentionWindow::new(30, 14)),
    // A save is frequent and only read back by what reacts to it.
    ("file", RetentionWindow::new(30, 14)),
];

/// A plugin namespace's window, unless its extension declares a shorter
/// one (`event_types.retention`, P8.D5).
pub const PLUGIN_DEFAULT: RetentionWindow = RetentionWindow::new(30, 14);

/// The longest window a project may set: a hundred years (tsk985).
pub const MAX_DAYS: i64 = 36_500;

/// The shortest window a project may set for one of core's expiring
/// namespaces: oxplow reads them back (the agent policy reads a turn's
/// tool payloads, evidence reads a run's), so a week (tsk985). A plugin's
/// namespace may be kept for as little as a day.
pub const MIN_CORE_DAYS: i64 = 7;

/// What's wrong with a project's `window` for `namespace`, if anything:
/// core state (kept whole), a window under the floor or over
/// [`MAX_DAYS`], or a body kept longer than its payload.
pub fn window_problem(namespace: &str, window: RetentionWindow) -> Option<String> {
    if is_kept_whole(namespace) {
        return Some(format!(
            "`{namespace}` is core state: its events are kept whole"
        ));
    }
    let min = if core_window(namespace).is_some() {
        MIN_CORE_DAYS
    } else {
        1
    };
    let days = [window.payload_days, window.content_days];
    if days.iter().any(|d| *d < min) {
        return Some(if min == 1 {
            "windows are at least 1 day".to_string()
        } else {
            format!("windows of a core namespace are at least {min} days (oxplow reads them back)")
        });
    }
    if days.iter().any(|d| *d > MAX_DAYS) {
        return Some(format!(
            "windows are at most {MAX_DAYS} days (a hundred years)"
        ));
    }
    if window.content_days > window.payload_days {
        return Some(
            "contentDays can't be longer than payloadDays: an event's large content goes with its payload"
                .to_string(),
        );
    }
    None
}

/// Whether `namespace` is core state, whose events are kept whole — no
/// window applies to it.
pub fn is_kept_whole(namespace: &str) -> bool {
    CORE_NAMESPACES.contains(&namespace) && !CORE_WINDOWS.iter().any(|(ns, _)| *ns == namespace)
}

/// Core's default window for `namespace`, when it is one of core's
/// expiring namespaces.
pub fn core_window(namespace: &str) -> Option<RetentionWindow> {
    CORE_WINDOWS
        .iter()
        .find(|(ns, _)| *ns == namespace)
        .map(|(_, w)| *w)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// tsk1072: an operation the person saw fail is kept a month, its
    /// captured output two weeks, and a project can't keep it under the
    /// core floor.
    #[test]
    fn ui_op_errors_keep_30_days_and_their_output_14() {
        assert!(CORE_NAMESPACES.contains(&"ui"));
        assert_eq!(core_window("ui"), Some(RetentionWindow::new(30, 14)));
        assert!(!is_kept_whole("ui"));
        assert!(window_problem("ui", RetentionWindow::new(MIN_CORE_DAYS - 1, 1)).is_some());
    }
}
