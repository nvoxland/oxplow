# The notes server's mapping: task tracker verbs as its tools, its notes as
# tasks. `x.phase` is invoke / invoked (a verb) or read / records (a
# collector); `x.output` is the tool's output, data to read.

# A note's state is its stage: the manifest declares them (`stages:`), and
# core resolves a bucket to one of them before the mapping sees it.

def ref(x, id):
    return uri.build("oxplow/task", provider = x["provider"], id = id)

def note_id(x, r):
    if not uri.is_uri(r) or uri.resource(r) != "oxplow/task":
        return None
    if uri.param(r, "provider") != x["provider"]:
        return None
    return uri.param(r, "id")

def record(x, note):
    return {
        "uri": ref(x, note["id"]),
        "title": note["title"],
        "body": note["body"],
        "stage": note["state"],
        "native": {},
        "parent_uri": ref(x, note["parent"]) if note["parent"] else None,
        "deleted": False,
    }

def refuse(field, message):
    return {"refuse": {"field": field, "message": message}}

def tool_call(x):
    input = x["input"]
    args = {}
    if x["command"] == "create":
        args["title"] = input["title"]
        if input.get("body"):
            args["body"] = input["body"]
    else:
        id = note_id(x, input["uri"])
        if not id:
            return refuse("/uri", "`%s` isn't a note" % input["uri"])
        args["id"] = id
        for field in ["title", "body"]:
            if input.get(field) != None:
                args[field] = input[field]
    parent = input.get("parent_uri")
    if parent != None:
        if parent == "":
            args["parent"] = ""
        else:
            pid = note_id(x, parent)
            if not pid:
                return refuse("/parent_uri", "`%s` isn't a note" % parent)
            args["parent"] = pid
    stage = input.get("to" if x["command"] == "transition" else "stage")
    if stage:
        args["state"] = stage
    return {"tool": "create_item" if x["command"] == "create" else "update_item", "arguments": args}

# The note argument a tool error names, as the verb's input field.
FIELDS = {"id": "/uri", "parent": "/parent_uri", "state": "/stage"}

def transform(x):
    phase = x["phase"]
    if phase == "invoke":
        return tool_call(x)
    if x.get("error"):
        return refuse(FIELDS.get(x["output"]["field"], ""), x["output"]["error"])
    if phase == "invoked":
        item = record(x, x["output"])
        return {
            "result": {"uri": item["uri"]},
            "events": [{"type": "task.recorded", "v": 3, "payload": {"item": item}, "subject": [item["uri"]]}],
        }
    if phase == "read":
        state = x["state"] or {}
        return {"tool": "list_items", "arguments": {"since": state.get("cursor", 0)}}
    return {
        "records": [record(x, n) for n in x["output"]["items"]],
        "state": {"cursor": x["output"]["cursor"]},
    }
