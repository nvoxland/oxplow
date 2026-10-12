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
    return _reply(parts, body.get("usage") or {}, body.get("stop_reason"))

# The V1 reply for `parts`, from Anthropic's usage object and stop reason.
def _reply(parts, usage, finish):
    input_tokens = _count(usage, "input_tokens")
    # Anthropic reports cache reads/writes separately from uncached input.
    if input_tokens != None:
        input_tokens += _count(usage, "cache_creation_input_tokens") or 0
        input_tokens += _count(usage, "cache_read_input_tokens") or 0
    output_tokens = _count(usage, "output_tokens")
    reasons = {"end_turn": "stop", "stop_sequence": "stop", "max_tokens": "length", "tool_use": "toolCalls"}
    return {"schemaVersion": 1, "parts": parts,
            "finishReason": reasons.get(finish, "unknown" if finish == None else "other"),
            "usage": {"inputTokens": input_tokens, "outputTokens": output_tokens,
                      "source": "observed" if input_tokens != None or output_tokens != None else "unknown"}}

def request(x):
    if x["op"] == "completeTyped":
        return _typed_request(x)
    if x["op"] == "streamTyped":
        call = _typed_request(x)
        call["body"]["stream"] = True
        return call
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
    if x["op"] == "streamTyped":
        return _streamed_response(x["state"])
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

# Streaming (`stream: true`): the Messages API's events folded into
# `state` = { parts, buffers, usage, stop, done }. A tool call's input
# arrives as JSON fragments, buffered per content block until its block
# stops.

def delta(x):
    state = x["state"] or {"parts": [], "buffers": {}, "usage": {}, "stop": None, "done": False}
    parts = list(state["parts"])
    buffers = dict(state["buffers"])
    usage = dict(state["usage"])
    stop = state["stop"]
    done = state["done"]
    out = []
    for e in x["events"]:
        d = json.decode(e["data"])
        kind = d.get("type")
        if kind == "error":
            return {"error": (d.get("error") or {}).get("message") or "the provider's stream failed"}
        elif kind == "message_start":
            usage.update((d.get("message") or {}).get("usage") or {})
        elif kind == "content_block_start":
            block = d["content_block"]
            if block.get("type") == "text":
                parts.append({"type": "text", "text": block.get("text") or ""})
            elif block.get("type") == "tool_use" and type(block.get("id")) == "string" and type(block.get("name")) == "string":
                parts.append({"type": "toolCall", "id": block["id"], "name": block["name"], "arguments": None})
                buffers[str(d["index"])] = ""
            else:
                return {"error": "unsupported assistant response part"}
        elif kind == "content_block_delta":
            at = d["index"]
            fragment = d["delta"]
            if fragment.get("type") == "text_delta" and parts[at]["type"] == "text":
                parts[at] = {"type": "text", "text": parts[at]["text"] + fragment["text"]}
                out.append({"type": "text", "text": fragment["text"]})
            elif fragment.get("type") == "input_json_delta" and str(at) in buffers:
                buffers[str(at)] += fragment.get("partial_json") or ""
            else:
                return {"error": "unsupported assistant response part"}
        elif kind == "content_block_stop":
            at = d["index"]
            if str(at) in buffers:
                call = parts[at]
                arguments = json.decode(buffers.pop(str(at)) or "{}")
                if type(arguments) != "dict":
                    return {"error": "Anthropic tool arguments require an object"}
                parts[at] = {"type": "toolCall", "id": call["id"], "name": call["name"], "arguments": arguments}
                out.append(parts[at])
        elif kind == "message_delta":
            stop = (d.get("delta") or {}).get("stop_reason") or stop
            usage.update(d.get("usage") or {})
        elif kind == "message_stop":
            done = True
    return {"deltas": out, "state": {"parts": parts, "buffers": buffers, "usage": usage, "stop": stop, "done": done}}

def _streamed_response(state):
    if not state or not state["done"] or state["buffers"]:
        return {"error": "the stream ended before the message was whole"}
    return _reply(state["parts"], state["usage"], state["stop"])
