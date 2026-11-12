# oxtask.task.comment { id, body }: add a comment. The insert mints the
# comment's id (`cmt` + the row number), which `finish` reads from the
# write's `rows`: the result names it and the inverse removes it. The author
# is who ran the command (`x["actor"]["uri"]`).

def transform(x):
    i = x["input"]
    there = scope("store.sql", {"sql": "SELECT 1 AS there FROM task WHERE id = ?1 AND deleted = 0", "params": [i["id"]]})["rows"]
    if len(there) == 0:
        return {"refuse": "no such task " + i["id"]}
    write = {
        "sql": "INSERT INTO task_comment (task_id, author, body) VALUES (?1, ?2, ?3) RETURNING id",
        "params": [i["id"], x["actor"]["uri"], i["body"]],
        "expect_changes": 1,
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": i["id"]},
    }

def finish(x):
    comment = x["steps"][0]["result"]["rows"][0][0]["id"]
    return {
        "result": {"id": x["input"]["id"], "comment": comment},
        "inverse": {"commands": [{"name": "oxtask.task.uncomment", "input": {"comment": comment}}]},
    }
