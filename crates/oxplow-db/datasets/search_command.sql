-- What site search indexes for each command an enabled extension declares
-- (`v_contribution`): titled by its label (its bus id when it has none)
-- and named by its label — a command with none (not run from the app) has
-- no name, so it ranks below what a person can run — and in the body its bus id, description, group, keywords and
-- the extension that declares it. They belong to the project, not a
-- stream.
SELECT CAST(c.uri AS TEXT) AS uri,
       CAST(coalesce(c.label, c.local) AS TEXT) AS title,
       CAST(c.label AS TEXT) AS name,
       CAST(trim(c.local || ' ' || coalesce(c.description, '') || ' '
                 || coalesce(c.keywords, '') || ' ' || c.extension) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('contribution') c
WHERE c.kind = 'command'
