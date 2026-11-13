# oxtask.task.link { id, to, type }: link a task to a URI (or another
# oxtask id, written as `tsk42`). A removed link of the same key comes back.
# The inverse is unlink, unless the link was already there.

def uri(to):
    if ":" in to:
        return to
    return "oxplow:oxplow/task?id=" + to + "&provider=oxtask"

def transform(x):
    i = x["input"]
    to = uri(i["to"])
    there = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [i["id"]]})["rows"]
    if len(there) == 0:
        return {"refuse": "no such task " + i["id"]}
    write = {
        "sql": "INSERT INTO task_link (task_id, to_uri, type) VALUES (?1, ?2, ?3) " +
               "ON CONFLICT(task_id, to_uri, type) DO UPDATE SET deleted = 0",
        "params": [i["id"], to, i["type"]],
        "expect_changes": 1,
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": i["id"], "to": to, "type": i["type"]},
    }
    found = scope("store.sql", {
        "sql": "SELECT 1 AS there FROM task_link WHERE task_id = ?1 AND to_uri = ?2 AND type = ?3 AND deleted = 0",
        "params": [i["id"], to, i["type"]],
    })["rows"]
    if len(found) == 0:
        out["inverse"] = {"commands": [{"name": "oxtask.task.unlink", "input": {"id": i["id"], "to": to, "type": i["type"]}}]}
    return out
