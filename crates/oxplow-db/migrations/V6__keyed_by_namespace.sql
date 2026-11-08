-- A contribution is named in its extension's namespace (everything oxplow
-- ships is `oxplow`), so its health is too: it keeps its state when the
-- contribution moves to a sibling in the namespace.
ALTER TABLE contribution_health RENAME COLUMN extension TO namespace;
-- Settings and private state are the namespace's too: extensions sharing
-- one share them, and they outlive a move between those extensions.
ALTER TABLE extension_config RENAME COLUMN extension TO namespace;
ALTER TABLE extension_state RENAME COLUMN extension TO namespace;
-- What a contribution is named in, beside who declares it.
ALTER TABLE contribution ADD COLUMN namespace TEXT NOT NULL DEFAULT '';
