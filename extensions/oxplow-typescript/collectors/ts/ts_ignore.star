# oxplow.ts.ts_ignore — find each `@ts-ignore` / `@ts-expect-error` suppression
# directives (type-error escape hatches). Scoped to comment nodes. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files.
# One `oxplow.ast_hit` FACT per hit (value 1, line, severity warning; rule="ts_ignore") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    for tri in _ts_files():
        for m in ast_query(tri[1], tri[2], "(comment) @c"):
            for _ in regex_find(r"@ts-(ignore|expect-error)", m["text"]):
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "ts_ignore", "severity": "warning", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = tri[0]), "path": tri[0], "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
