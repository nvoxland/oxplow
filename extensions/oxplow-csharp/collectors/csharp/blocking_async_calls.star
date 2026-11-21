# oxplow.csharp.blocking_async_calls — find each blocking sync-over-async sites:
# `.Result` property reads and *invoked* `.Wait()` calls (a deadlock/perf smell).
# `.Wait` only counts when actually invoked — a method-group reference like
# `var w = t.Wait;` (no parens) is not a blocking call. (A property literally
# named `Result` is still counted — the heuristic has no type info.) Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per hit
# (value 1, line, severity warning; rule="blocking_async") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule.
def transform(input):
    q = "(member_access_expression name: (identifier) @result) " + \
        "(invocation_expression function: (member_access_expression name: (identifier) @wait))"
    facts = []
    for f in files("**/*.cs"):
        for m in ast_query(f["text"], "csharp", q):
            if (m["capture"] == "result" and m["text"] == "Result") or (m["capture"] == "wait" and m["text"] == "Wait"):
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "blocking_async", "severity": "warning", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = f["path"]), "path": f["path"], "dims": {"oxplow.language": "csharp"}})
    return {"facts": facts}
