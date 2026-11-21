# oxplow.rust.unwrap_expect_calls — find each `.unwrap()` / `.expect()` method
# calls (a panic-risk signal). Matches method-call field identifiers via the
# AST and keeps the two panic-prone names. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per hit
# (value 1, line, severity info; rule="unwrap_expect") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def transform(input):
    facts = []
    q = "(call_expression function: (field_expression field: (field_identifier) @m))"
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", q):
            if m["text"] == "unwrap" or m["text"] == "expect":
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "unwrap_expect", "severity": "info", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = f["path"]), "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
