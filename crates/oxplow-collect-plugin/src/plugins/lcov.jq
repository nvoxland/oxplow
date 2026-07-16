# lcov coverage plugin (input: lcov → array of records).
# Each record: { "SF": ["<path>"], "DA": ["<line>,<hits>[,…]"], … }.
# A line is covered when hits > 0.
# Malformed DA entries (non-numeric line) are skipped, not fatal — `tonumber?`
# yields empty on failure, and `// null` keeps each step single-valued.
#
# PERFORMANCE (tsk88) — this must stay LINEAR. The original built each file's
# line lists with `.instrumented += [$n]` inside a `reduce`, copying the whole
# growing array once per entry: quadratic PER FILE. This repo's real report has
# a 4783-line file, so that one record alone cost ~11M element copies and the
# report blew the 5s SandboxBudget. A timeout there is NOT a limit — the worker
# thread is DETACHED (Rust can't kill a thread) and keeps burning a core, so the
# budget only bounded how long we waited before starting more of it. Coverage was
# never once ingested for this project as a result.
#
# `map` builds each array in a single pass, so no accumulator is ever copied.
# Keep it that way: a `reduce` with `+=` over per-line data reintroduces the bug.
# `lcov_plugin_parses_a_whole_workspace_report_well_inside_the_budget` pins it.
{
  files: (
    map(
      select((.SF[0]) != null)
      | (
          ((.DA) // [])
          | map(
              split(",")
              | {
                  n: ((.[0] | tonumber?) // null),
                  h: ((.[1] | tonumber?) // 0),
                }
            )
          | map(select(.n != null))
        ) as $das
      | {
          key: (.SF[0]),
          value: {
            instrumented: ($das | map(.n)),
            covered: ($das | map(select(.h > 0) | .n)),
          },
        }
    )
    # Later record wins for a repeated SF — same as the original `. + {…}`.
    | from_entries
  )
}
