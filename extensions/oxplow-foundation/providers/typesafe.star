# TypeSafe: Jev's typed decisions (`/v1/systemone`). It only answers typed
# questions; it has no chat.

def _jev_questions(questions):
    out = {}
    for name, q in questions.items():
        if q["type"] == "choice":
            out[name] = {"type": "choice", "instructions": q["instructions"],
                         "criteria": {o: None for o in q["options"]}}
        elif q["type"] == "score":
            out[name] = {"type": "score", "instructions": q["instructions"], "criteria": q["levels"]}
        else:
            out[name] = {"type": "noul", "instructions": q["instructions"]}
    return out

def _jev_answers(answers):
    out = {}
    for name, a in (answers or {}).items():
        if a.get("type") == "noul":
            out[name] = {"type": "noul", "probability": a.get("noul")}
        else:
            out[name] = a
    return out

def request(x):
    return {
        "path": "/v1/systemone",
        "headers": {"authorization": "Bearer {{key}}"},
        "body": {"model": x["model"], "state": x["state"], "questions": _jev_questions(x["questions"])},
    }

def response(x):
    b = x["body"]
    usage = b.get("usage") or {}
    return {
        "answers": _jev_answers(b.get("answers")),
        "usage": {"input": usage.get("input_tokens", 0), "output": usage.get("output_tokens", 0)},
    }
