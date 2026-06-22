# oxplow.csharp.blocking_async_calls — count `.Result` / `.Wait()` member
# accesses (sync-over-async — a deadlock/perf smell). Queries member-access
# names and keeps the two blocking accessors.
def transform(input):
    n = 0
    for f in files("**/*.cs"):
        for m in ast_query(f["text"], "csharp", "(member_access_expression name: (identifier) @n)"):
            if m["text"] == "Result" or m["text"] == "Wait":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "csharp"}}]}
