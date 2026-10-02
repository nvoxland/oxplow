# repo.ts_as_any — `x as any` casts: the most dangerous assertion (silences ALL
# type-checking on the value), a subset of repo.ts_as_cast worth its own alarm.
# The asserted type in a cast is a direct child of `as_expression`; `any` is a
# `predefined_type` (node kinds pinned by oxplow-code-metrics tests).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(as_expression (predefined_type) @t)"
    for tri in _ts_files():
        c = 0
        for t in ast_query(tri[1], tri[2], q):
            if t["text"] == "any":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.ts_as_any", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
