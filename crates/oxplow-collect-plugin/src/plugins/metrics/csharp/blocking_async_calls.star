# oxplow.csharp.blocking_async_calls — count blocking sync-over-async sites:
# `.Result` property reads and *invoked* `.Wait()` calls (a deadlock/perf smell).
# `.Wait` only counts when actually invoked — a method-group reference like
# `var w = t.Wait;` (no parens) is not a blocking call. (A property literally
# named `Result` is still counted — the heuristic has no type info.) Emits the
# repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero only), and
# a per-file `oxplow.ast_hit` FACT (rule="blocking_async") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
def transform(input):
    q = "(member_access_expression name: (identifier) @result) " + \
        "(invocation_expression function: (member_access_expression name: (identifier) @wait))"
    total = 0
    per_file = []
    facts = []
    for f in files("**/*.cs"):
        c = 0
        for m in ast_query(f["text"], "csharp", q):
            if m["capture"] == "result" and m["text"] == "Result":
                c += 1
            elif m["capture"] == "wait" and m["text"] == "Wait":
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "csharp"}})
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "blocking_async", "subject": "file:" + f["path"], "path": f["path"], "dims": {"language": "csharp"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "csharp"}}] + per_file, "facts": facts}
