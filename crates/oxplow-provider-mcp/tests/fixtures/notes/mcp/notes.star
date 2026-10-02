# The notes server's mapping: work-items verbs as its tools, its notes as
# work items. `x.phase` is invoke / invoked (a verb) or read / records (a
# collector); `x.output` is the tool's output, data to read.

STATES = {"todo": "open", "in_progress": "doing", "blocked": "stuck", "done": "closed", "canceled": "dropped"}
CANONICAL = {v: k for k, v in STATES.items()}

def ref(x, id):
    return "work_item:" + x["provider"] + ":" + id

def note_id(x, r):
    prefix = "work_item:" + x["provider"] + ":"
    if not r.startswith(prefix):
        return None
    return r[len(prefix):]

def record(x, note):
    return {
        "ref": ref(x, note["id"]),
        "title": note["title"],
        "body": note["body"],
        "state": CANONICAL[note["state"]],
        "native_state": note["state"],
        "native": {},
        "parent_ref": ref(x, note["parent"]) if note["parent"] else None,
        "deleted": False,
    }

def refuse(field, message):
    return {"refuse": {"field": field, "message": message}}

# The note state `input` asks for (`field` canonical, `native_state` the
# note's), or a refusal when they disagree.
def state_of(input, field):
    to = input.get(field)
    native = input.get("native_state")
    if to and native and STATES[to] != native:
        return None, refuse("/native_state", "`%s` isn't %s" % (native, to))
    if to:
        return STATES[to], None
    if native and native not in CANONICAL:
        return None, refuse("/native_state", "`%s` isn't a note state" % native)
    return native, None

def tool_call(x):
    input = x["input"]
    args = {}
    if x["command"] == "create":
        args["title"] = input["title"]
        if input.get("body"):
            args["body"] = input["body"]
    else:
        id = note_id(x, input["ref"])
        if not id:
            return refuse("/ref", "`%s` isn't a note" % input["ref"])
        args["id"] = id
        for field in ["title", "body"]:
            if input.get(field) != None:
                args[field] = input[field]
    parent = input.get("parent_ref")
    if parent != None:
        if parent == "":
            args["parent"] = ""
        else:
            pid = note_id(x, parent)
            if not pid:
                return refuse("/parent_ref", "`%s` isn't a note" % parent)
            args["parent"] = pid
    state, refused = state_of(input, "to" if x["command"] == "transition" else "state")
    if refused:
        return refused
    if state:
        args["state"] = state
    return {"tool": "create_item" if x["command"] == "create" else "update_item", "arguments": args}

# The note argument a tool error names, as the verb's input field.
FIELDS = {"id": "/ref", "parent": "/parent_ref", "state": "/native_state"}

def transform(x):
    phase = x["phase"]
    if phase == "invoke":
        return tool_call(x)
    if x.get("error"):
        return refuse(FIELDS.get(x["output"]["field"], ""), x["output"]["error"])
    if phase == "invoked":
        item = record(x, x["output"])
        return {
            "result": {"ref": item["ref"]},
            "events": [{"type": "work_item.recorded", "v": 1, "payload": {"item": item}, "subject": [item["ref"]]}],
        }
    if phase == "read":
        state = x["state"] or {}
        return {"tool": "list_items", "arguments": {"since": state.get("cursor", 0)}}
    return {
        "records": [record(x, n) for n in x["output"]["items"]],
        "state": {"cursor": x["output"]["cursor"]},
    }
