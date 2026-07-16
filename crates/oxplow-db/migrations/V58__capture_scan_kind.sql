-- V58: `scan_kind` on metric_capture (tsk71) — HOW a capture's scanned set is
-- determined, the discriminator the per-path fold branches on:
--
--   'delta'    (default) — a snapshot-backed tree scan over the snapshot's own
--               file rows; restates exactly the paths in that snapshot (the
--               incremental rescans the fs-watch event loop runs).
--   'full'     — a baseline scan over the RECONSTRUCTED tree as-of its snapshot
--               (`tree_at` semantics: latest file_snapshot row per path <= the
--               snapshot). Restates every path in the tree, so it supersedes
--               all older facts — including files whose count dropped to 0 —
--               WITHOUT fabricating a full-tree snapshot. This is what lets
--               `rebuild_metrics` baseline against the latest existing snapshot
--               instead of force-capturing one that pollutes effort
--               attribution.
--   'asserted' — the capture restates exactly the paths it emitted facts for
--               (agent-asserted `record_metric`, synthetic writes). A snapshot,
--               when present, is provenance only — never a scanned set.
--
-- Plain ADD COLUMN (no CHECK — mirrors V54's stance: another CHECK just
-- recreates the table-rebuild trap V52 documents; validity is enforced in Rust).
ALTER TABLE metric_capture ADD COLUMN scan_kind TEXT NOT NULL DEFAULT 'delta';

-- Backfill: every historical snapshot-less capture was, by the old fold's
-- definition, an assertion ("restates its own emitted paths").
UPDATE metric_capture SET scan_kind = 'asserted' WHERE snapshot_id IS NULL;
