SELECT change_id, path, head_start, head_end, base_start, base_end, head_after
FROM source('change_hunk')
