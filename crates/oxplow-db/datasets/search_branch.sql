-- What site search indexes for each git branch `v_branch` lists: its name
-- (a remote-tracking branch as `origin/topic`, and only when no local
-- branch has the name), and in the body the commit it points at and its
-- upstream. Branches live in the repository every stream shares, so they
-- belong to the project, not a stream.
SELECT CAST(uri('oxplow/branch', 'name',
                CASE WHEN b.kind = 'remote' THEN b.remote || '/' || b.name ELSE b.name END) AS TEXT) AS uri,
       CAST(CASE WHEN b.kind = 'remote' THEN b.remote || '/' || b.name ELSE b.name END AS TEXT) AS title,
       CAST(trim(b.kind || ' branch ' || substr(coalesce(b.head_sha, ''), 1, 12)
                 || ' ' || coalesce(b.upstream, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('branch') b
WHERE b.kind = 'local'
   OR NOT EXISTS (SELECT 1 FROM ref('branch') l WHERE l.kind = 'local' AND l.name = b.name)
