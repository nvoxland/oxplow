-- What site search indexes for each knowledge page, in
-- `v_oxplow_search_knowledge`'s shape, over the shared knowledge model.
SELECT CAST(k.uri AS TEXT) AS uri,
       CAST(k.title AS TEXT) AS title,
       CAST(coalesce(k.body, '') AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id,
       CAST(k.updated_at AS TEXT) AS version
FROM ref('knowledge_record') k
