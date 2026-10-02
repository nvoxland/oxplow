# repo.hardcoded_colors — hex/rgb color literals in TS/TSX (should be a
# var(--…), see .context/theming.md). Re-added as a single-concern per-path
# gauge (tsk61 — the original multi-concern version was dropped in the tsk42
# split and its catalog rows lingered as zombies). Scans string + template
# nodes only, so the regex runs on tiny texts.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    pat = r"#[0-9a-fA-F]{6}\b|#[0-9a-fA-F]{3}\b|rgba?\("
    q = "(string) @s (template_string) @s"
    for tri in _ts_files():
        c = 0
        for s in ast_query(tri[1], tri[2], q):
            c += len(regex_find(pat, s["text"]))
        if c > 0:
            facts.append({"measure": "repo.hardcoded_colors", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
