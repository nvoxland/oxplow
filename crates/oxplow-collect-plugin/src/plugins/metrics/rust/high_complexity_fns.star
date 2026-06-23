# oxplow.rust.high_complexity_fns — number of Rust functions whose cyclomatic
# complexity exceeds 10 (the threshold the legacy code-quality scan flagged).
# Uses the code_metrics() host builtin (per-function complexity via tree-sitter).
# Emits the repo-total ("tree:.") plus a per-file sample ("file:<path>", nonzero
# only) for effort attribution.
def transform(input):
    total = 0
    per_file = []
    for f in files("**/*.rs"):
        c = 0
        for m in code_metrics(f["text"], "rust"):
            if m["complexity"] > 10:
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file}
