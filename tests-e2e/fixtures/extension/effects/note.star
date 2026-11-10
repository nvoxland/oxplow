# e2e/note-created: what a spec drives through Delivery and Approvals.
# A title with "[fail]" comments on an item that doesn't exist (a failed
# reaction); one with "[delete]" deletes the item (destructive, so a
# proposal a person approves); any other is commented on.

# The item as it reads now: a person's retry composes from this afresh.
_ITEM = "SELECT CAST(uri AS TEXT) AS uri, CAST(title AS TEXT) AS title FROM v_task WHERE uri = :task"

def transform(x):
    rows = scope("sql.read", {"sql": _ITEM, "params": {"task": x["event"]["payload"]["task"]}})
    if not rows:
        return {"skip": "the item is gone"}
    item = rows[0]
    if "[delete]" in item["title"]:
        return {"commands": [{"name": "oxplow.task.delete", "input": {"uri": item["uri"]}}]}
    target = uri.build("oxplow/task", provider="oxtask", id="tsk999999") if "[fail]" in item["title"] else item["uri"]
    return {"commands": [{
        "name": "oxplow.task.comment",
        "input": {"uri": target, "body": "Noted by the suite's effect."},
    }]}
