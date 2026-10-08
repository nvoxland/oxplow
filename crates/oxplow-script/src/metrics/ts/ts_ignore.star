# oxplow.ts.ts_ignore — count `@ts-ignore` / `@ts-expect-error` suppression
# directives (type-error escape hatches). Scoped to comment nodes. Its facts are per-file
# (subject "file:<path>", nonzero only), so an effort's change attributes
# through its files.
# A per-file `oxplow.ast_hit` FACT (rule="ts_ignore") — the metric is the
# SPEC Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
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
        c = 0
        for cm in ast_query(tri[1], tri[2], "(comment) @c"):
            c += len(regex_find(r"@ts-(ignore|expect-error)", cm["text"]))
        if c > 0:
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "ts_ignore", "subject": "file:" + tri[0], "path": tri[0], "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
