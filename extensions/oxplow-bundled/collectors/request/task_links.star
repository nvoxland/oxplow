# Links the request of the turn a task changed in to that task (TEMPORARY:
# the switch renames the entity `request_task`). It runs for a task_record
# change whose cause is `trigger` or `fields` (the manifest's `where:`), so
# a reconcile links nothing.
#
# Gets {"rows": [...], "event": {...}}: one row {request, thread_id,
# thread_uri} when the change's turn has a request; none otherwise (a
# change made outside any turn). Returns the (request, task, basis) row,
# from what the change event says: `filed` for a new item (no state
# before), `transition` when it moved the state, `assigned` when oxplow's
# assignee field became this thread, else `edited`. Rows accumulate (sync:
# upsert).

def transform(x):
    if not x["rows"]:
        return {"entities": {"request_task_next": []}}
    r = x["rows"][0]
    p = x["event"]["payload"]
    changed = p.get("changed", [])
    before = p.get("before") or {}
    after = p.get("after") or {}
    if "state" in changed and "state" not in before:
        basis = "filed"
    elif "state" in changed:
        basis = "transition"
    elif "fields.oxplow.assignee" in changed and after.get("fields.oxplow.assignee") == r["thread_uri"]:
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
