# Anthropic's Messages API (`.context/ai-providers.md` "Scripted providers").

def _typed_request(x):
    request = x["request"]
    system = []
    messages = []
    for message in request["messages"]:
        role = message["role"]
        content = []
        if role == "system":
            if messages:
                fail("system messages must precede conversation messages")
            for part in message["parts"]:
                if part["type"] != "text":
                    fail("system messages accept only text")
                system.append({"type": "text", "text": part["text"]})
            continue
        saw_text = False
        for part in message["parts"]:
            kind = part["type"]
            if kind == "text" and role != "tool":
                content.append({"type": "text", "text": part["text"]})
                saw_text = True
            elif kind == "toolCall" and role == "assistant":
                if type(part["arguments"]) != "dict":
                    fail("Anthropic tool arguments require an object")
                content.append({"type": "tool_use", "id": part["id"],
                                "name": part["name"], "input": part["arguments"]})
            elif kind == "toolResult" and role in ["tool", "user"]:
                if saw_text:
                    fail("tool results must precede user text")
                content.append({"type": "tool_result", "tool_use_id": part["callId"],
                                "content": json.encode(part["result"])})
            else:
                fail("unsupported typed message part or role")
        if not content:
            fail("conversation messages require content")
        messages.append({"role": "user" if role == "tool" else role, "content": content})
    body = {"model": x["model"], "max_tokens": 4096, "messages": messages}
    if system:
        body["system"] = system
    tools = []
    for tool in request.get("tools", []):
        out = {"name": tool["name"], "input_schema": tool["parameters"]}
        if tool.get("description") != None:
            out["description"] = tool["description"]
        tools.append(out)
    if tools:
        body["tools"] = tools
    return {"path": "/v1/messages",
            "headers": {"anthropic-version": "2023-06-01", "x-api-key": "{{key}}"}, "body": body}

def _count(usage, key):
    value = usage.get(key)
    if value != None and (type(value) != "int" or value < 0):
        fail("invalid token usage")
    return value

def _typed_response(body):
    content = body.get("content")
    if type(content) != "list":
        return {"error": "no assistant content"}
    parts = []
    for part in content:
        if type(part) != "dict":
            return {"error": "invalid assistant part"}
        kind = part.get("type")
        if kind == "text" and type(part.get("text")) == "string":
            parts.append({"type": "text", "text": part["text"]})
        elif kind == "tool_use" and type(part.get("id")) == "string" and type(part.get("name")) == "string" and type(part.get("input")) == "dict":
            parts.append({"type": "toolCall", "id": part["id"], "name": part["name"], "arguments": part["input"]})
        else:
            return {"error": "unsupported assistant response part"}
    usage = body.get("usage") or {}
    input_tokens = _count(usage, "input_tokens")
    # Anthropic reports cache reads/writes separately from uncached input.
    if input_tokens != None:
        input_tokens += _count(usage, "cache_creation_input_tokens") or 0
        input_tokens += _count(usage, "cache_read_input_tokens") or 0
    output_tokens = _count(usage, "output_tokens")
    finish = body.get("stop_reason")
    reasons = {"end_turn": "stop", "stop_sequence": "stop", "max_tokens": "length", "tool_use": "toolCalls"}
    return {"schemaVersion": 1, "parts": parts,
            "finishReason": reasons.get(finish, "unknown" if finish == None else "other"),
            "usage": {"inputTokens": input_tokens, "outputTokens": output_tokens,
                      "source": "observed" if input_tokens != None or output_tokens != None else "unknown"}}

def request(x):
    if x["op"] == "completeTyped":
        return _typed_request(x)
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
    if x["op"] == "completeTyped":
        return _typed_response(x["body"])
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
