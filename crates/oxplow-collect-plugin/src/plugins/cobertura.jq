# Cobertura coverage plugin (input: xml → explicit element tree).
# <class filename="X"> … <line number="N" hits="H"/>. Every line descendant
# of a class is attributed to that class's file; covered when hits > 0.
# Line coverage uses recursive descent (dup lines under a method AND the class's
# <lines> collapse in the destination BTreeSet, so double-counting is harmless).
#
# Branch coverage rides in condition-coverage="H% (a/b)" — a = covered, b = total.
# It is summed ONLY over the class's DIRECT <lines> (not method <lines>), because
# `+=` would otherwise double-count a branch present under both. Function coverage
# counts <method> elements: found = count, hit = methods with line-rate > 0.
# A line whose `number` isn't numeric is skipped, not fatal for the whole report.
{
  files: reduce ([.. | select((type == "object") and (.tag == "class") and (.attrs.filename != null))][]) as $c
    ({};
      ($c.attrs.filename) as $path
      | ([$c | .. | select((type == "object") and (.tag == "method"))]) as $methods
      | ($methods | length) as $mfound
      | ([$methods[] | select((.attrs["line-rate"] // "0" | tonumber? // 0) > 0)] | length) as $mhit
      # Direct <class><lines><line …>, method <lines> excluded — see header.
      | ([$c.children[]? | select(.tag == "lines") | .children[]?
          | select((.tag == "line") and (.attrs["condition-coverage"] != null))
          | (.attrs["condition-coverage"] | split("(") | .[1] // "" | rtrimstr(")") | split("/"))]) as $ccs
      | ($ccs | map(.[0] // "" | tonumber? // 0) | add // 0) as $bhit
      | ($ccs | map(.[1] // "" | tonumber? // 0) | add // 0) as $bfound
      | . + { ($path): (
          (reduce ([$c | .. | select((type == "object") and (.tag == "line") and (.attrs.number != null))][]) as $ln
            ((.[$path] // { instrumented: [], covered: [], branchesFound: 0, branchesHit: 0, functionsFound: 0, functionsHit: 0 });
              ($ln.attrs.number | tonumber? // null) as $n
              | if $n == null then .
                else .instrumented += [$n]
                  | (if ($ln.attrs.hits // "0" | tonumber? // 0) > 0 then .covered += [$n] else . end)
                end))
          | .branchesHit += $bhit
          | .branchesFound += $bfound
          | .functionsHit += $mhit
          | .functionsFound += $mfound
        ) })
}
