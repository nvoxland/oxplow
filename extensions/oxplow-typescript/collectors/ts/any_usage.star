# oxplow.ts.any_usage — find each `any` type annotations across the TS/TSX tree
# (an escape-hatch / type-safety-erosion signal). `any` parses as a
# `predefined_type` node, so we match those and keep the ones spelled `any`.
# Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. and an `oxplow.ast_hit` FACT per file (its count, the lines in detail, severity info),
# (rule="any_usage") — the metric is the SPEC Sum(oxplow.ast_hit) filtered by
# that rule.
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
        for m in ast_query(tri[1], tri[2], "(predefined_type) @t"):
            if m["text"] == "any":
                hits.setdefault(tri[0], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "any_usage", "info", "typescript")}
