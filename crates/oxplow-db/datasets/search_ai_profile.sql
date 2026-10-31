-- What site search indexes for each ai profile an enabled extension declares
-- (`v_contribution`): its id, and in the body its description and the
-- extension that declares it. They belong to the project, not a stream.
SELECT CAST(c.uri AS TEXT) AS uri,
       CAST(c.local AS TEXT) AS title,
       CAST(trim(coalesce(c.description, '') || ' ' || c.extension) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('contribution') c
WHERE c.kind = 'ai_profile'
