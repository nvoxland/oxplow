# oxtask.task.uncomment { comment }: remove a comment from its task (a
# tombstone, so the collector restates the task's comments). It is the undo
# of oxtask.task.comment; it has no inverse of its own.

def transform(x):
    comment = x["input"]["comment"]
    write = {
        "sql": "UPDATE task_comment SET deleted = 1 WHERE id = ?1 AND deleted = 0",
        "params": [comment],
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"comment": comment},
    }
