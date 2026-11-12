# oxtask.task.comment { id, body, author? }: add a comment. Its id is minted
# by the insert (in the write's `rows`), so there is no `inverse`.

def transform(x):
    i = x["input"]
    there = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [i["id"]]})["rows"]
    if len(there) == 0:
        return {"refuse": "no such task " + i["id"]}
    write = {
        "sql": "INSERT INTO task_comment (task_id, author, body) VALUES (?1, ?2, ?3) RETURNING id",
        "params": [i["id"], i.get("author"), i["body"]],
        "expect_changes": 1,
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": i["id"]},
    }
