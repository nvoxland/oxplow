-- The requests a commit's changes came from, per path: the requests on the
-- commit's stream that changed the path since the previous commit that held
-- it, where the file as the work left it is the commit's content.
SELECT CAST(cr.sha AS TEXT) AS sha,
       CAST(uri('oxplow/commit', 'sha', cr.sha) AS TEXT) AS commit_uri,
       CAST(cr.path AS TEXT) AS path,
       CAST(uri('oxplow/request', 'id', 'req' || cr.request_id) AS TEXT) AS request,
       CAST(cr.basis AS TEXT) AS basis,
       CAST(cr.confidence AS TEXT) AS confidence,
       CAST(CASE WHEN cr.turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || cr.turn_id) END AS TEXT) AS turn,
       CAST(cr.at AS TEXT) AS at
  FROM source('commit_request') cr
