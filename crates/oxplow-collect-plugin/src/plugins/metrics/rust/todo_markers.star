# oxplow.rust.todo_markers — count TODO / FIXME markers inside comments only
# (so a "TODO" in a string literal doesn't inflate the count). Walks comment
# nodes via the AST, then regex-matches the marker words.
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        for c in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            n += len(regex_find(r"(?i)\b(TODO|FIXME)\b", c["text"]))
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
