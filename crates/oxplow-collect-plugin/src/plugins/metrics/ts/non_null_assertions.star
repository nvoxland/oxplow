# oxplow.ts.non_null_assertions — count `expr!` non-null assertions (a
# type-checker override that can hide real nullability bugs).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["text"], "tsx"))
    return out

def transform(input):
    n = 0
    for pair in _ts_files():
        n += len(ast_query(pair[0], pair[1], "(non_null_expression) @n"))
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
