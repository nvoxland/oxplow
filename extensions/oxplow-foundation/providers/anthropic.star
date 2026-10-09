# Anthropic's Messages API (`.context/ai-providers.md` "Scripted providers").

def request(x):
    system = x["system"] or ""
    if x["json"]:
        system = system + "\n\nReply with a single JSON object and nothing else."
    body = {
        "model": x["model"],
        "max_tokens": 4096,
        "messages": [{"role": "user", "content": x["prompt"]}],
    }
    if system.strip():
        body["system"] = system.strip()
    return {
        "path": "/v1/messages",
        "headers": {"anthropic-version": "2023-06-01", "x-api-key": "{{key}}"},
        "body": body,
    }

def response(x):
    b = x["body"]
    content = b.get("content")
    if type(content) != "list":
        return {"error": "no content"}
    text = "".join([p["text"] for p in content if type(p) == "dict" and "text" in p])
    usage = b.get("usage") or {}
    return {
        "text": text,
        "usage": {"input": usage.get("input_tokens", 0), "output": usage.get("output_tokens", 0)},
    }
