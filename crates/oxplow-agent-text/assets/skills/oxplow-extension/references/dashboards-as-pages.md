# Dashboards as pages

A dashboard is a page whose `view:` is a `grid` of tiles. Each tile is a
`viz` over a loader, or an `embed` of another page. It lives in a person's
extension (`my-pages` by default), so it can be kept, shared and tested
like any page. (The older dashboard store in `dashboards.md` still works
until it is migrated.)

```yaml
title: Work at a glance
params: []
answers:
  - { question: "How is work going?" }
data:
  open:   { sql: "SELECT count(*) AS n FROM v_work_item WHERE state != 'done'" }
  by_day: { sql: "SELECT date(created_at) AS day, count(*) AS n FROM v_work_item GROUP BY 1 ORDER BY 1" }
view:
  grid:
    - { id: open, title: Open, viz: number, from: open, size: small }
    - { id: trend, title: Created per day, viz: line, from: by_day, size: wide,
        chart: { x: day, y: n } }
    - { id: tests, embed: { page: oxplow-foundation/problems }, size: wide }
```

- A tile is a node with a `size`. Reorder tiles by reordering the list;
  rename by changing `title`; remove by deleting the node.
- Embed an existing page for a tile that is already a page. Its params
  flow by name from the dashboard's params and loaders.
- Every number is a query: read `reading-data.md` before you report one.

## Writing and changing one

Use `oxplow.extension.write_page`:

```
run_command oxplow.extension.write_page
  { extension: "my-pages", page: "work-glance", spec: "<the YAML text>", deploy: true }
```

- `spec` is the whole file as YAML text (comments and key order are kept).
  It must parse as a page and load with the extension, else nothing is
  written. The extension is created with a manifest when missing. An
  existing page is replaced, so to edit a dashboard, read its file, change
  the YAML, write it back.
- `deploy: true` deploys at once; without it the page waits for a deploy.
- Refused for a bundled extension, and from a worktree stream when the
  extension's source is in the project (write it from the main stream).
- It returns `{ page, path, deployed }`. Tell the person the title and
  that it is in the launcher (Cmd+P).
