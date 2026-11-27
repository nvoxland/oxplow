-- The host's own state (`host.db` in OXPLOW_HOME). Only the running host
-- opens it (it holds the file's lock for its whole life), or the CLI when
-- no host runs.

-- Paired devices: only a hash of each token, never the token.
CREATE TABLE device (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    paired_at TEXT NOT NULL,
    token_sha256 TEXT NOT NULL UNIQUE
);

-- The projects the host can start a daemon for, in the order they were added.
CREATE TABLE project (
    id TEXT PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL
);

-- A headless host's holds (the app's host keeps its own in memory). A hold
-- may name a device that never paired (the app's own) or a project since
-- removed, so neither is a foreign key.
CREATE TABLE hold (
    device TEXT NOT NULL,
    project TEXT NOT NULL,
    PRIMARY KEY (device, project)
);
