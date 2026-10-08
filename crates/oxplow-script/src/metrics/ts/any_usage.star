# oxplow.ts.any_usage — count `any` type annotations across the TS/TSX tree
# (an escape-hatch / type-safety-erosion signal). `any` parses as a
# `predefined_type` node, so we match those and keep the ones spelled `any`.
# Its facts are per-file
# (subject "file:<path>", nonzero only), so an effort's change attributes
# through its files. and a per-file `oxplow.ast_hit` FACT
# (rule="any_usage") — the metric is the SPEC Sum(oxplow.ast_hit) filtered by
# that rule (epic tsk12).
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
        c = 0
        for t in ast_query(tri[1], tri[2], "(predefined_type) @t"):
            if t["text"] == "any":
                c += 1
        if c > 0:
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "any_usage", "subject": "file:" + tri[0], "path": tri[0], "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
