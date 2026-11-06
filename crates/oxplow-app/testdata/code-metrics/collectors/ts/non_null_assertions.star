# code_metrics.ts.non_null_assertions — count `expr!` non-null assertions (a
# type-checker override that can hide real nullability bugs). Its facts are per-file
# (subject "file:<path>", nonzero only), so an effort's change attributes
# through its files.
# A per-file `code_metrics.ast_hit` FACT (rule="non_null_assertion") — the metric
# is the SPEC Sum(code_metrics.ast_hit) filtered by that rule (epic tsk12).
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    for tri in _ts_files():
        c = len(ast_query(tri[1], tri[2], "(non_null_expression) @n"))
        if c > 0:
            facts.append({"measure": "code_metrics.ast_hit", "value": c, "rule": "non_null_assertion", "subject": uri.build("oxplow/file", path = tri[0]), "path": tri[0], "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
