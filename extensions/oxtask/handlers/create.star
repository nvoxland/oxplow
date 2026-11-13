# oxtask.task.create { title, body?, stage?, parent?, priority?, links? }: insert a
# row. The insert mints the id (`tsk` + the row number) and returns it, so the
# result and the inverse come from `finish`, which sees the write's `rows`
# once it landed. The author is who ran the command (`x["actor"]["uri"]`), with its kind
# (`x["actor"]["kind"]`: person, agent or system).

def transform(x):
    i = x["input"]
    parent = i.get("parent")
    if parent != None:
        there = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [parent]})["rows"]
        if len(there) == 0:
            return {"refuse": "no such task " + parent}
    stage = i.get("stage", "backlog")
    write = {
        "sql": "INSERT INTO task (title, body, stage, parent, priority, author, author_kind, closed_at) " +
               "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, " +
               "CASE WHEN ?3 IN ('done', 'canceled') THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END) " +
               "RETURNING id",
        "params": [i["title"], i.get("body"), stage, i.get("parent"), i.get("priority"), x["actor"]["uri"], x["actor"]["kind"]],
    }
    batch = [write]
    # Each link goes to the task just inserted (the highest number: the
    # write is one transaction, and numbers only grow).
    for link in i.get("links", []):
        batch.append({
            "sql": "INSERT INTO task_link (task_id, to_uri, type) " +
                   "SELECT id, ?1, ?2 FROM task ORDER BY n DESC LIMIT 1",
            "params": [link["to"], link["type"]],
            "expect_changes": 1,
        })
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": batch}}],
        "events": [],
        "result": {"stage": stage},
    }

def finish(x):
    id = x["steps"][0]["result"]["rows"][0][0]["id"]
    return {
        "result": {"id": id, "stage": x["input"].get("stage", "backlog")},
        "inverse": {"commands": [{"name": "oxtask.task.delete", "input": {"id": id}}]},
    }
