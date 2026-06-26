# oxplow.fn_count — total functions/methods defined, across EVERY supported
# language. Language-agnostic: sweeps source_files() and uses the code_metrics()
# capability. Emits the repo-total ("tree:.") + per-file samples (dims.language).
# No findings — listing every function is just the count, not a problem set.
def transform(input):
    total = 0
    per_file = []
    for f in source_files():
        lang = f["language"]
        c = len(code_metrics(f["text"], lang))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file}
