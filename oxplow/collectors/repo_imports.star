# repo.file_imports — `import` statements per TS file (module coupling / fan-out).
# Feeds repo.total_imports (sum) and repo.max_file_imports (max — a coupling
# hotspot). One ast_query pass over the .ts/.tsx tree.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(import_statement) @i"
    for tri in _ts_files():
        c = len(ast_query(tri[1], tri[2], q))
        if c > 0:
            facts.append({"measure": "repo.file_imports", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
