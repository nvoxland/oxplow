# oxplow-bundled/verify-unchecked: after an acceptance that left claims
# unverified or decisions inferred (a forced one), file one item to verify
# them — a checklist naming each, on the active tracker like every new
# item.

# The acceptance's subject: the effort, its item, and each claim and
# decision accepted unchecked — still unverified or inferred now. None
# when this effect already filed (or proposed) a follow-up for an earlier
# acceptance of the effort; an earlier one it never reacted to (before its
# approval) or that skipped or failed doesn't count.
_ACCEPTED = """
SELECT CAST(json_extract(e.subject, '$[0]') AS TEXT) AS effort,
       CAST(json_extract(e.subject, '$[1]') AS TEXT) AS work_item,
       (SELECT json_group_array(json_object('claim', s.value, 'statement', c.statement))
          FROM json_each(e.subject) s JOIN v_claim c ON s.value = 'claim:' || c.id
         WHERE c.verified = 0) AS claims,
       (SELECT json_group_array(json_object('decision', s.value, 'question', d.question, 'choice', d.choice))
          FROM json_each(e.subject) s JOIN v_decision d ON s.value = 'decision:' || d.id
         WHERE d.provenance = 'inferred') AS decisions
FROM v_event e
WHERE e.id = :event_id
  AND NOT EXISTS (
    SELECT 1 FROM v_event p
      JOIN v_effect_run r ON r.event_id = p.id
     WHERE p.type = 'oxplow_bundled.accepted' AND p.seq < :event_seq
       AND json_extract(p.subject, '$[0]') = json_extract(e.subject, '$[0]')
       AND r.effect = 'oxplow-bundled/verify-unchecked' AND r.latest = 1
       AND r.state IN ('ok', 'proposed'))
"""

def _list(text):
    return json.decode(text) if text else []

# A checklist line holds a claim or decision as text, not markdown
# (`md_text`), so it can't add items, links or headings to the body.
_ITEMS = 50

def transform(x):
    event = x["event"]
    rows = scope("sql.read", {
        "sql": _ACCEPTED,
        "params": {"event_id": event["id"], "event_seq": event["seq"]},
    })
    if not rows:
        return {"skip": "nothing new was accepted unchecked"}
    row = rows[0]
    claims = _list(row["claims"])
    decisions = _list(row["decisions"])
    if not claims and not decisions:
        return {"skip": "nothing was accepted unchecked"}
    effort = row["effort"]
    item = row["work_item"]
    items = ["- [ ] %s: %s" % (c["claim"], md_text(c["statement"])) for c in claims]
    items.extend(["- [ ] %s: %s → %s" % (d["decision"], md_text(d["question"]), md_text(d["choice"])) for d in decisions])
    lines = ["The review of %s (%s) accepted these unchecked:" % (effort, item), ""]
    lines.extend(items[:_ITEMS])
    if len(items) > _ITEMS:
        lines.append("… and %d more; the review lists them all." % (len(items) - _ITEMS))
    return {
        "commands": [{
            "name": "oxplow.work_item.create",
            "input": {
                "title": "Verify what the review of %s accepted unchecked" % effort,
                "body": "\n".join(lines),
            },
        }],
    }
