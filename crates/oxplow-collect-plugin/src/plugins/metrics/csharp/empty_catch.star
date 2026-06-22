# oxplow.csharp.empty_catch — count empty `catch { }` blocks (swallowed
# exceptions — a correctness smell). Queries each catch clause's body block and
# keeps the ones whose text is just braces + whitespace.
def transform(input):
    n = 0
    for f in files("**/*.cs"):
        for b in ast_query(f["text"], "csharp", "(catch_clause (block) @b)"):
            if len(regex_find(r"^\{\s*\}$", b["text"])) > 0:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "csharp"}}]}
