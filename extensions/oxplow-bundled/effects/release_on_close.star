# Give a closed thread's, or an archived stream's, open tasks back to
# nobody, as core did: every task assigned to the thread that isn't done or
# canceled (a closed one has no assignee) goes to the end of nobody's queue,
# in the order the thread held them, its assignee and place set together.
#
# Gets {"event": {...}} of `thread.closed { thread }` (one thread) or
# `stream.archived { stream, threads }` (each of its threads, in order);
# composes oxplow.model.set_fields once per task. (Not oxplow.tasks.assign:
# its script runs once for all the steps, so every release would take the
# same place at the end; here each takes the next.)

def transform(x):
    p = x["event"]["payload"]
    threads = p["threads"] if "threads" in p else [p["thread"]]
    last = scope("sql.read", {"sql": "SELECT MAX(rank) AS last FROM v_oxplow_task WHERE assignee IS NULL"})
    rank = 0.0 if last[0]["last"] == None else last[0]["last"] + 1.0
    commands = []
    for t in threads:
        held = scope("sql.read", {
            "sql": "SELECT uri FROM v_oxplow_task WHERE assignee = :t AND state NOT IN ('done', 'canceled') ORDER BY rank IS NULL, rank, created_at, uri",
            "params": {"t": "oxplow:oxplow/thread?id=" + t},
        })
        for r in held:
            commands.append({"name": "oxplow.model.set_fields", "input": {
                "model": "oxplow/task_record", "uri": r["uri"],
                "values": {"assignee": None, "rank": rank}}})
            rank = rank + 1.0
    if not commands:
        return {"skip": "no task was assigned"}
    return {"commands": commands}
