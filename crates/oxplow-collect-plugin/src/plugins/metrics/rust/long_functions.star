# oxplow.rust.long_functions — number of Rust functions longer than 60 lines
# (a maintainability signal the legacy code-quality scan tracked). Function
# length comes from the code_metrics() host builtin. Emits the repo-total
# ("tree:.") plus a per-file sample ("file:<path>", nonzero only).
def transform(input):
    total = 0
    per_file = []
    findings = []
    for f in files("**/*.rs"):
        c = 0
        for m in code_metrics(f["text"], "rust"):
            if m["length"] > 60:
                c += 1
                findings.append({"path": f["path"], "line": m["start_line"], "message": m["name"], "value": m["length"]})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file, "findings": findings}
