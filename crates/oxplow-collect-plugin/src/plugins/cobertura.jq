# Cobertura coverage plugin (input: xml → explicit element tree).
# <class filename="X"> … <line number="N" hits="H"/>. Every line descendant
# of a class is attributed to that class's file; covered when hits > 0.
# Mirrors oxplow_coverage::parse_cobertura.
{
  files: reduce ([.. | select((type == "object") and (.tag == "class") and (.attrs.filename != null))][]) as $c
    ({};
      ($c.attrs.filename) as $path
      | . + { ($path): (
          reduce ([$c | .. | select((type == "object") and (.tag == "line") and (.attrs.number != null))][]) as $ln
            ((.[$path] // { instrumented: [], covered: [] });
              (($ln.attrs.number) | tonumber) as $n
              | .instrumented += [$n]
              | (if (($ln.attrs.hits // "0") | tonumber) > 0 then .covered += [$n] else . end))
        ) })
}
