# Dashboards

A dashboard is your own arrangement of metric tiles. The built-in
[Metrics](metrics.md) pages show every metric oxplow knows about; a
dashboard is the handful you actually want to watch, on one screen.

## Making one

`New Dashboard…` from the launcher (++cmd+p++), or `+ New dashboard`
on the Dashboards index. Then either:

- `+ Add metric` on the dashboard itself, or
- `Add to dashboard ▾` from any metric's detail page, which also
  offers `New dashboard…`.

Dashboards live in the database, not in `.oxplow/project.yaml`, so
they're local to you rather than shared with the project.

## Tiles

A tile can also be a [lens](lenses.md): **Pin to Dashboard** on any lens page
adds it, showing its first few rows.

Each metric tile is a **line** chart (the default) or a **number**
(the latest value, with how much it moved in the range). A line tile
charts the metric the way it adds up: running totals for things like
tokens, the plain value for things like coverage. There's also a plain
**text** item for labelling a group of tiles -- it's literal text, not
markdown.

Sizes are `small` (default), `wide`, `tall`, and `full`. Right-click
a tile for its Visualization and Size submenus and Remove. Drag a tile
to reorder.

## Filtering the whole board

The header carries **Range** (defaults to All time) and **Branch**.
Every tile follows them unless it has its own.

Dashboards are deliberately plain. For a breakdown by package or
language, or anything a tile can't show, ask your agent for a
[lens](lenses.md) and pin that.

## Letting the agent build one

Dashboards have an MCP surface (`list_dashboards`, `get_dashboard`,
`create_dashboard`, `add_dashboard_item`), so you can ask for one
instead of assembling it by hand:

> Make me a dashboard with test duration, coverage, and tokens per
> effort, as numbers.
