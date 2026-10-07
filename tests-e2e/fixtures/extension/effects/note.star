# e2e/note-created: what a spec drives through Delivery and Approvals.
# A title with "[fail]" comments on an item that doesn't exist (a failed
# reaction); one with "[delete]" deletes the item (destructive, so a
# proposal a person approves); any other is commented on.

def transform(x):
    if not x["rows"]:
        return {"skip": "the item is gone"}
    item = x["rows"][0]
    if "[delete]" in item["title"]:
        return {"commands": [{"name": "oxplow.work_item.delete", "input": {"ref": item["ref"]}}]}
    target = "work_item:oxplow:tsk999999" if "[fail]" in item["title"] else item["ref"]
    return {"commands": [{
        "name": "oxplow.work_item.comment",
        "input": {"ref": target, "body": "Noted by the suite's effect."},
    }]}
