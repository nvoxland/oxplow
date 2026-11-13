# Links the request of the turn a task changed in to that task (a row of the
# `oxplow/request_task` model, store `oxplow`). It runs for a task_record
# change whose cause is `trigger` or `fields` (the manifest's `where:`), so
# a reconcile links nothing.
#
# Gets {"rows": [...], "event": {...}}: one row {request, thread_id,
# thread_uri} when the change's turn has a request; none otherwise (a
# change made outside any turn). Returns the (request, task, basis) row,
# from what the change event says: `filed` for a new item (no state
# before), `transition` when it moved the state, `assigned` when oxplow's
# assignee field became this thread, else `edited` (a change by `fields`
# links nothing else, and a link already there is left as it is). `at` is the change's
# time and `turn_id` its turn. Rows accumulate (sync: upsert).

def transform(x):
    if not x["rows"]:
        return {}
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
    elif p.get("cause") == "fields":
        # A fields-only change (a reorder, a close's clear of the assignee)
        # is evidence of nothing; core linked on transition, create, update
        # and comment only.
        return {}
    else:
        basis = "edited"
    key = r["request"] + "|" + p["uri"] + "|" + basis
    # A link keeps the first evidence (core's insert or ignore).
    if key in (r["known"] or "").split("\n"):
        return {}
    return {"models": [{"model": "oxplow/request_task", "store": "oxplow", "upsert": [{
        "key": key,
        "request": r["request"],
        "task": p["uri"],
        "basis": basis,
        "thread": r["thread_uri"],
        "turn_id": x["event"]["anchors"]["turn_id"],
        "at": x["event"]["at"],
    }]}]}
