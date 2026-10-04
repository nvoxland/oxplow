-- Each synced pull request as a ref oxplow can name.
SELECT CAST('github_pr:' || pr.number AS TEXT) AS ref,
       pr.title,
       CAST(coalesce(pr.body, '') AS TEXT) AS body
FROM ref('pr') pr
