# oxplow.rust.todo_markers — count TODO / FIXME markers inside comments only
# (so a "TODO" in a string literal doesn't inflate the count). Walks comment
# nodes via the AST, then regex-matches the marker words. Emits the repo-total
# ("tree:.") plus a per-file sample ("file:<path>", nonzero only).
def transform(input):
    total = 0
    per_file = []
    for f in files("**/*.rs"):
        c = 0
        for cm in ast_query(f["text"], "rust", "[(line_comment) (block_comment)] @c"):
            c += len(regex_find(r"(?i)\b(TODO|FIXME)\b", cm["text"]))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file}
