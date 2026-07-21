-- tsk196 (from the tsk191 idle-CPU profile) — a cache-invalidation token for
-- the cube read path.
--
-- `cube_rows_for_measure` re-reads the whole `metric_cube JOIN metric_capture`
-- set on every call, parsing a timestamp per row. With N dashboard tiles
-- mounted, one `metricSamplesChanged` fires N identical full reads, at the
-- OTLP token cadence (~10s while any agent terminal is open). The fix is a
-- read cache; this is the token it keys on.
--
-- WHY NOT `epoch`. The obvious candidate is the column right next to this one,
-- and it is wrong. `write_cube_rows` READS `epoch` as an optimistic-concurrency
-- fence and abandons the fold when it moved (V66) — it never bumps it, and it
-- must not: if folds bumped the epoch, concurrent folds would abort each other,
-- which is the exact failure the fence exists to prevent. So `epoch` moves only
-- on WIPES. A cache keyed on it would never see an ordinary fold, and metrics
-- would silently freeze at their pre-fold values — a worse bug than the CPU
-- burn. Hence a second, independent counter: `epoch` fences writers, `version`
-- invalidates readers.
ALTER TABLE metric_cube_epoch ADD COLUMN version INTEGER NOT NULL DEFAULT 0;

-- WHY TRIGGERS rather than bumping this from Rust. Both FKs on `metric_cube`
-- are ON DELETE CASCADE (`measure_id`, `capture_id`), so archiving a stream or
-- dropping a measure deletes cube rows with NO Rust call site involved — a
-- hand-maintained counter would miss it and serve stale rows. On top of that
-- there are already two distinct fold paths (`write_cube_rows`,
-- `apply_build_batch`) and four wipe paths: six sites to keep in step forever,
-- against one invariant the database enforces for free. SQLite fires triggers
-- for FK cascade actions, so this covers the cascades too, and no write path
-- added later can violate it by forgetting.
--
-- Cost is one single-row UPDATE per cube row written. The cube is small (~2 MB
-- here) and every writer already runs inside a transaction, so this is noise
-- next to the full-table read it eliminates.
CREATE TRIGGER metric_cube_version_ai AFTER INSERT ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;

CREATE TRIGGER metric_cube_version_au AFTER UPDATE ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;

CREATE TRIGGER metric_cube_version_ad AFTER DELETE ON metric_cube BEGIN
    UPDATE metric_cube_epoch SET version = version + 1 WHERE id = 1;
END;
