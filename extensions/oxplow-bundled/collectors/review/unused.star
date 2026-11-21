# oxplow.review.unused — "this change removed the last call to `foo`, so
# `foo` may be unused now": a check the change review runs when a change's
# analysis is stored (`change.analyzed`, phase `files`).
#
# For each call-shaped name (`name(`) on a line the change removed, that
# isn't also called on a line it added in the same file (a call that moved
# isn't removed): a symbol the head tree defines (a library call has none),
# with no mention in the head tree besides its definition (a text search,
# so a mention in a comment or a string keeps it), is a row. The entity
# holds this change's rows, so a rerun replaces them: the keys it no longer
# produces are tombstoned.

# Words that look like calls and aren't.
KEYWORDS = {
    "if": 1, "for": 1, "while": 1, "switch": 1, "match": 1, "return": 1, "catch": 1,
    "fn": 1, "func": 1, "function": 1, "def": 1, "defn": 1, "lambda": 1, "sizeof": 1,
    "assert": 1, "else": 1, "do": 1, "try": 1, "new": 1, "await": 1, "yield": 1,
    "print": 1, "println": 1, "len": 1, "range": 1, "str": 1, "Some": 1, "Ok": 1, "Err": 1,
    "Box": 1, "Vec": 1, "String": 1, "None": 1,
}
KINDS = ["function", "method", "constructor", "macro"]
MOST_NAMES = 50

def call_names(text):
    """The names called on a line: an identifier right before `(`."""
    names = []
    for m in regex_find("([A-Za-z_][A-Za-z0-9_]*)\\(", text):
        names.append(m[1])
    return names

def transform(input):
    change_id = input["event"]["payload"]["change_id"]
    stream_id = input["rows"][0]["stream_id"]
    earlier = scope("sql.read", {
        "sql": "SELECT id FROM v_oxplow_review_unused WHERE change_id = :c",
        "params": {"c": change_id},
    })

    # Each name once, with where its call was removed.
    removed = {}
    for f in change_diff(change_id):
        added = {}
        for h in f["hunks"]:
            for l in h["added"]:
                for n in call_names(l["text"]):
                    added[n] = True
        for h in f["hunks"]:
            for l in h["removed"]:
                for n in call_names(l["text"]):
                    if n in KEYWORDS or n in added or n in removed:
                        continue
                    removed[n] = "%s:%d" % (f["path"], l["line"])

    rows = []
    for name in sorted(removed.keys())[:MOST_NAMES]:
        defined = scope("sql.read", {
            "sql": "SELECT uri, path, line FROM v_symbol WHERE stream_id = :s AND name = :n AND kind IN ('function', 'method', 'constructor', 'macro') ORDER BY path, line",
            "params": {"s": stream_id, "n": name},
        })
        if not defined:
            continue  # a library call: nothing here defines it
        definitions = {}
        for d in defined:
            definitions["%s:%d" % (d["path"], d["line"])] = True
        mentions = [
            h for h in grep_tree(name, max = 20)
            if ("%s:%d" % (h["path"], h["line"])) not in definitions
        ]
        if mentions:
            continue
        first = defined[0]
        rows.append({
            "id": "%d|%s|%d|possibly-unused" % (change_id, first["path"], first["line"]),
            "change_id": change_id,
            "path": first["path"],
            "line": first["line"],
            "signal": "possibly-unused",
            "title": "Possibly unused now: %s" % name,
            "why": "The change removed the last call to %s (%s); no other mention was found (a text search)." % (name, removed[name]),
            "uri": first["uri"],
        })

    keep = [r["id"] for r in rows]
    return {
        "entities": {"review_unused": rows},
        "deleted": {"review_unused": [e["id"] for e in earlier if e["id"] not in keep]},
    }
