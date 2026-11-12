# oxtask.task.update { id, title?, body?, priority?, parent? }: change the
# fields given (null clears body, priority and parent). Returns the inverse,
# the same command with the values the task has now.

FIELDS = ["title", "body", "priority", "parent"]

def transform(x):
    i = x["input"]
    id = i["id"]
    sets = []
    params = [id]
    for f in FIELDS:
        if f in i:
            params.append(i[f])
            sets.append(f + " = ?" + str(len(params)))
    if len(sets) == 0:
        return {"refuse": "give at least one of title, body, priority, parent"}
    parent = i.get("parent")
    if parent != None:
        if not parent.startswith("tsk") or not parent[3:].isdigit():
            return {"refuse": "parent must be a task id like tsk12"}
        there = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [parent]})["rows"]
        if len(there) == 0:
            return {"refuse": "no such task " + parent}
    sets.append("updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
    write = {
        "sql": "UPDATE task SET " + ", ".join(sets) + " WHERE id = ?1 AND deleted = 0",
        "params": params,
        "expect_changes": 1,
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
    found = scope("store.sql", {
        "sql": "SELECT title, body, priority, parent FROM task WHERE id = ?1 AND deleted = 0",
        "params": [id],
    })["rows"]
    if len(found) == 0:
        return {"refuse": "no such task " + id}
    if len(found) == 1:
        back = {"id": id}
        for f in FIELDS:
            if f in i:
                back[f] = found[0][f]
        out["inverse"] = {"commands": [{"name": "oxtask.task.update", "input": back}]}
    return out
