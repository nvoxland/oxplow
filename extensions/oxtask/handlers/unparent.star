# oxtask.task.unparent { id }: take a task out of its epic. Returns the
# inverse: the update that puts it back under the epic it had.

def transform(x):
    id = x["input"]["id"]
    found = scope("store.sql", {
        "sql": "SELECT parent FROM task WHERE id = ?1 AND deleted = 0",
        "params": [id],
    })["rows"]
    if len(found) == 0:
        return {"refuse": "no such task " + id}
    write = {
        "sql": "UPDATE task SET parent = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND deleted = 0",
        "params": [id],
        "expect_changes": 1,
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
    if found[0]["parent"] != None:
        out["inverse"] = {"commands": [{"name": "oxtask.task.update", "input": {"id": id, "parent": found[0]["parent"]}}]}
    return out
