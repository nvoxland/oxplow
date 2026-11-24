# oxplow.ts.non_null_assertions — find each `expr!` non-null assertions (a
# type-checker override that can hide real nullability bugs). Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files.
# One `oxplow.ast_hit` FACT per file, the hits' lines in its detail (value 1, line, severity info) (rule="non_null_assertion") — the metric
# is the SPEC Sum(oxplow.ast_hit) filtered by that rule.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

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
    for tri in _ts_files():
        for m in ast_query(tri[1], tri[2], "(non_null_expression) @n"):
            hits.setdefault(tri[0], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "non_null_assertion", "info", "typescript")}
