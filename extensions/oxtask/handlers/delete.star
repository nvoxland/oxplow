# oxtask.task.delete { id }: soft-delete a task (a tombstone, so the
# collector sees it). The inverse is oxtask.task.restore.

def transform(x):
    id = x["input"]["id"]
    write = {
        "sql": "UPDATE task SET deleted = 1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND deleted = 0",
        "params": [id],
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
    found = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [id]})["rows"]
    if len(found) == 1:
        out["inverse"] = {"commands": [{"name": "oxtask.task.restore", "input": {"id": id}}]}
    return out
