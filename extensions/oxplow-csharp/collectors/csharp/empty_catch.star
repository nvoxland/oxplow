# oxplow.csharp.empty_catch — find each empty `catch { }` blocks (swallowed
# exceptions — a correctness smell). Queries each catch clause's body block and
# keeps the ones whose text is just braces + whitespace. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per hit
# (value 1, line, severity warning; rule="empty_catch") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def transform(input):
    facts = []
    for f in files("**/*.cs"):
        for m in ast_query(f["text"], "csharp", "(catch_clause (block) @b)"):
            if len(regex_find(r"^\{\s*\}$", m["text"])) > 0:
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "empty_catch", "severity": "warning", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = f["path"]), "path": f["path"], "dims": {"oxplow.language": "csharp"}})
    return {"facts": facts}
