# oxplow.csharp.high_complexity_fns — number of C# functions whose cyclomatic
# complexity exceeds 10. Uses the code_metrics() host builtin (per-function
# complexity via tree-sitter), the same threshold the Rust/TS gauges flag.
# Emits the repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero
# only) for effort attribution.
def transform(input):
    total = 0
    per_file = []
    findings = []
    for f in files("**/*.cs"):
        c = 0
        for m in code_metrics(f["text"], "csharp"):
            if m["complexity"] > 10:
                c += 1
                findings.append({"path": f["path"], "line": m["start_line"], "message": m["name"], "value": m["complexity"]})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "csharp"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "csharp"}}] + per_file, "findings": findings}
