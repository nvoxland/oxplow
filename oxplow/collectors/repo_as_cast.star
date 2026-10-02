# repo.ts_as_cast — `x as T` type-assertion casts (a type-safety escape hatch;
# `as const` counts too, but those are rare). One ast_query pass over the
# .ts/.tsx tree (see repo_shared_state.star for why).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(as_expression) @a"
    for tri in _ts_files():
        c = len(ast_query(tri[1], tri[2], q))
        if c > 0:
            facts.append({"measure": "repo.ts_as_cast", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
