SELECT change_id, path, start_line, end_line, lines, peer_path, peer_start_line, peer_end_line
FROM source('change_duplicate')
