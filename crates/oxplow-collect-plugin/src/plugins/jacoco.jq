# JaCoCo coverage plugin (input: xml → explicit element tree).
# <package name="P"> … <sourcefile name="F"> … <line nr="N" ci="C" mi="M"/>.
# Path is P/F (bare F when P empty); covered when covered-instructions ci > 0.
# Branch + function/method coverage come from the sourcefile-level
# <counter type="BRANCH"/"METHOD" covered="" missed=""> summaries (found =
# covered + missed). Absent counter → 0. A line whose `nr` isn't numeric is
# skipped (`tonumber? // null`), not fatal for the whole report.
{
  files: reduce ([.. | select((type == "object") and (.tag == "package"))][]) as $pkg
    ({};
      ($pkg.attrs.name // "") as $pname
      | reduce ([$pkg | .. | select((type == "object") and (.tag == "sourcefile") and (.attrs.name != null))][]) as $sf
          (.;
            ($sf.attrs.name) as $sname
            | (if $pname == "" then $sname else (($pname | rtrimstr("/")) + "/" + $sname) end) as $path
            | ([$sf | .. | select((type == "object") and (.tag == "counter") and (.attrs.type == "BRANCH"))][0]) as $br
            | ([$sf | .. | select((type == "object") and (.tag == "counter") and (.attrs.type == "METHOD"))][0]) as $me
            | (($br.attrs.covered // "0") | tonumber? // 0) as $brc
            | (($br.attrs.missed // "0") | tonumber? // 0) as $brm
            | (($me.attrs.covered // "0") | tonumber? // 0) as $mec
            | (($me.attrs.missed // "0") | tonumber? // 0) as $mem
            | . + { ($path): (
                (reduce ([$sf | .. | select((type == "object") and (.tag == "line") and (.attrs.nr != null))][]) as $ln
                  ((.[$path] // { instrumented: [], covered: [] });
                    ($ln.attrs.nr | tonumber? // null) as $n
                    | if $n == null then .
                      else .instrumented += [$n]
                        | (if ($ln.attrs.ci // "0" | tonumber? // 0) > 0 then .covered += [$n] else . end)
                      end))
                | .branchesHit = $brc
                | .branchesFound = ($brc + $brm)
                | .functionsHit = $mec
                | .functionsFound = ($mec + $mem)
              ) }))
}
