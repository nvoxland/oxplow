# repo.eval_usage — `eval(...)` calls: dynamic code execution / a code-injection
# surface (defensive-security signal). TS/JS. A security guardrail that reads 0
# on a clean tree and alarms the moment one lands.
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
            if m["text"] == "eval":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.eval_usage", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
