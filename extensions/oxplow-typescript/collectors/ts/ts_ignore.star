# oxplow.ts.ts_ignore — find each `@ts-ignore` / `@ts-expect-error` suppression
# directives (type-error escape hatches). Scoped to comment nodes. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files.
# One `oxplow.ast_hit` FACT per file, the hits' lines in its detail (value 1, line, severity warning; rule="ts_ignore") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule.
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
        for m in ast_query(tri[1], tri[2], "(comment) @c"):
            for _ in regex_find(r"@ts-(ignore|expect-error)", m["text"]):
                hits.setdefault(tri[0], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "ts_ignore", "warning", "typescript")}
