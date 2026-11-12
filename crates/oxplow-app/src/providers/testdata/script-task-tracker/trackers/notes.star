# A task tracker written as a script: one function per task tracker verb.
#
# Each verb gets one dict, `x`:
#   x["input"]   the verb's input (core resolved stages and refs already)
#   x["config"]  the implementation's `config:` from extension.yaml
#   x["id"]      the implementation's id, which its items' URIs carry
#   x["key"]     the call's idempotency key
# and answers { result, events?, inverse? }, as a provider's verb does.
#
# It keeps nothing of its own: each answer records the item's whole row as
# a `task.recorded@3` event, and a later verb reads the row back from
# `v_task` (`needs: [sql.read]`).

def _recorded(item):
    return {
        "type": "task.recorded",
        "v": 3,
        "payload": {"item": item},
        "subject": [item["uri"]],
    }

def _item(uri_text):
    rows = scope("sql.read", {
        "sql": "SELECT uri, title, body, stage FROM v_task WHERE uri = :uri",
        "params": {"uri": uri_text},
    })
    if not rows:
        fail("no note %s" % uri_text)
    row = rows[0]
    return {
        "uri": row["uri"],
        "title": row["title"],
        "body": row["body"] or "",
        "stage": row["stage"],
        "native": {},
    }

def create(x):
    i = x["input"]
    # The key is the call's: the same call, sent again, is the same note.
    number = abs(hash(x["key"]))
    item = {
        "uri": uri.build("oxplow/task", id = "%s%d" % (x["config"].get("prefix", "n"), number), provider = x["id"]),
        "title": i["title"],
        "body": i.get("body", ""),
        "stage": i["stage"],
        "native": {},
    }
    return {"result": {"uri": item["uri"]}, "events": [_recorded(item)]}

def update(x):
    i = x["input"]
    item = _item(i["uri"])
    for key in ["title", "body", "stage"]:
        if key in i:
            item[key] = i[key]
    return {"result": {"uri": item["uri"]}, "events": [_recorded(item)]}

def transition(x):
    i = x["input"]
    item = _item(i["uri"])
    was = item["stage"]
    item["stage"] = i["to"]
    return {
        "result": {"uri": item["uri"]},
        "events": [_recorded(item)],
        # Undo moves it back.
        "inverse": {"command": "transition", "input": {"uri": item["uri"], "to": was}},
    }
