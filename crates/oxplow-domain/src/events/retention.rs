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
];

/// A plugin namespace's window, unless its extension declares a shorter
/// one (`event_types.retention`, P8.D5).
pub const PLUGIN_DEFAULT: RetentionWindow = RetentionWindow::new(30, 14);

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
