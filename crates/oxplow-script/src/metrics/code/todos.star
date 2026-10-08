# oxplow.todos — TODO/FIXME/HACK/XXX/BUG markers in comments, across EVERY
# supported language. Language-agnostic: it sweeps source_files() (each file
# tagged with its language) and asks the markers() capability — the per-language
# knowledge lives in the language layer (oxplow-code-metrics), not here.
#
# Inverted substrate (epic tsk12): the durable grain is a per-marker
# `oxplow.todo` FACT (value 1, subject = the file), and the marker total is the
# metric SPEC `oxplow.todos` = count of those facts. Facts are the only output;
# the reads aggregate them via the engine (the read flip, T-C3).
def transform(input):
    facts = []
    for f in source_files():
        lang = f["language"]
        for mk in markers(f["text"], lang):
            facts.append({"measure": "oxplow.todo", "value": 1, "subject": "file:" + f["path"], "path": f["path"], "line": mk["line"], "dims": {"oxplow.language": lang}})
    return {"facts": facts}
