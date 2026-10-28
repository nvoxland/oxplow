# Reading oxplow's data correctly

`oxplow_query_sql` answers whatever you ask. These are the ways a correct-looking
answer is still wrong, and the queries to start from.

## Know the schema before you query it

- `SELECT view, name, description FROM v_dataset` lists every view;
  `SELECT name, sql_type, doc FROM v_field WHERE view = 'v_test_run'`
  documents its columns. Read both; a guessed column is an error at best
  and a different column than you meant at worst.
- `v_metric_catalog` lists the metrics you can read with `MEASURE('<key>')`.

## What the result says about itself

- **`truncated`**: more rows matched than `limit` (default 500). The rows
  you got are a prefix, not the answer. Never total, rank or average a
  truncated list; aggregate in SQL instead.
- **`coverage`**: one entry per collector behind a collected dataset you
  read, with its status: `current`, `stale`, `failed`, `disabled`,
  `needs_approval`, `never`. Core's own datasets have none (core keeps them
  current). A complete result over a `failed` or `never` collector is
  missing data, not showing zero. Say so.
- **`generation`**: the extension catalog's fingerprint when the query
  ran. It changes when any extension does; it says nothing about the data.
  Compare it across two reads to see whether the catalog moved between them.
- **`freshness`**: when each dataset last changed in this process. It is not
  a staleness check on a collector; `coverage` is.

None of these says the query was *right*: they say how complete it was.

## Aggregate before the cap

`LIMIT` and `limit` cut rows after the query ran. Do the `GROUP BY`,
`count`, `sum` and `max` in SQL, so the cap only trims the answer's rows.
Fetching 500 rows and counting them in your head is the mistake.

## Metrics: use the grid, not a sum

A metric knows how it folds: sum, last, or a ratio re-derived from its
numerator and denominator. Read it with
`SELECT bucket, MEASURE('<key>') FROM metric_grid('day'|'week'|'month'[, '<dimension>'])`
(`'capture'` keeps one row per capture). Summing `v_fact.value` yourself
double counts a semi-additive measure across captures, and averages a
percentage instead of dividing its totals.

- Facts are recorded when something changed. A test that kept passing has
  no new fact; a delta capture writes only the files it rescanned. Read
  the series, which carries the last value forward, not the rows of
  `v_fact`, to ask "what is true at that time".
- Buckets are UTC: a day is a UTC day, a week starts on Monday.
- `v_capture.provenance` is `observed` (oxplow measured it) or `asserted`
  (a person or agent reported it with `oxplow.metric.record`). Keep them
  apart, and say which a number is.

## Unavailable is not empty

An empty result can mean nothing happened, or that nothing was collected.
Check `coverage`, and for a metric whether any capture exists
(`SELECT count(*) FROM v_capture`). "No test runs" and "tests were never
collected" are different answers. An error from a view you are not allowed
to read is neither.

## Scope your question

Most datasets span every stream. Say which you mean: `stream_id` (and
`thread_id`) narrow to one; `branch` on test runs, captures and
`v_test_case_stat` separates branches; a time window needs an explicit
`WHERE` on the dataset's time column. A number without its stream, branch and
window is only true somewhere.

## Examples

Each runs as written; a test runs them over a seeded project.

### Coverage trend by week

```sql
SELECT bucket, MEASURE('oxplow.coverage.abs_pct') AS coverage_pct
FROM metric_grid('week')
ORDER BY bucket
```

One row per UTC week with that week's coverage. A week with no capture is
absent: check `coverage` before reading a gap as a drop.

### Failing or flaky tests

```sql
SELECT subject, last_status, runs, failures, flips
FROM v_test_case_stat
WHERE last_status = 'failed' OR flips >= 2
ORDER BY last_status = 'failed' DESC, flips DESC, subject
```

`last_status` is the latest result; `flips` counts pass/fail changes, so a
test that fails once and stays red is failing, not flaky. Add
`AND branch = '<branch>'` for one branch.

### Efforts per thread

```sql
SELECT t.id AS thread_id, t.title,
       count(e.id) AS efforts,
       count(e.id) FILTER (WHERE e.ended_at IS NULL) AS open
FROM v_thread t
LEFT JOIN v_effort e ON e.thread_id = t.id
GROUP BY t.id, t.title
ORDER BY efforts DESC, t.id
```

The `LEFT JOIN` keeps a thread with no efforts as a row of zeros. Count
`e.id`, never `*` or a test on `e.ended_at` alone: a thread with no effort
has one all-NULL joined row, and `ended_at IS NULL` is true for it.

### Token usage per thread and agent

```sql
SELECT thread_id, agent_kind,
       sum(input_tokens + output_tokens) AS tokens,
       sum(cache_read_input_tokens) AS cache_read,
       count(*) AS reports
FROM v_token_usage
GROUP BY thread_id, agent_kind
ORDER BY tokens DESC, thread_id
```

A usage row is one report from an agent turn; it carries the thread, the
effort and the turn, not the session, so group by thread (and join
`v_agent_session` on `thread_id` for its sessions). Cache reads are
reported apart from input and output: add them only if you mean them.
