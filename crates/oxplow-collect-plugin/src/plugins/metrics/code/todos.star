# oxplow.todos — TODO/FIXME/HACK/XXX/BUG markers in comments, across EVERY
# supported language. Language-agnostic: it sweeps source_files() (each file
# tagged with its language) and asks the markers() capability — the per-language
# knowledge lives in the language layer (oxplow-code-metrics), not here. Emits
# the repo-total ("tree:.") + per-file samples (dims.language) for attribution,
# and a finding per marker (language-tagged) for drill-in.
def transform(input):
    total = 0
    per_file = []
    findings = []
    for f in source_files():
        lang = f["language"]
        ms = markers(f["text"], lang)
        c = len(ms)
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
        for mk in ms:
            findings.append({"path": f["path"], "line": mk["line"], "message": mk["text"], "subject": "language:" + lang})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file, "findings": findings}
