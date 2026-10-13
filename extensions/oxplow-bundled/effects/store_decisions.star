# oxplow-bundled/store-decisions: a `decisions` job's answer becomes the
# effort's inferred decisions (replacing any it had). The job's intent key
# (`decisions:<effort ref>`, `…@<n>` when asked again) says which effort. A reply that isn't the
# JSON asked for is skipped, saying so: a model that writes prose isn't a
# failure of this effect.

PREFIX = "decisions:"
MAX = 8

def text_of(field):
    return field.strip() if type(field) == "string" else ""

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
    job = scope("jobs.read", {"id": payload["job_id"]})
    key = job.get("intentKey") or ""
    if not key.startswith(PREFIX):
        return {"skip": "not an effort's decisions: " + key}
    answer = job.get("result") or {}
    reply = "".join([p.get("text", "") for p in answer.get("parts", []) if p.get("type") == "text"])
    found = proposals(reply)
    if found == None or type(found.get("decisions")) != "list":
        return {"skip": "the model's reply wasn't the JSON asked for: " + reply[:200]}
    decisions = []
    for d in found["decisions"]:
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
    return {"commands": [{
        "name": "oxplow.effort.replace_inferred_decisions",
        "input": {"effort": key[len(PREFIX):].split("@")[0], "decisions": decisions},
    }]}
