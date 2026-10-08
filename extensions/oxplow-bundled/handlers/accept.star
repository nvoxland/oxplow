# oxplow.review.accept { ref, force? }: comment the review on the effort's
# work item, then mark it done. Refuses while a claim is unverified or an
# inferred decision unreviewed, unless forced. The verdict is logged as
# `oxplow_bundled.accepted` with the run: its subject — kept when the
# payload expires — names the effort, the item, and every claim and
# decision accepted unchecked.

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

# Claims and decisions are agent-written: each is spliced in as one inert
# line of text (`md_text`), so it can't add headings, links or items to
# the reviewer's comment.

def _count(n, one, many):
    return "%d %s" % (n, one if n == 1 else many)

def transform(x):
    ref = x["input"]["ref"]
    rows = scope("sql.read", {"sql": _REVIEW, "params": {"ref": ref}})
    if not rows:
        return {"refuse": "no effort `%s`" % ref}
    row = rows[0]
    if not row["work_item"]:
        return {"refuse": "effort `%s` has no work item to review" % ref}
    claims = _list(row["unverified"])
    decisions = _list(row["inferred"])
    if (claims or decisions) and not x["input"].get("force", False):
        return {"refuse": "%s and %s to review first — verify, confirm or dismiss them, or accept with `force`" % (
            _count(len(claims), "unverified claim", "unverified claims"),
            _count(len(decisions), "inferred decision", "inferred decisions"),
        )}
    lines = ["Review accepted (%s)." % ref]
    if claims:
        lines.append("")
        lines.append("Accepted with unverified claims:")
        lines.extend(["- %s" % md_text(c["statement"]) for c in claims])
    if decisions:
        lines.append("")
        lines.append("Accepted with unreviewed decisions:")
        lines.extend(["- %s → %s" % (md_text(d["question"]), md_text(d["choice"])) for d in decisions])
    item = row["work_item"]
    return {
        "commands": [
            {"name": "oxplow.work_item.comment", "input": {"ref": item, "body": "\n".join(lines)}},
            {"name": "oxplow.work_item.transition", "input": {"ref": item, "to": "done"}},
        ],
        "events": [{
            "type": "oxplow_bundled.accepted",
            "subject": [ref, item] + [c["claim"] for c in claims] + [d["decision"] for d in decisions],
            "payload": {
                "unverified": len(claims),
                "inferred": len(decisions),
                "deviated": len(_list(row["deviated"])),
            },
        }],
        "result": {"work_item": item},
    }
