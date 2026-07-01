# oxplow.todos — TODO/FIXME/HACK/XXX/BUG markers in comments, across EVERY
# supported language. Language-agnostic: it sweeps source_files() (each file
# tagged with its language) and asks the markers() capability — the per-language
# knowledge lives in the language layer (oxplow-code-metrics), not here.
#
# Inverted substrate (epic tsk12): the durable grain is a per-marker
# `oxplow.todo` FACT (value 1, subject = the file), and the marker total is the
# metric SPEC `oxplow.todos` = count of those facts. The baked "tree:." total +
# per-file samples + per-marker findings are dual-written for the legacy read
# path until reads flip to the engine.
def transform(input):
    total = 0
    per_file = []
    findings = []
    facts = []
    for f in source_files():
        lang = f["language"]
        ms = markers(f["text"], lang)
        c = len(ms)
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
        for mk in ms:
            findings.append({"path": f["path"], "line": mk["line"], "message": mk["text"], "subject": "language:" + lang})
            facts.append({"measure": "oxplow.todo", "value": 1, "subject": "file:" + f["path"], "path": f["path"], "line": mk["line"], "dims": {"language": lang}})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file, "findings": findings, "facts": facts}
