# oxplow.tasks.reorder { uri, before?|after? }: a task's place in its
# assignee's queue (or among the unassigned): before or after another task
# of that queue. Its rank goes between the neighbours'; when a neighbour
# has no rank yet (the queue came from starts, not from this command) the
# whole queue is numbered 0, 1, … in its shown order, the task in its new
# place. The undo writes every rank it changed back.

def _set(uri, rank):
    return {"name": "oxplow.model.set_fields", "input": {
        "model": "oxplow/task_record", "uri": uri, "values": {"rank": rank}}}

def transform(x):
    i = x["input"]
    before = i.get("before")
    after = i.get("after")
    if (before == None) == (after == None):
        return {"refuse": "give `before` or `after`, not both and not neither"}
    anchor = before if before != None else after
    found = scope("sql.read", {
        "sql": "SELECT state, assignee FROM v_oxplow_task WHERE uri = :u",
        "params": {"u": i["uri"]},
    })
    if not found:
        return {"refuse": "no task `%s`" % i["uri"]}
    me = found[0]
    if me["state"] in ["done", "canceled"]:
        return {"refuse": "`%s` is %s: a closed item has no place in a queue" % (i["uri"], me["state"])}
    queue = scope("sql.read", {
        "sql": "SELECT uri, rank FROM v_oxplow_task WHERE assignee IS :a AND state NOT IN ('done', 'canceled') ORDER BY rank IS NULL, rank, created_at, uri",
        "params": {"a": me["assignee"]},
    })
    others = [q for q in queue if q["uri"] != i["uri"]]
    mine = [q for q in queue if q["uri"] == i["uri"]]
    old = mine[0]["rank"] if mine else None
    at = None
    for n in range(len(others)):
        if others[n]["uri"] == anchor:
            at = n
    if anchor == i["uri"] or at == None:
        return {"refuse": "`%s` isn't in that queue" % anchor}
    slot = at if before != None else at + 1
    low = others[slot - 1]["rank"] if slot > 0 else None
    high = others[slot]["rank"] if slot < len(others) else None
    ranked = (slot == 0 or low != None) and (slot == len(others) or high != None)
    if ranked:
        if low == None and high == None:
            rank = 0.0
        elif low == None:
            rank = high - 1.0
        elif high == None:
            rank = low + 1.0
        else:
            rank = (low + high) / 2.0
        new = [(i["uri"], old, rank)]
    else:
        order = others[:slot] + [{"uri": i["uri"], "rank": old}] + others[slot:]
        new = []
        for n in range(len(order)):
            if order[n]["rank"] != float(n):
                new.append((order[n]["uri"], order[n]["rank"], float(n)))
    if not new or (len(new) == 1 and new[0][1] == new[0][2]):
        return {"commands": [], "result": {"uri": i["uri"], "rank": old}}
    return {
        "commands": [_set(u, r) for (u, o, r) in new],
        "result": {"uri": i["uri"], "rank": [r for (u, o, r) in new if u == i["uri"]][0] if [1 for (u, o, r) in new if u == i["uri"]] else old},
    }
