# Taking a handoff that is about a task moves that task onto the recipient
# thread (a handoff about anything else is none of this effect's), as core's
# `handoff.resolve` did: only when the outcome is `taken`, the handoff names a
# task that is open, and that task isn't already the recipient's. It goes to
# the end of the recipient's queue (oxplow.tasks.assign).
#
# Gets {"event": {...}} of `handoff.settled { handoff, state, from_thread,
# to_thread }`; composes oxplow.tasks.assign.

def transform(x):
    p = x["event"]["payload"]
    if p["state"] != "taken":
        return {"skip": "not taken"}
    found = scope("sql.read", {
        "sql": "SELECT h.about AS task, w.assignee, w.state FROM v_handoff h JOIN v_oxplow_task w ON w.uri = h.about WHERE h.id = CAST(substr(:h, 4) AS INTEGER)",
        "params": {"h": p["handoff"]},
    })
    if not found:
        return {"skip": "it is about no task of the model"}
    to = "oxplow:oxplow/thread?id=" + p["to_thread"]
    if found[0]["assignee"] == to:
        return {"skip": "already the recipient's"}
    if found[0]["state"] in ["done", "canceled"]:
        return {"skip": "the task is closed"}
    return {"commands": [{"name": "oxplow.tasks.assign", "input": {"uri": found[0]["task"], "to": to}}]}
