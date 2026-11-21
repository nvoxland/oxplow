# oxplow.csharp.blocking_async_calls — find each blocking sync-over-async sites:
# `.Result` property reads and *invoked* `.Wait()` calls (a deadlock/perf smell).
# `.Wait` only counts when actually invoked — a method-group reference like
# `var w = t.Wait;` (no parens) is not a blocking call. (A property literally
# named `Result` is still counted — the heuristic has no type info.) Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per file, the hits' lines in its detail
# (value 1, line, severity warning; rule="blocking_async") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule.
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
    q = "(member_access_expression name: (identifier) @result) " + \
        "(invocation_expression function: (member_access_expression name: (identifier) @wait))"
    hits = {}
    for f in files("**/*.cs"):
        for m in ast_query(f["text"], "csharp", q):
            if (m["capture"] == "result" and m["text"] == "Result") or (m["capture"] == "wait" and m["text"] == "Wait"):
                hits.setdefault(f["path"], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "blocking_async", "warning", "csharp")}
