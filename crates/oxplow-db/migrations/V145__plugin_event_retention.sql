-- P8.D5: an extension's own retention window for its event types'
-- namespace (`event_types.retention`), shorter than the plugin default
-- (payload 30 days, large content 14). Kept when the extension is
-- unloaded — its rows stay in the log, under the window it promised.
CREATE TABLE plugin_event_retention (
    namespace TEXT PRIMARY KEY,
    extension TEXT NOT NULL,
    payload_days INTEGER NOT NULL CHECK (payload_days >= 1),
    content_days INTEGER NOT NULL CHECK (content_days >= 1),
    updated_at TEXT NOT NULL
) STRICT;
