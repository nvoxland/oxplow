-- A code-quality scan names the revision it read the way everything else
-- does (P5.B2, `oxplow_domain::vcs::Revision`): `working`, `snap:<id>` or
-- `git:<rev>`, in one `revision` column replacing the V9
-- `tree_version_kind` / `tree_version_value` pair.
DROP INDEX IF EXISTS idx_code_quality_scan_version;
ALTER TABLE code_quality_scan ADD COLUMN revision TEXT NOT NULL DEFAULT 'working';
UPDATE code_quality_scan SET revision = CASE
    WHEN tree_version_kind = 'ref' AND tree_version_value IS NOT NULL
        THEN 'git:' || tree_version_value
    WHEN tree_version_kind = 'snapshot' AND tree_version_value IS NOT NULL
        THEN 'snap:' || tree_version_value
    ELSE 'working'
END;
ALTER TABLE code_quality_scan DROP COLUMN tree_version_kind;
ALTER TABLE code_quality_scan DROP COLUMN tree_version_value;
CREATE INDEX idx_code_quality_scan_revision
    ON code_quality_scan(tool, revision, file_filter);
