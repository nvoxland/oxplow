# oxplow.ts.ts_ignore — count `@ts-ignore` / `@ts-expect-error` suppression
# directives (type-error escape hatches). Scoped to comment nodes. Emits the
# repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero only).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    total = 0
    per_file = []
    for tri in _ts_files():
        c = 0
        for cm in ast_query(tri[1], tri[2], "(comment) @c"):
            c += len(regex_find(r"@ts-(ignore|expect-error)", cm["text"]))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + tri[0], "dims": {"language": "typescript"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "typescript"}}] + per_file}
