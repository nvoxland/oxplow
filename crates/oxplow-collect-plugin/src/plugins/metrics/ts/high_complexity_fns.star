# oxplow.ts.high_complexity_fns — number of TS/TSX functions whose cyclomatic
# complexity exceeds 10. Uses the code_metrics() host builtin per file. Emits
# the repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero only)
# for effort attribution.
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
    findings = []
    for tri in _ts_files():
        c = 0
        for m in code_metrics(tri[1], tri[2]):
            if m["complexity"] > 10:
                c += 1
                findings.append({"path": tri[0], "line": m["start_line"], "message": m["name"], "value": m["complexity"]})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + tri[0], "dims": {"language": "typescript"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "typescript"}}] + per_file, "findings": findings}
