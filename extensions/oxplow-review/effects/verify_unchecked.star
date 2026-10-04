# oxplow-review/verify-unchecked: after an acceptance that left claims
# unverified or decisions inferred (a forced one), file one item to verify
# them, on the reviewed item's provider — a checklist naming each. Its
# `input` reads the acceptance's subject (the effort, its item, what was
# accepted unchecked) and is empty when nothing was, or when an earlier
# forced acceptance of the same effort was followed up already.

def _list(text):
    return json.decode(text) if text else []

def transform(x):
    if not x["rows"]:
        return {"skip": "nothing new was accepted unchecked"}
    row = x["rows"][0]
    claims = _list(row["claims"])
    decisions = _list(row["decisions"])
    if not claims and not decisions:
        return {"skip": "nothing was accepted unchecked"}
    effort = row["effort"]
    item = row["work_item"]
    # `work_item:<provider>:<id>`: file on the same tracker.
    provider = item.split(":")[1]
    lines = ["The review of %s (%s) accepted these unchecked:" % (effort, item), ""]
    lines.extend(["- [ ] %s: %s" % (c["claim"], c["statement"]) for c in claims])
    lines.extend(["- [ ] %s: %s → %s" % (d["decision"], d["question"], d["choice"]) for d in decisions])
    return {
        "commands": [{
            "name": "work_item.create",
            "input": {
                "provider": provider,
                "title": "Verify what the review of %s accepted unchecked" % effort,
                "body": "\n".join(lines),
            },
        }],
    }
