# repo.react_use_effect — `useEffect(...)` call sites (a re-render/complexity signal).
# One ast_query pass over the .ts/.tsx tree (see repo_clone.star for why).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(call_expression function: (identifier) @f)"
    for tri in _ts_files():
        c = 0
        for m in ast_query(tri[1], tri[2], q):
            if m["text"] == "useEffect":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.react_use_effect", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
