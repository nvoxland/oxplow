# oxplow.fn_count — total functions/methods defined, across EVERY supported
# language. Language-agnostic: sweeps source_files() and uses the code_metrics()
# capability.
#
# Inverted substrate (epic tsk12): the durable grain is a per-function
# `oxplow.parameter_count` FACT (one per function — its parameter count), and the
# function total is the metric SPEC `oxplow.fn_count` = count of those facts. The
# baked "tree:." total + per-file samples are dual-written for the legacy read
# path until reads flip to the engine.
def transform(input):
    total = 0
    per_file = []
    facts = []
    for f in source_files():
        lang = f["language"]
        c = 0
        for m in code_metrics(f["text"], lang):
            c += 1
            facts.append({"measure": "oxplow.parameter_count", "value": m["parameter_count"], "subject": "symbol:" + f["path"] + "::" + m["name"], "path": f["path"], "line": m["start_line"], "dims": {"language": lang}})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file, "facts": facts}
