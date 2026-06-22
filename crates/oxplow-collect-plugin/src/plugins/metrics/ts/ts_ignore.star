# oxplow.ts.ts_ignore — count `@ts-ignore` / `@ts-expect-error` suppression
# directives (type-error escape hatches). Scoped to comment nodes.
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
        for c in ast_query(pair[0], pair[1], "(comment) @c"):
            n += len(regex_find(r"@ts-(ignore|expect-error)", c["text"]))
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
