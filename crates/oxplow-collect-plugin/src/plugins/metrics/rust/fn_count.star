# oxplow.rust.fn_count — total `fn` definitions across the Rust tree (a coarse
# size/structure gauge). Counts `function_item` nodes (free fns, methods, assoc
# fns); closures are not included.
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        n += len(ast_query(f["text"], "rust", "(function_item) @f"))
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
