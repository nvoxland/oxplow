# Dashboards

A dashboard is your own arrangement of tiles: the handful of numbers,
charts and lists you actually want to watch, on one screen. The built-in
[Metrics](metrics.md) pages show every metric oxplow knows about.

A dashboard is a [page](pages.md) whose view is a grid, in the Dashboards
category. It lives as a file in one of your extensions (`my-pages` by
default), so it is yours to keep, edit by hand, commit and share like any
page.

## Making one

**New Dashboard…** from the launcher (++cmd+p++) creates an empty one, named
from the title you give it, and opens it. Then add tiles:

- **Pin to Dashboard** in any page's ☰ menu adds that page as a tile, with
  the params it has now. Pick the dashboard, or make a new one.
- A metric's detail page offers its own **Add to dashboard**.

## Tiles

A tile is another page, embedded, or a chart or table over a query of the
dashboard's own. Sizes are `small` (default), `wide`, `tall` and `full`.
Reorder, rename, resize and remove tiles in the app, or by editing the
page file; the app's edits rewrite the file and redeploy it.

A line tile charts a metric the way it adds up: running totals for things
like tokens, the plain value for things like coverage; a number tile shows
the latest value, with how much it moved in the range.

## Letting the agent build one

Ask your agent for one instead of assembling it by hand:

> Make me a dashboard with test duration, coverage, and tokens per effort,
> as numbers.

It writes the page with `oxplow.extension.new_dashboard` and
`oxplow.extension.write_page` and tells you where it is. Anything a tile
can't show (a breakdown by package or language) is a page of its own that you
pin.
