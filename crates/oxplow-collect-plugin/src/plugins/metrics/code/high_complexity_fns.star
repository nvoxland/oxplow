# oxplow.high_complexity_fns — functions whose cyclomatic complexity exceeds 10,
# across EVERY supported language. Language-agnostic: sweeps source_files() and
# uses the code_metrics() capability (per-function metrics from the language
# layer). Emits the repo-total ("tree:.") + per-file samples (dims.language) and
# a finding per offending function (language-tagged) for drill-in.
def transform(input):
    total = 0
    per_file = []
    findings = []
    for f in source_files():
        lang = f["language"]
        c = 0
        for m in code_metrics(f["text"], lang):
            if m["complexity"] > 10:
                c += 1
                findings.append({"path": f["path"], "line": m["start_line"], "message": m["name"], "value": m["complexity"], "subject": "language:" + lang})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file, "findings": findings}
