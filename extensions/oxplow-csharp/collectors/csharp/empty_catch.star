# oxplow.csharp.empty_catch — find each empty `catch { }` blocks (swallowed
# exceptions — a correctness smell). Queries each catch clause's body block and
# keeps the ones whose text is just braces + whitespace. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per file, the hits' lines in its detail
# (value 1, line, severity warning; rule="empty_catch") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
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
    hits = {}
    for f in files("**/*.cs"):
        for m in ast_query(f["text"], "csharp", "(catch_clause (block) @b)"):
            if len(regex_find(r"^\{\s*\}$", m["text"])) > 0:
                hits.setdefault(f["path"], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "empty_catch", "warning", "csharp")}
