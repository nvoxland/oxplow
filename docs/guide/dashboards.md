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

Each metric tile renders as a **line** chart (the default),
**number**, **sparkline**, or **bar**. There's also a plain **text**
item for labelling a group of tiles -- it's literal text, not
markdown.

Sizes are `small` (default), `wide`, `tall`, and `full`. Right-click
a tile for its Visualization and Size submenus, the
`Warn when off target` toggle, and Remove. Drag a tile to reorder.

A tile whose metric has a target shows an `Off target` or `Failing`
chip when it breaches. That's on by default; turn it off per tile.

## Filtering the whole board

The header carries **Range** (defaults to All time), **Branch**, and
a **Filter by** dimension + value. These cascade to every tile, so
you can scope a whole board to one package or one language without
touching each tile.

Per-tile settings win over the board's. A tile that can't honor the
current filter -- because its metric doesn't carry that dimension --
dims with a dashed overlay rather than showing a wrong number.

`Save` stores the current range, branch, and filter as the board's
default. `Save Copy` forks the board so you can keep the original.

## Letting the agent build one

Dashboards have an MCP surface (`list_dashboards`, `get_dashboard`,
`create_dashboard`, `add_dashboard_item`), so you can ask for one
instead of assembling it by hand:

> Make me a dashboard with test duration, coverage, and tokens per
> effort, all as sparklines.
