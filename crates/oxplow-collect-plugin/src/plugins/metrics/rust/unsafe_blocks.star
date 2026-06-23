# oxplow.rust.unsafe_blocks — count `unsafe { … }` blocks across the Rust tree.
# A tree-derived gauge: reads the snapshot via files() and the AST via
# ast_query(). Deterministic (no I/O) → observed. Emits the repo-total
# (subject "tree:.") plus one per-file sample (subject "file:<path>", nonzero
# only) so an effort's change can be attributed via its claimed files.
def transform(input):
    total = 0
    per_file = []
    for f in files("**/*.rs"):
        c = len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file}
