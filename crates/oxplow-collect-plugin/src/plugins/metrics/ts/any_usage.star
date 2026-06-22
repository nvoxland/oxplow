# oxplow.ts.any_usage — count `any` type annotations across the TS/TSX tree
# (an escape-hatch / type-safety-erosion signal). `any` parses as a
# `predefined_type` node, so we match those and keep the ones spelled `any`.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["text"], "tsx"))
    return out

def transform(input):
    n = 0
    for pair in _ts_files():
        for t in ast_query(pair[0], pair[1], "(predefined_type) @t"):
            if t["text"] == "any":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
