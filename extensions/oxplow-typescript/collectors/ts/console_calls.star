# oxplow.ts.console_calls — find each `console.*(...)` calls (stray debug logging
# left in shipped code). Matches a call whose callee is a member access on
# `console`, whether bare (`console.log(...)`) or namespaced
# (`window.console.log(...)`, `globalThis.console.error(...)`). Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files.
# One `oxplow.ast_hit` FACT per file, the hits' lines in its detail (value 1, line, severity info) (rule="console_call") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def _per_file(hits, rule, severity, language):
    """One fact per file: the count (what the metric sums) and the hits'
    lines in `detail` (what the change review matches to changed lines).
    One fact per hit would multiply what the metric cube folds."""
    facts = []
    for path in sorted(hits.keys()):
        lines = hits[path]
        facts.append({"measure": "oxplow.ast_hit", "value": len(lines), "rule": rule, "severity": severity, "detail": json.encode({"lines": lines}), "subject": uri.build("oxplow/file", path = path), "path": path, "dims": {"oxplow.language": language}})
    return facts

def transform(input):
    # @o: bare `console.x(...)`; @c: `<obj>.console.x(...)`.
    q = "(call_expression function: (member_expression object: (identifier) @o)) " + \
        "(call_expression function: (member_expression object: (member_expression property: (property_identifier) @c)))"
    hits = {}
    for tri in _ts_files():
        for m in ast_query(tri[1], tri[2], q):
            if m["text"] == "console" and (m["capture"] == "o" or m["capture"] == "c"):
                hits.setdefault(tri[0], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "console_call", "info", "typescript")}
