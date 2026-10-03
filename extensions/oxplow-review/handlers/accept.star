# oxplow_review.accept { ref, force? }: comment the review on the effort's
# work item, then mark it done. Refuses while a claim is unverified or an
# inferred decision unreviewed, unless forced. The verdict is logged as
# `oxplow_review.verdict` with the run.

def _list(text):
    return json.decode(text) if text else []

def _count(n, one, many):
    return "%d %s" % (n, one if n == 1 else many)

def transform(x):
    ref = x["input"]["ref"]
    if not x["rows"]:
        return {"refuse": "no effort `%s`" % ref}
    row = x["rows"][0]
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
        lines.extend(["- %s" % c["statement"] for c in claims])
    if decisions:
        lines.append("")
        lines.append("Accepted with unreviewed decisions:")
        lines.extend(["- %s → %s" % (d["question"], d["choice"]) for d in decisions])
    item = row["work_item"]
    return {
        "commands": [
            {"name": "work_item.comment", "input": {"ref": item, "body": "\n".join(lines)}},
            {"name": "work_item.transition", "input": {"ref": item, "to": "done"}},
        ],
        "events": [{
            "type": "oxplow_review.verdict",
            "subject": [ref, item],
            "payload": {
                "effort": ref,
                "work_item": item,
                "verdict": "accepted",
                "forced": bool(claims or decisions),
                "unverified": len(claims),
                "inferred": len(decisions),
                "deviated": len(_list(row["deviated"])),
            },
        }],
        "result": {"work_item": item},
    }
