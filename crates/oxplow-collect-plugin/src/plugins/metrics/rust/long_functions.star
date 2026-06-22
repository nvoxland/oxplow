# oxplow.rust.long_functions — number of Rust functions longer than 60 lines
# (a maintainability signal the legacy code-quality scan tracked). Function
# length comes from the code_metrics() host builtin.
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        for m in code_metrics(f["text"], "rust"):
            if m["length"] > 60:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
