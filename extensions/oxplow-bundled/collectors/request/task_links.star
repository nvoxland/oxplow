# Links the request of the turn a task changed in to that task (TEMPORARY:
# the switch renames the entity `request_task`). It runs for a task_record
# change whose cause is `trigger` or `fields` (the manifest's `where:`), so
# a reconcile links nothing.
#
# Gets {"rows": [...], "event": {...}}: one row {request, thread_id,
# thread_uri} when the change's turn has a request; none otherwise (a
# change made outside any turn). Returns the (request, task, basis) row:
# `transition` when the change moved the state, `assigned` when it was a
# field write and the task is now assigned to this thread (the event names
# no field and no value: `changed` is just ["fields"]), else `edited`. Rows accumulate (sync: upsert).

def transform(x):
    if not x["rows"]:
        return {"entities": {"request_task_next": []}}
    r = x["rows"][0]
    p = x["event"]["payload"]
    changed = p.get("changed", [])
    if "state" in changed:
        basis = "transition"
    elif p["cause"] == "fields" and r["assignee"] != None and r["assignee"] == r["thread_uri"]:
        basis = "assigned"
    else:
        basis = "edited"
    return {"entities": {"request_task_next": [{
        "key": r["request"] + "|" + p["uri"] + "|" + basis,
        "request": r["request"],
        "task": p["uri"],
        "basis": basis,
        "thread_id": r["thread_id"],
    }]}}
