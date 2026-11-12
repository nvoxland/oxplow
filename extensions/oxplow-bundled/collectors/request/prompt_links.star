# Links the request of a person's prompt to the tasks the prompt names, as
# core did (basis `prompt`): a prompt pointing at a task with
# `[oxplow uri oxplow:oxplow/task?id=…]` or a task mention is a link from its
# turn, recorded as an edge when the prompt is logged. A reprompt in the same
# turn adds its own.
#
# Gets {"rows": [...], "event": {...}}: one row {request, thread_uri, task}
# for each task the turn's prompts named; the event's `source` must be
# `person` (a report's or a peer's text links none). Returns the (request, task, `prompt`) rows,
# stamped with the event's time and turn (TEMPORARY: the model is
# `request_task_next` until the flip).

def transform(x):
    e = x["event"]
    # Whose words the prompt are is the prompt's, not the turn's: a person's
    # prompt landing in a turn a report started links; a report's text does not.
    if e["payload"].get("source") != "person":
        return {}
    rows = []
    for r in x["rows"]:
        rows.append({
            "key": r["request"] + "|" + r["task"] + "|prompt",
            "request": r["request"],
            "task": r["task"],
            "basis": "prompt",
            "thread": r["thread_uri"],
            "turn_id": e["anchors"]["turn_id"],
            "at": e["at"],
        })
    if not rows:
        return {}
    return {"models": [{"model": "oxplow/request_task_next", "store": "oxplow", "upsert": rows}]}
