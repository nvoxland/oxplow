-- tsk14: idempotent fact ingestion. `record_facts` was plain INSERTs, so
-- re-ingesting the same report (a replayed PostToolUse hook, ingest_coverage /
-- ingest_analysis called twice) double-inserted the capture + its facts and
-- double-counted additive reads. Give a capture an optional CONTENT IDENTITY —
-- `idempotency_key` = hash(producer + basis + verbatim payload) — and skip the
-- write when one already exists.
--
-- Nullable: per-run captures with no natural identity (code gauges, tokens,
-- lifecycle, …) keep inserting fresh rows every run. The partial unique index
-- constrains only keyed captures, so existing NULL-key rows never collide.
ALTER TABLE metric_capture ADD COLUMN idempotency_key TEXT;
CREATE UNIQUE INDEX idx_metric_capture_idempotency
    ON metric_capture(idempotency_key) WHERE idempotency_key IS NOT NULL;
