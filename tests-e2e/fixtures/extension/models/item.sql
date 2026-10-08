-- Each of oxplow's tasks as an `e2e_item:<n>` ref (`work_item:oxplow:tsk<n>`).
SELECT CAST('e2e_item:' || substr(w.ref, length('work_item:oxplow:tsk') + 1) AS TEXT) AS ref,
       CAST(w.title AS TEXT) AS title
FROM ref('work_item') w
WHERE w.provider = 'oxplow'
