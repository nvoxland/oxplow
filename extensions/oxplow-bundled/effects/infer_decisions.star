# oxplow-bundled/infer-decisions: a finished effort's unrecorded decisions
# are asked about (`effort.infer_decisions` builds the request). With no
# model to ask, it skips and says why — nothing is queued to fail.

def transform(x):
    found = scope("ai.discover", {"role": "decisions"})
    if not found.get("model"):
        return {"skip": (found.get("problem") or "no model serves " + found["role"])
                        + " — bind `" + found["role"] + "` or Main in Settings → AI"}
    return {"commands": [{
        "name": "oxplow.effort.infer_decisions",
        "input": {"effort": x["event"]["payload"]["effort"]},
    }]}
