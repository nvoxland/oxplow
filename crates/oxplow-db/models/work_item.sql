SELECT w.ref, w.provider, w.title, w.body, w.state, w.native_state, w.native,
       -- A deleted parent isn't a live work item: no parent then.
       (SELECT p.ref FROM source('work_item') p
         WHERE p.ref = w.parent_ref AND p.deleted_at IS NULL) AS parent_ref,
       w.created_at, w.updated_at
FROM source('work_item') w
WHERE w.deleted_at IS NULL
