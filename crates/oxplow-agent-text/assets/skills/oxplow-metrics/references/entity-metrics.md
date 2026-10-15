# Metrics over data (entity metrics)

## Metrics over data (entity metrics)

When the number is about records oxplow already has (work items, commits,
test runs, an extension's synced entities), skip the collector. Aggregate the view
directly (the views and their columns are in `v_model` / `v_model_column`):

```yaml
metrics:
  - key: repo.items_done
    title: Work items completed
    entity: v_work_item
    where: "state = 'done'"
    time: closed_at           # optional: an event metric, bucketed by this time
    aggregation: count        # count | count_distinct | sum | avg | min | max | median | p90
    # value: "e.rank"         # the expression aggregated (not needed for count)
dimensions:
  - key: repo.priority
    entity: v_work_item
    expr: "json_extract(e.native, '$.priority')"   # a field the list declares
    # join: "LEFT JOIN v_thread t ON t.id = e.thread_id"
```

- **Event vs. state.** With `time` it is an event metric, computed live and
  bucketed by day, week or month. Without it, it is a state metric (a
  current level): oxplow captures it over time, so its history starts when
  you add it.
- **Fragments.** They are SQL over the view, aliased `e`. Only a
  dimension's `expr` sees its `join`. A fragment that doesn't compile keeps
  the metric out of the catalog; check the log.
- **Slicing.** Group by an entity dimension over the same view:
  `SELECT bucket, "repo.priority", MEASURE('repo.tasks_done') FROM
  metric_grid('week', 'repo.priority')`.
