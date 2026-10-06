# oxplow-bundled/verify-unchecked: after an acceptance that left claims
# unverified or decisions inferred (a forced one), file one item to verify
# them — a checklist naming each, on the active tracker like every new
# item. Its
# `input` reads the acceptance's subject (the effort, its item, what was
# accepted unchecked) and is empty when nothing was, or when this effect
# already followed up an earlier acceptance of the same effort.

def _list(text):
    return json.decode(text) if text else []

# A checklist line holds text, not markdown: whitespace (newlines too)
# becomes one space, markdown's punctuation is escaped, and a long one is
# cut — so a claim can't add items, links or headings to the body.
_LINE = 300
_ITEMS = 50

def _text(value):
    s = " ".join(str(value).split())
    for ch in ["\\", "`", "*", "_", "[", "]", "<", ">", "#", "|"]:
        s = s.replace(ch, "\\" + ch)
    if len(s) > _LINE:
        s = s[:_LINE].rstrip("\\") + "…"
    return s

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
    items = ["- [ ] %s: %s" % (c["claim"], _text(c["statement"])) for c in claims]
    items.extend(["- [ ] %s: %s → %s" % (d["decision"], _text(d["question"]), _text(d["choice"])) for d in decisions])
    lines = ["The review of %s (%s) accepted these unchecked:" % (effort, item), ""]
    lines.extend(items[:_ITEMS])
    if len(items) > _ITEMS:
        lines.append("… and %d more; the review lists them all." % (len(items) - _ITEMS))
    return {
        "commands": [{
            "name": "work_item.create",
            "input": {
                "title": "Verify what the review of %s accepted unchecked" % effort,
                "body": "\n".join(lines),
            },
        }],
    }
