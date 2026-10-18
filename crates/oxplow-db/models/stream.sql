SELECT id, kind, title, branch, worktree_path, created_at, updated_at, archived_at,
       CAST('stream:str' || id AS TEXT) AS ref,
       custom_prompt AS prompt
FROM source('streams')
