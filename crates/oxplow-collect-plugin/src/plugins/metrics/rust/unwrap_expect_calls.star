# oxplow.rust.unwrap_expect_calls — count `.unwrap()` / `.expect()` method
# calls (a panic-risk signal). Matches method-call field identifiers via the
# AST and keeps the two panic-prone names. Emits the repo-total ("tree:.") plus
# a per-file sample ("file:<path>", nonzero only) for effort attribution.
def transform(input):
    total = 0
    per_file = []
    q = "(call_expression function: (field_expression field: (field_identifier) @m))"
    for f in files("**/*.rs"):
        c = 0
        for m in ast_query(f["text"], "rust", q):
            if m["text"] == "unwrap" or m["text"] == "expect":
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file}
