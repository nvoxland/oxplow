# oxplow.rust.high_complexity_fns — number of Rust functions whose cyclomatic
# complexity exceeds 10 (the threshold the legacy code-quality scan flagged).
# Uses the code_metrics() host builtin (per-function complexity via tree-sitter).
# Emits the repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero
# only) for effort attribution, and a finding per offending function (drill-in).
def transform(input):
    total = 0
    per_file = []
    findings = []
    for f in files("**/*.rs"):
        c = 0
        for m in code_metrics(f["text"], "rust"):
            if m["complexity"] > 10:
                c += 1
                findings.append({"path": f["path"], "line": m["start_line"], "message": m["name"], "value": m["complexity"]})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file, "findings": findings}
