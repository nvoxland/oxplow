# oxplow.long_functions — functions longer than 60 lines, across EVERY supported
# language. Language-agnostic: sweeps source_files() and uses the code_metrics()
# capability.
#
# Inverted substrate (epic tsk12): the durable grain is a per-function
# `oxplow.fn_length` FACT (emitted for EVERY function, so a spec can re-threshold),
# and the "> 60" count is the metric SPEC `oxplow.long_functions` over those facts.
# Facts are the only output; the reads aggregate them via the engine (T-C3).
def transform(input):
    facts = []
    for f in source_files():
        lang = f["language"]
        for m in code_metrics(f["text"], lang):
            facts.append({"measure": "oxplow.fn_length", "value": m["length"], "subject": "symbol:" + f["path"] + "::" + m["name"], "path": f["path"], "line": m["start_line"], "dims": {"language": lang}})
    return {"facts": facts}
