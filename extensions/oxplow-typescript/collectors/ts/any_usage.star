# oxplow.ts.any_usage — find each `any` type annotations across the TS/TSX tree
# (an escape-hatch / type-safety-erosion signal). `any` parses as a
# `predefined_type` node, so we match those and keep the ones spelled `any`.
# Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. and an `oxplow.ast_hit` FACT, one per hit (value 1, line, severity info),
# (rule="any_usage") — the metric is the SPEC Sum(oxplow.ast_hit) filtered by
# that rule.
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
        for m in ast_query(tri[1], tri[2], "(predefined_type) @t"):
            if m["text"] == "any":
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "any_usage", "severity": "info", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = tri[0]), "path": tri[0], "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
