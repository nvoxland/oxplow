SELECT ref, provider, title, body, state, native_state, native, parent_ref,
       created_at, updated_at
FROM source('work_item')
WHERE deleted_at IS NULL
