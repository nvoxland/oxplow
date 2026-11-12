# oxtask.task.create { title, body?, stage?, parent?, priority?, author? }:
# insert a row. The id is minted by the insert (`tsk` + the row number), so
# it is in the write's `rows` (the run's children), not in this script's
# `result`: the script runs before the write. That is also why create has no
# `inverse`: it can't name the row it makes.

def transform(x):
    i = x["input"]
    parent = i.get("parent")
    if parent != None:
        there = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [parent]})["rows"]
        if len(there) == 0:
            return {"refuse": "no such task " + parent}
    stage = i.get("stage", "backlog")
    write = {
        "sql": "INSERT INTO task (title, body, stage, parent, priority, author, closed_at) " +
               "VALUES (?1, ?2, ?3, ?4, ?5, ?6, " +
               "CASE WHEN ?3 IN ('done', 'canceled') THEN strftime('%Y-%m-%dT%H:%M:%fZ', 'now') END) " +
               "RETURNING id",
        "params": [i["title"], i.get("body"), stage, i.get("parent"), i.get("priority"), i.get("author")],
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"stage": stage},
    }
