-- Run state of extension-declared sources (tsk291). The entity data
-- itself lives in ext__<extension>__<entity> tables that oxplow creates
-- at run time from the declared schema (a re-syncable cache), each with
-- a v_<extension>_<entity> view in the semantic layer.
CREATE TABLE ext_source_state (
    extension TEXT NOT NULL,
    source_id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'error')),
    last_run_at TEXT NOT NULL,
    error TEXT,
    -- {"<entity>": <row count>, …} from the last successful run.
    row_counts_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (extension, source_id)
);
