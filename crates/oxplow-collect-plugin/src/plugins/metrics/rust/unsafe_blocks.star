# oxplow.rust.unsafe_blocks — count `unsafe { … }` blocks across the Rust tree.
# A tree-derived gauge: reads the snapshot via files() and the AST via
# ast_query(). Deterministic (no I/O) → observed.
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        n += len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
