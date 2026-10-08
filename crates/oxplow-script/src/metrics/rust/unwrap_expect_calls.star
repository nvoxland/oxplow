# oxplow.rust.unwrap_expect_calls — count `.unwrap()` / `.expect()` method
# calls (a panic-risk signal). Matches method-call field identifiers via the
# AST and keeps the two panic-prone names. Its facts are per-file
# (subject "file:<path>", nonzero only), so an effort's change attributes
# through its files. and a
# per-file `oxplow.ast_hit` FACT (rule="unwrap_expect") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
def transform(input):
    facts = []
    q = "(call_expression function: (field_expression field: (field_identifier) @m))"
    for f in files("**/*.rs"):
        c = 0
        for m in ast_query(f["text"], "rust", q):
            if m["text"] == "unwrap" or m["text"] == "expect":
                c += 1
        if c > 0:
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "unwrap_expect", "subject": "file:" + f["path"], "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
