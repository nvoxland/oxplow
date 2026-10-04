# oxplow_review.request_changes { ref, note? }: comment a checklist of
# what to fix on the effort's work item — each unverified claim, each
# inferred decision, each file outside the item's area — and move it back
# to todo (oxplow: ready). The verdict is logged as
# `oxplow_review.changes_requested` with the run, about the effort and
# the item.

def _list(text):
    return json.decode(text) if text else []

def transform(x):
    ref = x["input"]["ref"]
    if not x["rows"]:
        return {"refuse": "no effort `%s`" % ref}
    row = x["rows"][0]
    if not row["work_item"]:
        return {"refuse": "effort `%s` has no work item to review" % ref}
    note = x["input"].get("note", "")
    checklist = (
        ["- [ ] Back up the claim: %s" % c["statement"] for c in _list(row["unverified"])] +
        ["- [ ] Confirm or rework the decision: %s → %s" % (d["question"], d["choice"]) for d in _list(row["inferred"])] +
        ["- [ ] Explain or revert the change outside the task's area: %s" % p for p in _list(row["deviated"])]
    )
    if not checklist and not note:
        return {"refuse": "nothing to ask for: every claim is backed, no decision is inferred and nothing deviated — add a `note`"}
    lines = ["Changes requested (%s)." % ref]
    if note:
        lines.extend(["", note])
    if checklist:
        lines.append("")
        lines.extend(checklist)
    item = row["work_item"]
    payload = {
        "unverified": len(_list(row["unverified"])),
        "inferred": len(_list(row["inferred"])),
        "deviated": len(_list(row["deviated"])),
    }
    if note:
        payload["note"] = note
    return {
        "commands": [
            {"name": "work_item.comment", "input": {"ref": item, "body": "\n".join(lines)}},
            {"name": "work_item.transition", "input": {"ref": item, "to": "todo"}},
        ],
        "events": [{"type": "oxplow_review.changes_requested", "subject": [ref, item], "payload": payload}],
        "result": {"work_item": item},
    }
