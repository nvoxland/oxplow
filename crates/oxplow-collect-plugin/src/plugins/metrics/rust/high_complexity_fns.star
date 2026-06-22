# oxplow.rust.high_complexity_fns — number of Rust functions whose cyclomatic
# complexity exceeds 10 (the threshold the legacy code-quality scan flagged).
# Uses the code_metrics() host builtin (per-function complexity via tree-sitter).
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        for m in code_metrics(f["text"], "rust"):
            if m["complexity"] > 10:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
