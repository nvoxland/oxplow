# lcov coverage plugin (input: lcov → array of records).
# Each record: { "SF": ["<path>"], "DA": ["<line>,<hits>[,…]"], … }.
# A line is covered when hits > 0. Mirrors oxplow_coverage::parse_lcov.
{
  files: reduce .[] as $r ({};
    ($r.SF[0]) as $path
    | if $path == null then .
      else . + { ($path): (
          reduce (($r.DA) // [])[] as $da
            ({ instrumented: [], covered: [] };
              ($da | split(",")) as $p
              | (($p[0]) | tonumber) as $n
              | .instrumented += [$n]
              | (if (($p[1]) | tonumber) > 0 then .covered += [$n] else . end))
        ) }
      end)
}
