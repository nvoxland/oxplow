# oxplow.ts.console_calls — count `console.*(...)` calls (stray debug logging
# left in shipped code). Matches a call whose callee is a member access on
# `console`, whether bare (`console.log(...)`) or namespaced
# (`window.console.log(...)`, `globalThis.console.error(...)`).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["text"], "tsx"))
    return out

def transform(input):
    # @o: bare `console.x(...)`; @c: `<obj>.console.x(...)`.
    q = "(call_expression function: (member_expression object: (identifier) @o)) " + \
        "(call_expression function: (member_expression object: (member_expression property: (property_identifier) @c)))"
    n = 0
    for pair in _ts_files():
        for m in ast_query(pair[0], pair[1], q):
            if m["text"] == "console" and (m["capture"] == "o" or m["capture"] == "c"):
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
