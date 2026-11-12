# e2e/note-created: what a spec drives through Delivery and Approvals.
# A title with "[fail]" comments on an item that doesn't exist (a failed
# reaction); one with "[delete]" deletes the item (destructive, so a
# proposal a person approves); any other is commented on. It reacts to a
# task being filed (a change of the task model that sets its state for the
# first time), not to later changes.

# The item as it reads now: a person's retry composes from this afresh.
_ITEM = "SELECT CAST(uri AS TEXT) AS uri, CAST(title AS TEXT) AS title, CAST(native_id AS TEXT) AS id FROM v_oxplow_task WHERE uri = :task"

def transform(x):
    p = x["event"]["payload"]
    changed = p.get("changed", [])
    if "state" not in changed or "state" in (p.get("before") or {}):
        return {"skip": "not a task being filed"}
    rows = scope("sql.read", {"sql": _ITEM, "params": {"task": p["uri"]}})
    if not rows:
        return {"skip": "the item is gone"}
    item = rows[0]
    if "[delete]" in item["title"]:
        return {"commands": [{"name": "oxtask.task.delete", "input": {"id": item["id"]}}]}
    target = "tsk999999" if "[fail]" in item["title"] else item["id"]
    return {"commands": [{
        "name": "oxtask.task.comment",
        "input": {"id": target, "body": "Noted by the suite's effect."},
    }]}
