# oxplow/infer-reasoning: a finished effort's unrecorded decisions
# and the claims its answers made are asked about (`effort.infer_reasoning` builds the request). With no
# model to ask, it skips and says why — nothing is queued to fail.

def transform(x):
    found = scope("ai.discover", {"role": "reasoning"})
    if not found.get("model"):
        return {"skip": (found.get("problem") or "no model serves " + found["role"])
                        + " — bind `" + found["role"] + "` or Main in Settings → AI"}
    return {"commands": [{
        "name": "oxplow.effort.infer_reasoning",
        "input": {"effort": x["event"]["payload"]["effort"]},
    }]}
