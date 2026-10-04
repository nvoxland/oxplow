# clippy / rustc analysis plugin (input: lines → array of strings).
# `cargo clippy --message-format=json` emits one JSON object per line (the
# cargo message stream). We keep only `compiler-message` entries that carry a
# diagnostic with at least one source span, and emit one finding per message
# using its primary span (or the first span when none is flagged primary).
# Non-JSON lines (blank lines, plain stderr) are tolerated: `fromjson?` yields
# empty on a parse error so they're skipped. Span-less summary messages
# ("N warnings emitted") have no real location and are dropped.
{
  findings: [
    .[]
    | (fromjson?)
    | select(.reason == "compiler-message")
    | .message
    | select(.spans != null and (.spans | length) > 0)
    | . as $m
    | (([.spans[] | select(.is_primary == true)] + .spans)[0]) as $span
    | {
        path: $span.file_name,
        line: $span.line_start,
        column: $span.column_start,
        severity: ($m.level
          | if . == "error" then "error"
            elif . == "warning" then "warning"
            elif . == "note" or . == "help" then "note"
            else "info" end),
        rule: ($m.code.code),
        message: $m.message
      }
  ]
}
