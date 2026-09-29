SELECT id, kind, title, branch, worktree_path, created_at, updated_at, archived_at
FROM source('streams')
