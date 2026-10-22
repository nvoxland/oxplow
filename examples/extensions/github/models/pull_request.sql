-- Each synced pull request as a URI oxplow can name.
SELECT CAST(uri('github/pr', 'id', pr.number) AS TEXT) AS uri,
       pr.title,
       CAST(coalesce(pr.body, '') AS TEXT) AS body
FROM ref('pr') pr
