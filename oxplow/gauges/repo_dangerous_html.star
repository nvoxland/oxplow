# repo.dangerous_html — `dangerouslySetInnerHTML` usages (XSS-injection surface).
# One ast_query pass over the .ts/.tsx tree (see repo_shared_state.star for why).
# The JSX attribute name is a `property_identifier`.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(property_identifier) @p"
    for tri in _ts_files():
        c = 0
        for p in ast_query(tri[1], tri[2], q):
            if p["text"] == "dangerouslySetInnerHTML":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.dangerous_html", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
