---
date: 2026-07-22
categories:
  - Releases
---

# Oxplow 0.6 -- Improving Metrics

0.6 is mostly one thing: measuring your codebase and your agents' work, and storing those numbers in a way that stays correct as history grows.

<!-- more -->

## Metrics

The metric system was rebuilt. Instead of a fixed set of numbers data knows how to collect, it's a pluggable system feeding into a common storage.

- Define measures, dimensions, gauges, and metrics in `project.yaml`. Metrics are specs over measures -- a filter, an aggregation, optionally a formula combining two of them -- so you can add a new view of existing data without recollecting anything.
- Ratio metrics keep their numerator and denominator, which means an average of ratios is computed correctly rather than averaging the percentages.
- Targets, warn/fail thresholds, and direction (higher-better or lower-better) are part of the definition, so a metric knows what "off target" means.
- Drill down by dimension -- package, language, file, test suite -- and step between siblings without going back to the list.
- Reads are branch-aware, so a metric on a feature branch doesn't blend into main's history.

The system is designed for your agents to create additional metrics you care about with the `scaffold_metric` MCP tool.

## Dashboards

You can build your own dashboards out of metric tiles -- sparkline, bar, and text tiles, resizable, drag to reorder. Filters set on the dashboard flow down into the tiles, and there's an "Add to dashboard..." action on the metric detail page so you can assemble one as you browse. Agents can author dashboards over MCP as well.

## Token and effort economics

One of the new set of metrics added are agent token usage:

- Tokens per turn and per effort, split by kind, with cache-hit ratio.
- Time-to-green and steering-per-effort.
- A wasted-token ratio derived from git-revert detection, which is a rough proxy but an interesting one.

## Upgrading from 0.5

Two things moved, and there's no automatic migration. Do this with Oxplow closed:

- `oxplow.yaml` -> `.oxplow/project.yaml`
- `.oxplow/state.sqlite*` -> `.oxplow/local.sqlite*`
