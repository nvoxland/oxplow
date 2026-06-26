# oxplow.long_functions — functions longer than 60 lines, across EVERY supported
# language. Language-agnostic: sweeps source_files() and uses the code_metrics()
# capability. Emits the repo-total ("tree:.") + per-file samples (dims.language)
# and a finding per long function (language-tagged) for drill-in.
def transform(input):
    total = 0
    per_file = []
    findings = []
    for f in source_files():
        lang = f["language"]
        c = 0
        for m in code_metrics(f["text"], lang):
            if m["length"] > 60:
                c += 1
                findings.append({"path": f["path"], "line": m["start_line"], "message": m["name"], "value": m["length"], "subject": "language:" + lang})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file, "findings": findings}
