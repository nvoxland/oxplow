# Assign a task to the thread that started it, and clear the assignment
# when it is done or canceled. Reacts to task_record changes whose cause is
# `trigger` (the manifest's `where:`); a change's `fields` cause is its own
# set_fields, which this effect must not answer.
#
# Gets {"event": {...}}; composes oxplow.model.set_fields.

def transform(x):
    e = x["event"]
    p = e["payload"]
    state = p.get("after", {}).get("state")
    if state == "in_progress":
        thread = e["anchors"].get("thread_id")
        if thread == None:
            return {"skip": "no thread started it"}
        id = str(thread)
        if not id.startswith("thr"):
            id = "thr" + id
        values = {"assignee": "oxplow:oxplow/thread?id=" + id}
    elif state in ["done", "canceled"]:
        values = {"assignee": None}
    else:
        return {"skip": "not a start or a close"}
    return {"commands": [{"name": "oxplow.model.set_fields", "input": {
        "model": "oxplow/task_record", "uri": p["uri"], "values": values}}]}
