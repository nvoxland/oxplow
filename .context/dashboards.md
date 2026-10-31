# Dashboards

A dashboard is a page. Read [extensions-pages.md](./extensions-pages.md) "Pages" for
the page format; this doc covers only what is specific to dashboards.

## Page-file dashboards

A dashboard is a page in an extension with `category: Dashboards` and a
`view: { grid: [...] }`. Tiles are nodes, including embeds of other pages;
the page file is the source of their order and settings. There is no
dashboard table, store or `dashboard.*` command: migration V113 dropped
them.

The foundation's `pages/dashboards.yaml` is the **Dashboards** index in
the launcher's **Data** section. It lists `v_page WHERE category =
'Dashboards'`, ordered by title and id, and links each row through its
stored page URI. The index is itself in Data so it never lists itself.
It includes dashboard pages from every enabled extension, rather than
only pages in the default personal extension.

**New dashboard…** is a whole-page form action on
`oxplow.extension.new_dashboard`. The index fixes `extension: my-pages`
and asks for the title; the command derives the page slug, creates an
empty grid, writes the page and deploys it. An existing slug is refused
rather than overwritten. The command's `ui.open_after` opens the returned
page URI. A dashboard with no tiles says how to add one (Pin to Dashboard…),
not a blank page (`DashboardGrid`). Like `extension.write_page`, authoring is refused for bundled
extensions and for an in-project source from a worktree stream.

Read or edit the YAML for an existing dashboard, then use
`oxplow.extension.write_page { extension, page, spec, deploy: true }` to
save and deploy it. The document text is preserved as supplied; comments
and key order need not be discarded. Visual editing uses the same path:
`pages/dashboardEdits.ts` edits YAML Document nodes, preserving comments,
scalar styles and key order. `DashboardGrid` previews drag order, restores
the server order on failure, and offers tile removal through a right-click
menu and an inline confirmation. `DashboardTitle` is the page's title in the nav bar
(the body has no heading) and saves an inline rename
on Enter; Escape cancels. Bundled or missing sources are read-only.

`page_spec { page }` reads the writable primary source immediately before
an edit, including changes that have not been deployed. Read-only pages
return their deployed YAML. This prevents an edit from rebuilding a page
from its rendered DTO and losing author annotations. The current writer
has no compare-and-swap against concurrent filesystem changes.

The nav bar's **Pin to Dashboard** lists writable dashboard pages and
adds an embed with the page's current params and a stable tile id. Its
**New dashboard…** path uses the remembered `pages.extension` preference,
defaulting to `my-pages`. An inline view is saved as a page before pinning.

## Agent authoring

An agent builds a dashboard on request ("make me a dashboard of the
coverage metrics") by writing a page: `oxplow.extension.new_dashboard`
for an empty one, then `oxplow.extension.write_page` with the grid. The
`oxplow-extension` skill's `dashboards` reference
(`crates/oxplow-agent-text/assets/skills/oxplow-extension/references/dashboards.md`)
teaches the choice between `oxplow_show_page`, a page, a dashboard, a
custom component and a snapshot; its YAML examples are parsed by
`pages/doc_examples_tests.rs`.
