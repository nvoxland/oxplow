-- What site search indexes for each commit `commit` holds (the last 500
-- reachable from every stream's head): the subject, and in the body the
-- message body, the author and the paths it changed (up to 300). Commits
-- belong to the project, not a stream. A commit never changes, so `version`
-- is its sha.
SELECT CAST(c.uri AS TEXT) AS uri,
       CAST(c.subject AS TEXT) AS title,
       CAST(coalesce(c.body, '') || char(10) || c.author
            || coalesce((SELECT group_concat(char(10) || f.path, '')
                           FROM (SELECT path FROM ref('commit_file')
                                  WHERE sha = c.sha ORDER BY path LIMIT 300) f), '')
            AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id,
       CAST(c.sha AS TEXT) AS version
FROM ref('commit') c
