# Any OpenAI-compatible chat completions API: OpenAI itself, and local
# servers (Ollama, LM Studio, vLLM, LiteLLM).

def request(x):
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
    choices = b.get("choices")
    if type(choices) != "list" or not choices or type(choices[0].get("message")) != "dict":
        return {"error": "no choices[0].message.content"}
    text = choices[0]["message"].get("content")
    if type(text) != "string":
        return {"error": "no choices[0].message.content"}
    usage = b.get("usage") or {}
    return {
        "text": text,
        "usage": {"input": usage.get("prompt_tokens", 0), "output": usage.get("completion_tokens", 0)},
    }
