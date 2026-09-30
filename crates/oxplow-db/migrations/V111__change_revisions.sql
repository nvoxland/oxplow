-- A change names its two sides as revisions (P5.B2,
-- `oxplow_domain::vcs::Revision`: `working`, `snap:<id>`, `git:<rev>`)
-- rather than display labels (`working tree`, `snapshot 12`, a bare sha),
-- so a lens link opens exactly the version it names.
ALTER TABLE change RENAME COLUMN base_label TO base_revision;
ALTER TABLE change RENAME COLUMN head_label TO head_revision;
UPDATE change SET base_revision = CASE
    WHEN base_revision IS NULL THEN NULL
    WHEN base_revision = 'working tree' THEN 'working'
    WHEN base_revision LIKE 'snapshot %' THEN 'snap:' || substr(base_revision, 10)
    ELSE 'git:' || base_revision
END;
UPDATE change SET head_revision = CASE
    WHEN head_revision IS NULL THEN NULL
    WHEN head_revision = 'working tree' THEN 'working'
    WHEN head_revision LIKE 'snapshot %' THEN 'snap:' || substr(head_revision, 10)
    ELSE 'git:' || head_revision
END;
