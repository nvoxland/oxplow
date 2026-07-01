# oxplow.high_complexity_fns — functions whose cyclomatic complexity exceeds 10,
# across EVERY supported language. Language-agnostic: sweeps source_files() and
# uses the code_metrics() capability (per-function metrics from the language
# layer).
#
# Inverted substrate (epic tsk12): the durable grain is a per-function
# `oxplow.complexity` FACT (emitted for EVERY function, not just offenders — so a
# spec can re-threshold), and the "> 10" count is the metric SPEC
# `oxplow.high_complexity_fns` computed over those facts. The baked "tree:." total
# + per-file samples + offender findings are dual-written for the legacy read
# path until reads flip to the engine.
def transform(input):
    total = 0
    per_file = []
    findings = []
    facts = []
    for f in source_files():
        lang = f["language"]
        c = 0
        for m in code_metrics(f["text"], lang):
            facts.append({"measure": "oxplow.complexity", "value": m["complexity"], "subject": "symbol:" + f["path"] + "::" + m["name"], "path": f["path"], "line": m["start_line"], "dims": {"language": lang}})
            if m["complexity"] > 10:
                c += 1
                findings.append({"path": f["path"], "line": m["start_line"], "message": m["name"], "value": m["complexity"], "subject": "language:" + lang})
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": lang}})
    return {"samples": [{"value": total, "subject": "tree:."}] + per_file, "findings": findings, "facts": facts}
