# type_coverage.jq — maps `type-coverage --json-output` to a single ratio FACT on
# repo.type_coverage (num = well-typed nodes, den = total nodes, value = %).
#
# input: text (raw report) parsed defensively with `try fromjson` — a missing or
# empty report (`target/type-coverage.json` not generated yet) yields no fact
# instead of failing the gauge. The tool's shape:
#   { "correctCount": N, "totalCount": M, "percent": P, ... }
(try fromjson catch null) as $r
| { facts: (
    if ($r | type) == "object" and (($r.totalCount) // 0) > 0
    then [ {
      measure: "repo.type_coverage",
      value: ($r.percent // 0),
      num: ($r.correctCount // 0),
      den: ($r.totalCount // 0),
    } ]
    else []
    end
  ) }
