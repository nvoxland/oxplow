-- What a person's oxplow remembers for them outside any one tab: a page's
-- memory (by the page's ref, every tab of it; `kind:<kind>` for an entity
-- kind's pages), a panel's (its ref), a custom component's own
-- (`component:<ext>/<id>`) and the app's preferences (`app`). Local to this project's database, never the repo,
-- like `panel_layout`. A value is JSON text.
CREATE TABLE ui_preference (
    scope TEXT NOT NULL,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (scope, key)
) STRICT;
