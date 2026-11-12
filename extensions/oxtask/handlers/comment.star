# oxtask.task.comment { id, body, author? }: add a comment. Its id is minted
# by the insert (in the write's `rows`), so there is no `inverse`.

def transform(x):
    i = x["input"]
    write = {
        "sql": "INSERT INTO task_comment (task_id, author, body) VALUES (?1, ?2, ?3) RETURNING id",
        "params": [i["id"], i.get("author"), i["body"]],
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": i["id"]},
    }
