-- Each oxplow task as an `e2e_item:<id>` ref.
SELECT CAST('e2e_item:' || t.id AS TEXT) AS ref,
       CAST(t.title AS TEXT) AS title
FROM ref('task') t
