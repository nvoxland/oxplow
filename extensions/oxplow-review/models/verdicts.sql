-- Every verdict a reviewer gave (`oxplow_review.verdict`), appended as
-- each lands.
SELECT e.seq,
       CAST(json_extract(e.payload, '$.effort') AS TEXT) AS ref,
       CAST(json_extract(e.payload, '$.verdict') AS TEXT) AS verdict,
       json_extract(e.payload, '$.forced') AS forced
FROM ref('event') e
WHERE e.type = 'oxplow_review.verdict'
