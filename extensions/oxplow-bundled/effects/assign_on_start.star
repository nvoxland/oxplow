# Assign a task to the thread whose agent started it, at the end of that
# thread's queue, and clear its assignment (the assignee and the place) when
# it is done or canceled: what core's tracker did. Reacts to task_record
# changes whose cause is `trigger` (the manifest's `where:`); a change's
# `fields` cause is its own set_fields, which this effect must not answer.
#
# Only an agent's start assigns: the change's source is `agent:<thread>`
# (a person's start, `human`, assigns nobody).
#
# Gets {"event": {...}}; composes oxplow.tasks.assign (the end of the
# queue) or oxplow.model.set_fields (the clear).

def transform(x):
    e = x["event"]
    p = e["payload"]
    state = p.get("after", {}).get("state")
    if state == "in_progress":
        source = e.get("source") or ""
        if not source.startswith("agent:"):
            return {"skip": "not an agent's start"}
        thread = source[len("agent:"):]
        if not thread.startswith("thr"):
            thread = "thr" + thread
        return {"commands": [{"name": "oxplow.tasks.assign", "input": {
            "uri": p["uri"], "to": "oxplow:oxplow/thread?id=" + thread}}]}
    elif state in ["done", "canceled"]:
        return {"commands": [{"name": "oxplow.model.set_fields", "input": {
            "model": "oxplow/task_record", "uri": p["uri"],
            "values": {"assignee": None, "rank": None}}}]}
    return {"skip": "not a start or a close"}
