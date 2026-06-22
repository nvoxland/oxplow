# oxplow.ts.console_calls — count `console.*(...)` calls (stray debug logging
# left in shipped code). Matches a call whose callee is a member access on the
# `console` identifier.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["text"], "tsx"))
    return out

def transform(input):
    n = 0
    q = "(call_expression function: (member_expression object: (identifier) @o))"
    for pair in _ts_files():
        for m in ast_query(pair[0], pair[1], q):
            if m["text"] == "console":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
