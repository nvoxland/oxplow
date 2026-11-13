# oxtask.task.move { id, stage }: put a task in a stage. done and canceled
# set closed_at; any other stage clears it. The inverse moves it back.

def transform(x):
    id = x["input"]["id"]
    stage = x["input"]["stage"]
    write = {
        "sql": "UPDATE task SET stage = ?2, " +
               "closed_at = CASE WHEN ?2 IN ('done', 'canceled') THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END, " +
               "updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') " +
               "WHERE id = ?1 AND deleted = 0",
        "params": [id, stage],
        "expect_changes": 1,
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id, "stage": stage},
    }
    found = scope("store.sql", {
        "sql": "SELECT stage FROM task WHERE id = ?1 AND deleted = 0",
        "params": [id],
    })["rows"]
    if len(found) == 0:
        return {"refuse": "no such task " + id}
    if len(found) == 1:
        out["inverse"] = {"commands": [{"name": "oxtask.task.move", "input": {"id": id, "stage": found[0]["stage"]}}]}
    return out
