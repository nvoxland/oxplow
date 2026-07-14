# repo.eslint_disable — `eslint-disable` directives: TS/JS lints being hidden.
# One ast_query pass (comment nodes only, so the regex runs on tiny texts).
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
        for cm in ast_query(tri[1], tri[2], "(comment) @c"):
            c += len(regex_find(r"eslint-disable", cm["text"]))
        if c > 0:
            facts.append({"measure": "repo.eslint_disable", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
