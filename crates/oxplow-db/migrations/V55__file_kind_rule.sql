-- What kind of file a path is, as the enabled extensions (`file_kinds:`)
-- and the project (`oxplow.files.kinds`) declare it: one row per glob.
-- Restated whole by its source: the extensions' rows with each published
-- generation, the project's (`extension` = 'project-settings') when its
-- setting changes.
CREATE TABLE file_kind_rule_decl (
    kind TEXT NOT NULL,
    glob TEXT NOT NULL,
    label TEXT,
    extension TEXT NOT NULL,
    PRIMARY KEY (kind, glob, extension)
) WITHOUT ROWID;
