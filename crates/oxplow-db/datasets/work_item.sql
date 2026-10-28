-- The active work list's items only: the interface shows one list at a
-- time, whichever implementation it is (none: nothing).
SELECT w.uri, w.provider,
       CAST(uri_param(w.uri, 'id') AS TEXT) AS native_id,
       w.title, w.body, w.state, w.native_state, w.native,
       -- A deleted parent isn't a live work item: no parent then.
       (SELECT p.uri FROM source('work_item') p
         WHERE p.uri = w.parent_uri AND p.deleted_at IS NULL) AS parent_uri,
       w.thread_id, w.rank, w.closed_at, w.created_at, w.updated_at
FROM source('work_item') w
JOIN source('capability_provider') a
  ON a.capability = 'work_items' AND a.active = 1 AND a.provider = w.provider
WHERE w.deleted_at IS NULL
