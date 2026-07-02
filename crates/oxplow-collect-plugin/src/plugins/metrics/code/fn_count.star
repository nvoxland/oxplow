# oxplow.fn_count — total functions/methods defined, across EVERY supported
# language. Language-agnostic: sweeps source_files() and uses the code_metrics()
# capability.
#
# Inverted substrate (epic tsk12): the durable grain is a per-function
# `oxplow.parameter_count` FACT (one per function — its parameter count), and the
# function total is the metric SPEC `oxplow.fn_count` = count of those facts.
# Facts are the only output; the reads aggregate them via the engine (T-C3).
def transform(input):
    facts = []
    for f in source_files():
        lang = f["language"]
        for m in code_metrics(f["text"], lang):
            facts.append({"measure": "oxplow.parameter_count", "value": m["parameter_count"], "subject": "symbol:" + f["path"] + "::" + m["name"], "path": f["path"], "line": m["start_line"], "dims": {"oxplow.language": lang}})
    return {"facts": facts}
