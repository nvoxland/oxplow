# Links the request of a turn that ended to the tasks in progress and
# assigned to its thread (basis `assigned`, the weakest evidence: the thread
# held the task as the turn ended), as core did.
#
# Gets {"rows": [...], "event": {...}}: one row {request, thread_uri, task}
# for each such task; none when the turn has no request or the thread holds
# nothing in progress. Returns the (request, task, `assigned`) rows, stamped
# with the event's time and turn.

def transform(x):
    e = x["event"]
    rows = []
    for r in x["rows"]:
        rows.append({
            "key": r["request"] + "|" + r["task"] + "|assigned",
            "request": r["request"],
            "task": r["task"],
            "basis": "assigned",
            "thread": r["thread_uri"],
            "turn_id": e["anchors"]["turn_id"],
            "at": e["at"],
        })
    if not rows:
        return {}
    return {"models": [{"model": "oxplow/request_task", "store": "oxplow", "upsert": rows}]}
