# oxplow-bundled/store-reasoning: a `reasoning` job's answer becomes the
# effort's inferred decisions and claims (replacing any it had; a claim a
# reviewer acted on stays, core keeps it). The job's intent key
# (`reasoning:<effort id>`, as `reasoning:eff3`, `…@<n>` when asked again) says which effort.
# A reply that isn't the JSON asked for (an object with a `decisions` or
# a `claims` list) is skipped, saying so: a model that writes prose isn't
# a failure of this effect. Both writers run even with an empty list, so a
# stale one is cleared.

PREFIX = "reasoning:"
MAX = 8
KINDS = ("tests_pass", "no_behavior_change", "handles_case", "other")

def text_of(field):
    return field.strip() if type(field) == "string" else ""

def items(found, name):
    """The list under `name`, or none when it is missing or not a list."""
    return found[name] if type(found.get(name)) == "list" else []

def proposals(reply):
    """The JSON object in `reply` (a code fence around it is dropped), or None."""
    text = reply.strip()
    if text.startswith("```"):
        text = text[text.find("\n") + 1:] if "\n" in text else ""
        text = text[:text.rfind("```")] if "```" in text else text
        text = text.strip()
    if not (text.startswith("{") and text.endswith("}")):
        return None
    return json.decode(text)

def transform(x):
    payload = x["event"]["payload"]
    if payload["outcome"] != "succeeded":
        return {"skip": "the job didn't succeed (" + payload["outcome"] + ")"}
    run = scope("jobs.read", {"job_run": payload["job_run"]})
    key = run.get("intentKey") or ""
    if not key.startswith(PREFIX):
        return {"skip": "not an effort's reasoning: " + key}
    answer = run.get("result") or {}
    reply = "".join([p.get("text", "") for p in answer.get("parts", []) if p.get("type") == "text"])
    found = proposals(reply)
    if found == None or type(found) != "dict" or (type(found.get("decisions")) != "list" and type(found.get("claims")) != "list"):
        return {"skip": "the model's reply wasn't the JSON asked for: " + reply[:200]}
    decisions = []
    for d in items(found, "decisions"):
        if type(d) != "dict":
            continue
        question, choice = text_of(d.get("question")), text_of(d.get("choice"))
        if not question or not choice:
            continue
        alternatives = d.get("alternatives")
        confidence = text_of(d.get("confidence")).lower()
        decision = {
            "question": question[:1000],
            "choice": choice[:1000],
            "alternatives": [a[:1000] for a in alternatives if type(a) == "string"][:16] if type(alternatives) == "list" else [],
            "why": text_of(d.get("why"))[:4000],
        }
        if confidence in ("low", "medium", "high"):
            decision["confidence"] = confidence
        decisions.append(decision)
        if len(decisions) == MAX:
            break
    claims = []
    for c in items(found, "claims"):
        if type(c) != "dict":
            continue
        statement = text_of(c.get("statement"))
        if not statement:
            continue
        kind = text_of(c.get("kind")).lower()
        claim = {"statement": statement[:1000], "kind": kind if kind in KINDS else "other"}
        evidence = text_of(c.get("evidence_ref"))
        if evidence:
            claim["evidence_ref"] = evidence[:400]
        claims.append(claim)
        if len(claims) == MAX:
            break
    effort = key[len(PREFIX):].split("@")[0]
    return {"commands": [
        {"name": "oxplow.effort.replace_inferred_decisions", "input": {"effort": effort, "decisions": decisions}},
        {"name": "oxplow.effort.replace_inferred_claims", "input": {"effort": effort, "claims": claims}},
    ]}
