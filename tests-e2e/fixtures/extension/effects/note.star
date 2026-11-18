# e2e/note-created: what a spec drives through Delivery and Approvals.
# Each filed task is bookmarked in its stream, under its title. A title with
# "[fail]" is bookmarked in a stream that doesn't exist (a failed reaction);
# one with "[delete]" deletes the project file `doomed.txt` (destructive, so
# a proposal a person approves).
# It reacts to a task being filed (a change of the task model that sets its
# state for the first time), not to later changes.
#
# A task store's own commands (`oxtask.task.*`) write outside the
# transaction, which an effect's composed commands can't, so the effect
# answers with core commands: a bookmark of the task and a file delete.

# The item as it reads now: a person's retry composes from this afresh.
_ITEM = "SELECT CAST(uri AS TEXT) AS uri, CAST(title AS TEXT) AS title, (SELECT 'str' || id FROM v_stream LIMIT 1) AS stream FROM v_oxplow_task WHERE uri = :task"

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
        return {"commands": [{"name": "oxplow.file.delete", "input": {"stream": item["stream"], "path": "doomed.txt"}}]}
    stream = "str999999" if "[fail]" in item["title"] else item["stream"]
    return {"commands": [{
        "name": "oxplow.bookmark.set",
        "input": {"uri": item["uri"], "page_kind": "oxplow/task", "label": item["title"], "scope": "stream", "stream": stream},
    }]}
