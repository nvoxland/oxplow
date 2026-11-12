# oxtask.task.restore { id }: bring a deleted task back. It is the inverse
# of delete; its own inverse is delete.

def transform(x):
    id = x["input"]["id"]
    write = {
        "sql": "UPDATE task SET deleted = 0, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND deleted = 1",
        "params": [id],
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
    found = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 1", "params": [id]})["rows"]
    if len(found) == 1:
        out["inverse"] = {"commands": [{"name": "oxtask.task.delete", "input": {"id": id}}]}
    return out
