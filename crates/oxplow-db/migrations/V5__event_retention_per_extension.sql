-- Extensions share a namespace (everything oxplow ships is `oxplow`), so a
-- declared retention window is each extension's own: keyed by namespace
-- and extension, and the sweep keeps a namespace's events as long as its
-- longest window.
CREATE TABLE "extension_event_retention_new" (
    namespace TEXT NOT NULL,
    extension TEXT NOT NULL,
    payload_days INTEGER NOT NULL CHECK (payload_days >= 1),
    content_days INTEGER NOT NULL CHECK (content_days >= 1),
    updated_at TEXT NOT NULL,
    PRIMARY KEY (namespace, extension)
) STRICT;
INSERT INTO extension_event_retention_new
    SELECT namespace, extension, payload_days, content_days, updated_at
      FROM extension_event_retention;
DROP TABLE extension_event_retention;
ALTER TABLE extension_event_retention_new RENAME TO extension_event_retention;
