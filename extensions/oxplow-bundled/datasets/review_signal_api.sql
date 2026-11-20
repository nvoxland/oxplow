-- Public API the change altered: a public function whose parameter count
-- changed, or one it removed. Callers outside the change may break.
SELECT fn.change_id, 'oxplow/review_signal_api' AS source, 'code' AS area,
       CASE WHEN fn.status = 'deleted' THEN 'public-removed' ELSE 'signature-changed' END AS signal,
       'medium' AS severity, fn.path, fn.start_line AS line, NULL AS end_line,
       CASE WHEN fn.status = 'deleted' THEN 'Public function removed: `' ELSE 'Public signature changed: `' END
         || CASE WHEN fn.container <> '' THEN fn.container || '::' ELSE '' END || fn.name || '`' AS title,
       CASE WHEN fn.status = 'deleted' THEN 'Callers outside this change may break.'
            ELSE printf('Parameters %d → %d. Callers outside this change may break.',
                        fn.params_before, fn.params_after) END AS why,
       fn.uri
FROM ref('change_function') fn
WHERE fn.is_test = 0 AND fn.visibility = 'public'
  AND (fn.status = 'deleted' OR (fn.status = 'modified' AND fn.signature_changed = 1))
