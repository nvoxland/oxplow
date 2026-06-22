# oxplow.csharp.blocking_async_calls — count blocking sync-over-async sites:
# `.Result` property reads and *invoked* `.Wait()` calls (a deadlock/perf smell).
# `.Wait` only counts when actually invoked — a method-group reference like
# `var w = t.Wait;` (no parens) is not a blocking call. (A property literally
# named `Result` is still counted — the heuristic has no type info.)
def transform(input):
    q = "(member_access_expression name: (identifier) @result) " + \
        "(invocation_expression function: (member_access_expression name: (identifier) @wait))"
    n = 0
    for f in files("**/*.cs"):
        for m in ast_query(f["text"], "csharp", q):
            if m["capture"] == "result" and m["text"] == "Result":
                n += 1
            elif m["capture"] == "wait" and m["text"] == "Wait":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "csharp"}}]}
