-- What a change's deep analysis was computed from — the build, its base and
-- head trees, an effort's own files — so a rerun over the same inputs keeps
-- what's stored instead of recomputing (change_analysis.rs `analyze`).
ALTER TABLE change ADD COLUMN analyzed_from TEXT;
