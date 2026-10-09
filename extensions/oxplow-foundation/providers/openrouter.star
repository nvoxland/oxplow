# OpenRouter: one key for many models over OpenAI-compatible chat; a Jev
# model answers typed questions natively (`/systemone`), any other as a chat.

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
    if x["op"] == "decide":
        if "jev" not in x["model"]:
            return None
        return {
            "path": "/systemone",
            "headers": {"authorization": "Bearer {{key}}"},
            "body": {"model": x["model"], "state": x["state"], "questions": _jev_questions(x["questions"])},
        }
    messages = []
    if x["system"] and x["system"].strip():
        messages.append({"role": "system", "content": x["system"]})
    messages.append({"role": "user", "content": x["prompt"]})
    body = {"model": x["model"], "messages": messages}
    if x["json"]:
        body["response_format"] = {"type": "json_object"}
    return {
        "path": "/chat/completions",
        "headers": {"authorization": "Bearer {{key}}"},
        "body": body,
    }

def response(x):
    b = x["body"]
    usage = b.get("usage") or {}
    if x["op"] == "decide":
        return {
            "answers": _jev_answers(b.get("answers")),
            "usage": {"input": usage.get("input_tokens", 0), "output": usage.get("output_tokens", 0)},
        }
    choices = b.get("choices")
    if type(choices) != "list" or not choices or type(choices[0].get("message")) != "dict":
        return {"error": "no choices[0].message.content"}
    text = choices[0]["message"].get("content")
    if type(text) != "string":
        return {"error": "no choices[0].message.content"}
    return {
        "text": text,
        "usage": {"input": usage.get("prompt_tokens", 0), "output": usage.get("completion_tokens", 0)},
    }
