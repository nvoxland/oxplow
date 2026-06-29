# JUnit test plugin (input: xml → explicit element tree).
# <testsuite name="S"> … <testcase classname="C" name="N" time="T"> with an
# optional <failure>/<error> (→ failed) or <skipped> (→ skipped) child;
# otherwise passed. Mirrors oxplow_coverage::parse_junit. Tolerant of the
# <testsuites> wrapper and a bare <testsuite> root.
#
# Each testcase is taken from its IMMEDIATE parent testsuite's direct
# children — NOT a recursive descent — so a NESTED testsuite (bun emits
# file-suite → describe-suite → testcase) doesn't count the same case
# under both levels (the failed=2-for-1-failure double-count).
{
  suites: [
    ([.. | select((type == "object") and (.tag == "testsuite"))][])
    | {
        name: (.attrs.name // ""),
        cases: [
          ((.children // [])[] | select((type == "object") and (.tag == "testcase")))
          | {
              classname: (.attrs.classname // ""),
              name: (.attrs.name // ""),
              status: (
                if ([.. | select((type == "object") and ((.tag == "failure") or (.tag == "error")))] | length) > 0
                then "failed"
                elif ([.. | select((type == "object") and (.tag == "skipped"))] | length) > 0
                then "skipped"
                else "passed"
                end
              ),
              timeMs: (if .attrs.time != null then ((.attrs.time | tonumber) * 1000 | round) else null end)
            }
        ]
      }
  ]
}
