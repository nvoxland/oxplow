# oxplow.rust.fn_count — total `fn` definitions across the Rust tree (a coarse
# size/structure gauge). Counts `function_item` nodes (free fns, methods, assoc
# fns); closures are not included. Emits the repo-total ("tree:.") plus a
# per-file sample ("file:<path>", nonzero only) for effort attribution.
def transform(input):
    total = 0
    per_file = []
    for f in files("**/*.rs"):
        c = len(ast_query(f["text"], "rust", "(function_item) @f"))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file}
