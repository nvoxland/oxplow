# oxtask.task.unlink { id, to, type }: remove a link (a tombstone, so the
# collector sees it). The inverse links it again, if it was there.

def uri(to):
    if ":" in to:
        return to
    return "oxplow:oxplow/task?id=" + to + "&provider=oxtask"

def transform(x):
    i = x["input"]
    to = uri(i["to"])
    write = {
        "sql": "UPDATE task_link SET deleted = 1 WHERE task_id = ?1 AND to_uri = ?2 AND type = ?3 AND deleted = 0",
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
    if len(found) == 1:
        out["inverse"] = {"commands": [{"name": "oxtask.task.link", "input": {"id": i["id"], "to": to, "type": i["type"]}}]}
    return out
