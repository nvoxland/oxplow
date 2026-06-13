# eslint analysis plugin (input: json → array of file results).
# `eslint -f json` emits an array of `{ filePath, messages: [ { ruleId, severity,
# line, column, message } ] }`. eslint severity is 2 = error, 1 = warning
# (0 = off never appears in output). One finding per message; `ruleId` may be
# null (e.g. a parser error), which maps to a finding with no rule.
{
  findings: [
    .[]
    | .filePath as $path
    | (.messages // [])[]
    | {
        path: $path,
        line: .line,
        column: .column,
        severity: (if .severity == 2 then "error"
                   elif .severity == 1 then "warning"
                   else "info" end),
        rule: .ruleId,
        message: .message
      }
  ]
}
