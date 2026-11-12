# oxplow.tasks.assign { uri, to? }: who is going to work on a task — a
# thread (its URI), the person (`oxplow:oxplow/person`) or nobody (`to`
# missing or null) — at the end of that queue. It writes oxplow's two
# fields on the task, `assignee` and `rank`, through `oxplow.model.set_fields`
# and answers the undo: the previous assignee and rank written back.
# (The switch from the core verb renames it `oxplow.task.assign`.)

PERSON = "oxplow:oxplow/person"

def transform(x):
    i = x["input"]
    found = scope("sql.read", {
        "sql": "SELECT state, assignee, rank FROM v_oxplow_task_next WHERE uri = :u",
        "params": {"u": i["uri"]},
    })
    if not found:
        return {"refuse": "no task `%s`" % i["uri"]}
    task = found[0]
    to = i.get("to")
    if to != None and to != PERSON:
        if not to.startswith("oxplow:oxplow/thread?id=thr"):
            return {"refuse": "`to` is a thread's URI, `%s` or null" % PERSON}
        thread = scope("sql.read", {"sql": "SELECT 1 AS there FROM v_thread WHERE uri = :t", "params": {"t": to}})
        if not thread:
            return {"refuse": "no thread `%s`" % to}
    if to != None and task["state"] in ["done", "canceled"]:
        return {"refuse": "`%s` is %s: a closed item is nobody's to work on" % (i["uri"], task["state"])}
    if to == task["assignee"]:
        return {"commands": [], "result": {"uri": i["uri"], "assignee": to, "rank": task["rank"]}}
    if to == None:
        last = scope("sql.read", {"sql": "SELECT MAX(rank) AS last FROM v_oxplow_task_next WHERE assignee IS NULL AND uri != :u", "params": {"u": i["uri"]}})
    else:
        last = scope("sql.read", {"sql": "SELECT MAX(rank) AS last FROM v_oxplow_task_next WHERE assignee = :t", "params": {"t": to}})
    top = last[0]["last"]
    rank = 0.0 if top == None else top + 1.0
    return {
        "commands": [{"name": "oxplow.model.set_fields", "input": {
            "model": "oxplow/task_record", "uri": i["uri"],
            "values": {"assignee": to, "rank": rank}}}],
        "result": {"uri": i["uri"], "assignee": to, "rank": rank},
    }
