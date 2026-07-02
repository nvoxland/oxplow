# oxplow.high_complexity_fns — functions whose cyclomatic complexity exceeds 10,
# across EVERY supported language. Language-agnostic: sweeps source_files() and
# uses the code_metrics() capability (per-function metrics from the language
# layer).
#
# Inverted substrate (epic tsk12): the durable grain is a per-function
# `oxplow.complexity` FACT (emitted for EVERY function, not just offenders — so a
# spec can re-threshold), and the "> 10" count is the metric SPEC
# `oxplow.high_complexity_fns` computed over those facts. Facts are the only
# output; the reads aggregate them via the engine (the read flip, T-C3).
def transform(input):
    facts = []
    for f in source_files():
        lang = f["language"]
        for m in code_metrics(f["text"], lang):
            facts.append({"measure": "oxplow.complexity", "value": m["complexity"], "subject": "symbol:" + f["path"] + "::" + m["name"], "path": f["path"], "line": m["start_line"], "dims": {"oxplow.language": lang}})
    return {"facts": facts}
