# oxplow.ts.non_null_assertions — count `expr!` non-null assertions (a
# type-checker override that can hide real nullability bugs). Emits the
# repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero only),
# and a per-file `oxplow.ast_hit` FACT (rule="non_null_assertion") — the metric
# is the SPEC Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
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
    facts = []
    for tri in _ts_files():
        c = len(ast_query(tri[1], tri[2], "(non_null_expression) @n"))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + tri[0], "dims": {"language": "typescript"}})
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "non_null_assertion", "subject": "file:" + tri[0], "path": tri[0], "dims": {"language": "typescript"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "typescript"}}] + per_file, "facts": facts}
