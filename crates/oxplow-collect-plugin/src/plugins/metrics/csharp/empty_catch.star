# oxplow.csharp.empty_catch — count empty `catch { }` blocks (swallowed
# exceptions — a correctness smell). Queries each catch clause's body block and
# keeps the ones whose text is just braces + whitespace. Emits the repo-total
# ("tree:.") plus a per-file sample ("file:<path>", nonzero only), and a per-file
# `oxplow.ast_hit` FACT (rule="empty_catch") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
def transform(input):
    total = 0
    per_file = []
    facts = []
    for f in files("**/*.cs"):
        c = 0
        for b in ast_query(f["text"], "csharp", "(catch_clause (block) @b)"):
            if len(regex_find(r"^\{\s*\}$", b["text"])) > 0:
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "csharp"}})
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "empty_catch", "subject": "file:" + f["path"], "path": f["path"], "dims": {"language": "csharp"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "csharp"}}] + per_file, "facts": facts}
