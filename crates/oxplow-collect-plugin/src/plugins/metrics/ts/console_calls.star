# oxplow.ts.console_calls — count `console.*(...)` calls (stray debug logging
# left in shipped code). Matches a call whose callee is a member access on
# `console`, whether bare (`console.log(...)`) or namespaced
# (`window.console.log(...)`, `globalThis.console.error(...)`). Emits the
# repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero only),
# and a per-file `oxplow.ast_hit` FACT (rule="console_call") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    # @o: bare `console.x(...)`; @c: `<obj>.console.x(...)`.
    q = "(call_expression function: (member_expression object: (identifier) @o)) " + \
        "(call_expression function: (member_expression object: (member_expression property: (property_identifier) @c)))"
    total = 0
    per_file = []
    facts = []
    for tri in _ts_files():
        c = 0
        for m in ast_query(tri[1], tri[2], q):
            if m["text"] == "console" and (m["capture"] == "o" or m["capture"] == "c"):
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + tri[0], "dims": {"oxplow.language": "typescript"}})
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "console_call", "subject": "file:" + tri[0], "path": tri[0], "dims": {"oxplow.language": "typescript"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"oxplow.language": "typescript"}}] + per_file, "facts": facts}
