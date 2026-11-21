# oxplow.rust.unwrap_expect_calls — find each `.unwrap()` / `.expect()` method
# calls (a panic-risk signal). Matches method-call field identifiers via the
# AST and keeps the two panic-prone names. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per file, the hits' lines in its detail
# (value 1, line, severity info; rule="unwrap_expect") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def _per_file(hits, rule, severity, language):
    """One fact per file: the count (what the metric sums) and the hits'
    lines in `detail` (what the change review matches to changed lines).
    One fact per hit would multiply what the metric cube folds."""
    facts = []
    for path in sorted(hits.keys()):
        lines = hits[path]
        facts.append({"measure": "oxplow.ast_hit", "value": len(lines), "rule": rule, "severity": severity, "detail": json.encode({"lines": lines}), "subject": uri.build("oxplow/file", path = path), "path": path, "dims": {"oxplow.language": language}})
    return facts

def transform(input):
    hits = {}
    q = "(call_expression function: (field_expression field: (field_identifier) @m))"
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", q):
            if m["text"] == "unwrap" or m["text"] == "expect":
                hits.setdefault(f["path"], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "unwrap_expect", "info", "rust")}
