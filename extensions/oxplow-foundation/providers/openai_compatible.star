# Any OpenAI-compatible chat completions API: OpenAI itself, and local
# servers (Ollama, LM Studio, vLLM, LiteLLM).

def _typed_messages(request):
    messages = []
    for message in request["messages"]:
        role = message["role"]
        content = []
        calls = []
        if role == "tool":
            if not message["parts"]:
                fail("tool message requires a result")
            for part in message["parts"]:
                if part["type"] != "toolResult":
                    fail("tool messages accept only tool results")
                messages.append({"role": "tool", "tool_call_id": part["callId"],
                                 "content": json.encode(part["result"])})
            continue
        for part in message["parts"]:
            kind = part["type"]
            if kind == "text":
                # Chat separates content from calls; refuse an order it cannot preserve.
                if calls:
                    fail("text after a tool call cannot be represented by chat completions")
                content.append({"type": "text", "text": part["text"]})
            elif kind == "toolCall" and role == "assistant":
                calls.append({"id": part["id"], "type": "function", "function": {
                    "name": part["name"], "arguments": json.encode(part["arguments"])}})
            else:
                fail("unsupported typed message part or role")
        out = {"role": role, "content": content if content else None}
        if calls:
            out["tool_calls"] = calls
        messages.append(out)
    return messages

def _typed_request(x):
    request = x["request"]
    body = {"model": x["model"], "messages": _typed_messages(request)}
    tools = []
    for tool in request.get("tools", []):
        function = {"name": tool["name"], "parameters": tool["parameters"]}
        if tool.get("description") != None:
            function["description"] = tool["description"]
        tools.append({"type": "function", "function": function})
    if tools:
        body["tools"] = tools
    return {"path": "/chat/completions",
            "headers": {"authorization": "Bearer {{key}}"}, "body": body}

def _count(usage, key):
    value = usage.get(key)
    if value != None and (type(value) != "int" or value < 0):
        fail("invalid token usage")
    return value

def _typed_response(body):
    choices = body.get("choices")
    if type(choices) != "list" or not choices or type(choices[0]) != "dict":
        return {"error": "no completion choice"}
    message = choices[0].get("message")
    if type(message) != "dict" or message.get("role") != "assistant":
        return {"error": "no assistant message"}
    if message.get("refusal") or message.get("audio") or message.get("function_call"):
        return {"error": "unsupported assistant response part"}
    parts = []
    content = message.get("content")
    if type(content) == "string":
        parts.append({"type": "text", "text": content})
    elif type(content) == "list":
        for part in content:
            if type(part) != "dict" or part.get("type") != "text" or type(part.get("text")) != "string":
                return {"error": "unsupported assistant content"}
            parts.append({"type": "text", "text": part["text"]})
    elif content != None:
        return {"error": "invalid assistant content"}
    calls = message.get("tool_calls") or []
    if type(calls) != "list":
        return {"error": "invalid tool calls"}
    for call in calls:
        if type(call) != "dict" or call.get("type") != "function" or type(call.get("function")) != "dict":
            return {"error": "unsupported tool call"}
        function = call["function"]
        if type(call.get("id")) != "string" or type(function.get("name")) != "string" or type(function.get("arguments")) != "string":
            return {"error": "invalid function call"}
        parts.append({"type": "toolCall", "id": call["id"], "name": function["name"],
                      "arguments": json.decode(function["arguments"])})
    return _reply(parts, body.get("usage") or {}, choices[0].get("finish_reason"))

# The V1 reply for `parts`, from the usage object and finish reason.
def _reply(parts, usage, finish):
    input_tokens = _count(usage, "prompt_tokens")
    output_tokens = _count(usage, "completion_tokens")
    reasons = {"stop": "stop", "length": "length", "tool_calls": "toolCalls"}
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
        call["body"]["stream_options"] = {"include_usage": True}
        return call
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
    if x["op"] == "completeTyped":
        return _typed_response(x["body"])
    if x["op"] == "streamTyped":
        return _streamed_response(x["state"])
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

# Streaming (`stream: true`): chat-completions chunks folded into
# `state` = { text, pending, calls, finish, usage }. Text is handed over
# as it comes; a tool call's arguments arrive as fragments per `index`
# and the calls are handed over, whole, when the finish reason arrives.

def delta(x):
    state = x["state"] or {"text": None, "pending": {}, "calls": [], "finish": None, "usage": {}}
    text = state["text"]
    pending = dict(state["pending"])
    calls = list(state["calls"])
    finish = state["finish"]
    usage = dict(state["usage"])
    out = []
    for e in x["events"]:
        if e["data"] == "[DONE]":
            continue
        chunk = json.decode(e["data"])
        if chunk.get("error"):
            return {"error": chunk["error"].get("message") or "the provider's stream failed"}
        usage.update(chunk.get("usage") or {})
        for choice in chunk.get("choices") or []:
            if choice.get("index", 0) != 0:
                continue
            piece = choice.get("delta") or {}
            if piece.get("refusal") or piece.get("audio") or piece.get("function_call"):
                return {"error": "unsupported assistant response part"}
            content = piece.get("content")
            if type(content) == "string" and content:
                text = (text or "") + content
                out.append({"type": "text", "text": content})
            elif content != None and type(content) != "string":
                return {"error": "invalid assistant content"}
            elif content == "" and text == None:
                text = ""
            for call in piece.get("tool_calls") or []:
                key = str(call["index"])
                held = pending.get(key) or {"id": None, "name": None, "arguments": ""}
                function = call.get("function") or {}
                pending[key] = {"id": call.get("id") or held["id"],
                                "name": function.get("name") or held["name"],
                                "arguments": held["arguments"] + (function.get("arguments") or "")}
            if choice.get("finish_reason") != None:
                finish = choice["finish_reason"]
                for key in sorted(pending.keys(), key = int):
                    held = pending[key]
                    if type(held["id"]) != "string" or type(held["name"]) != "string":
                        return {"error": "invalid function call"}
                    call = {"type": "toolCall", "id": held["id"], "name": held["name"],
                            "arguments": json.decode(held["arguments"] or "{}")}
                    calls.append(call)
                    out.append(call)
                pending = {}
    return {"deltas": out, "state": {"text": text, "pending": pending, "calls": calls, "finish": finish, "usage": usage}}

def _streamed_response(state):
    if not state or state["finish"] == None or state["pending"]:
        return {"error": "the stream ended before the message was whole"}
    parts = []
    if state["text"] != None:
        parts.append({"type": "text", "text": state["text"]})
    return _reply(parts + state["calls"], state["usage"], state["finish"])
