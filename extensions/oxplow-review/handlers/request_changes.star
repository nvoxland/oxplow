# oxplow_review.request_changes { ref, note? }: comment a checklist of
# what to fix on the effort's work item — each unverified claim, each
# inferred decision, each file outside the item's area — and move it back
# to todo (oxplow: ready).

def _list(text):
    return json.decode(text) if text else []

def transform(x):
    ref = x["input"]["ref"]
    if not x["rows"]:
        return {"refuse": "no effort `%s`" % ref}
    row = x["rows"][0]
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
    return {
        "commands": [
            {"name": "work_item.comment", "input": {"ref": item, "body": "\n".join(lines)}},
            {"name": "work_item.transition", "input": {"ref": item, "to": "todo"}},
        ],
        "result": {"work_item": item},
    }
