# oxplow.rust.unwrap_expect_calls — count `.unwrap()` / `.expect()` method
# calls (a panic-risk signal). Matches method-call field identifiers via the
# AST and keeps the two panic-prone names.
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        q = "(call_expression function: (field_expression field: (field_identifier) @m))"
        for m in ast_query(f["text"], "rust", q):
            if m["text"] == "unwrap" or m["text"] == "expect":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
