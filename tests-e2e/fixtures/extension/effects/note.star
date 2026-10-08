# e2e/note-created: what a spec drives through Delivery and Approvals.
# A title with "[fail]" comments on an item that doesn't exist (a failed
# reaction); one with "[delete]" deletes the item (destructive, so a
# proposal a person approves); any other is commented on.

# The item as it reads now: a person's retry composes from this afresh.
_ITEM = "SELECT CAST(ref AS TEXT) AS ref, CAST(title AS TEXT) AS title FROM v_work_item WHERE ref = :work_item"

def transform(x):
    rows = capability("sql.read", {"sql": _ITEM, "params": {"work_item": x["event"]["payload"]["work_item"]}})
    if not rows:
        return {"skip": "the item is gone"}
    item = rows[0]
    if "[delete]" in item["title"]:
        return {"commands": [{"name": "oxplow.work_item.delete", "input": {"ref": item["ref"]}}]}
    target = "work_item:oxplow:tsk999999" if "[fail]" in item["title"] else item["ref"]
    return {"commands": [{
        "name": "oxplow.work_item.comment",
        "input": {"ref": target, "body": "Noted by the suite's effect."},
    }]}
