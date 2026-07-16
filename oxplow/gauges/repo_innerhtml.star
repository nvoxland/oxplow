# repo.innerhtml_assign — `x.innerHTML = ...` assignments (an XSS-injection
# surface). One ast_query pass over the .ts/.tsx tree: the LHS of an assignment
# that is a member access whose property is `innerHTML`.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(assignment_expression left: (member_expression property: (property_identifier) @p))"
    for tri in _ts_files():
        c = 0
        for p in ast_query(tri[1], tri[2], q):
            if p["text"] == "innerHTML":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.innerhtml_assign", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
