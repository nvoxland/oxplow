-- Each synced pull request as a URI oxplow can name.
SELECT CAST(uri('github/pr', 'id', pr.number) AS TEXT) AS ref,
       pr.title,
       CAST(coalesce(pr.body, '') AS TEXT) AS body
FROM ref('pr') pr
