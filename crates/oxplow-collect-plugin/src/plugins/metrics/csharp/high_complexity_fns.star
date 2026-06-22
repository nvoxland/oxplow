# oxplow.csharp.high_complexity_fns — number of C# functions whose cyclomatic
# complexity exceeds 10. Uses the code_metrics() host builtin (per-function
# complexity via tree-sitter), the same threshold the Rust/TS gauges flag.
def transform(input):
    n = 0
    for f in files("**/*.cs"):
        for m in code_metrics(f["text"], "csharp"):
            if m["complexity"] > 10:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "csharp"}}]}
