# oxplow.ts.high_complexity_fns — number of TS/TSX functions whose cyclomatic
# complexity exceeds 10. Uses the code_metrics() host builtin per file.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["text"], "tsx"))
    return out

def transform(input):
    n = 0
    for pair in _ts_files():
        for m in code_metrics(pair[0], pair[1]):
            if m["complexity"] > 10:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "typescript"}}]}
