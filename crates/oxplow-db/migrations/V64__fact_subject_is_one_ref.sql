-- A fact's subject is stored as one ref (`file:src/a.rs`, `symbol:src/a.rs/f`,
-- `ai_model:claude-opus-4-8`), not split at the first colon into a kind and an
-- id that every reader had to put back together (`.context/refs.md` "Using
-- refs"). `kind` stays as a filter convenience, derived from the ref by the
-- store at insert; it is never an identity.
--
-- Existing rows become `kind || ':' || ref`, except where the old ref already
-- began with its kind (writers that stored a whole ref under its kind:
-- `file:src/a.rs` under `file`), which is kept as it is. Two old rows that
-- name one ref collapse into one subject. Rows with no kind or no id were
-- never refs (a collector subject without a colon, a kind with no id): they are
-- deleted and their facts keep no subject (their path and value stay).

UPDATE fact SET subject_id = NULL
 WHERE subject_id IN (SELECT id FROM fact_subject WHERE kind IS NULL OR ref IS NULL);
DELETE FROM fact_subject WHERE kind IS NULL OR ref IS NULL;

-- Each surviving old row's whole ref. Token facts were stored under kind
-- `model` with the model's name (once bare, later already `ai_model:<name>`);
-- they become `ai_model:<name>` refs, which is what names a language model
-- (`model:` names a published view). Keyed by the old id, so each fact
-- finds its row by index rather than scanning the map.
CREATE TEMP TABLE fact_subject_map (
    id INTEGER PRIMARY KEY,
    kind TEXT,
    ref TEXT
);
INSERT INTO fact_subject_map (id, kind, ref)
SELECT id,
       CASE WHEN kind = 'model' THEN 'ai_model'
            ELSE kind END AS kind,
       CASE WHEN kind = 'model' AND substr(ref, 1, 9) = 'ai_model:' THEN ref
            WHEN kind = 'model' THEN 'ai_model:' || ref
            WHEN substr(ref, 1, length(kind) + 1) = kind || ':' THEN ref
            ELSE kind || ':' || ref END AS ref
  FROM fact_subject;

CREATE TABLE fact_subject_new (
    id INTEGER PRIMARY KEY,
    ref TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL
);
INSERT INTO fact_subject_new (id, ref, kind)
SELECT MIN(id), ref, kind FROM fact_subject_map GROUP BY ref;

UPDATE fact SET subject_id = (
    SELECT n.id FROM fact_subject_map m JOIN fact_subject_new n ON n.ref = m.ref
     WHERE m.id = fact.subject_id)
 WHERE subject_id IS NOT NULL;
DROP TABLE fact_subject_map;

DROP TABLE fact_subject;
ALTER TABLE fact_subject_new RENAME TO fact_subject;

-- The cube folds by subject; it rebuilds from the facts.
DELETE FROM metric_live_fact;
DELETE FROM metric_cube;
DELETE FROM metric_cube_state;
UPDATE metric_cube_epoch SET epoch = epoch + 1;
