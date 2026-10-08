# oxplow.review.request_changes { ref, note? }: comment a checklist of
# what to fix on the effort's work item — each unverified claim, each
# inferred decision, each file outside the item's area — and move it back
# to todo (oxplow: ready). The verdict is logged as
# `oxplow_bundled.changes_requested` with the run, about the effort and
# the item.

# The effort's work item, and what its review weighs: unverified claims,
# inferred decisions, files outside the item's area (JSON arrays).
_REVIEW = """
SELECT e.work_item,
       (SELECT json_group_array(json_object('claim', 'claim:' || c.id, 'statement', c.statement))
          FROM v_claim c WHERE c.effort_id = e.id AND c.verified = 0) AS unverified,
       (SELECT json_group_array(json_object('decision', 'decision:' || d.id, 'question', d.question, 'choice', d.choice))
          FROM v_decision d WHERE d.effort_id = e.id AND d.provenance = 'inferred') AS inferred,
       (SELECT json_group_array(v.path)
          FROM v_oxplow_bundled_deviation v WHERE v.effort_id = e.id) AS deviated
FROM v_effort e
WHERE 'effort:eff' || e.id = :ref
"""

def _list(text):
    return json.decode(text) if text else []

def transform(x):
    ref = x["input"]["ref"]
    rows = capability("sql.read", {"sql": _REVIEW, "params": {"ref": ref}})
    if not rows:
        return {"refuse": "no effort `%s`" % ref}
    row = rows[0]
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
            {"name": "oxplow.work_item.comment", "input": {"ref": item, "body": "\n".join(lines)}},
            {"name": "oxplow.work_item.transition", "input": {"ref": item, "to": "todo"}},
        ],
        "events": [{"type": "oxplow_bundled.changes_requested", "subject": [ref, item], "payload": payload}],
        "result": {"work_item": item},
    }
