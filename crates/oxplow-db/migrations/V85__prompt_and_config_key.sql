-- What a person can ask (core's questions and each enabled extension's
-- `intent.prompts`) and what they can configure (project.yaml's keys),
-- restated whole with each extension generation and read as `v_prompt`
-- and `v_config_key`.
CREATE TABLE catalog_prompt (
    position INTEGER PRIMARY KEY,
    source_kind TEXT NOT NULL,
    source TEXT NOT NULL,
    prompt TEXT NOT NULL,
    about TEXT
) STRICT;

CREATE TABLE config_key (
    key TEXT PRIMARY KEY,
    doc TEXT NOT NULL,
    human_only INTEGER NOT NULL,
    schema_json TEXT NOT NULL
) STRICT;
