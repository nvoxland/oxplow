# oxplow.ts.fn_count — total function definitions across the TS/TSX tree
# (declarations, methods, arrow + function expressions) — a coarse size gauge.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["text"], "tsx"))
    return out

def transform(input):
    q = "[(function_declaration) (method_definition) (arrow_function) (function_expression)] @f"
    n = 0
    for pair in _ts_files():
        n += len(ast_query(pair[0], pair[1], q))
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
