# oxplow.ts.fn_count — total function definitions across the TS/TSX tree
# (declarations, methods, arrow + function expressions) — a coarse size gauge.
# Emits the repo-total ("tree:.") plus a per-file sample ("file:<path>",
# nonzero only) for effort attribution.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    q = "[(function_declaration) (method_definition) (arrow_function) (function_expression)] @f"
    total = 0
    per_file = []
    for tri in _ts_files():
        c = len(ast_query(tri[1], tri[2], q))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + tri[0], "dims": {"language": "typescript"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "typescript"}}] + per_file}
