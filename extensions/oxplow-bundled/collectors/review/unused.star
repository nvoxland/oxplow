# oxplow.review.unused — "this change removed the last call to `foo`, so
# `foo` may be unused now": a check the change review runs when a change's
# analysis is stored (`change.analyzed`, phase `files`).
#
# For each call-shaped name (`name(`) on a line the change removed, that
# isn't also called on a line it added in the same file (a call that moved
# isn't removed): a function the change's head tree defines (a hit that
# starts a function of that name, by `code_metrics`; a library call has no
# such hit), with no mention in the head tree besides its definition (a text
# search, so a mention in a comment or a string keeps it), is a row. The
# tree is the change's head (`grep_tree` and `files` read the event's
# revision), not the worktree's. The entity
# holds this change's rows, so a rerun replaces them: the keys it no longer
# produces are tombstoned.

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
    earlier = scope("sql.read", {
        "sql": "SELECT id FROM v_oxplow_review_unused WHERE change_id = :c",
        "params": {"c": change_id},
    })

    # The files whose removed lines can hold a call: some lines removed, in
    # a language a pack parses, not generated, vendored or a lockfile. One
    # `change_diff` call returns the files that fit its limits and names the
    # rest, so it is asked again for those (each call reads at least one
    # file or sets one aside as too large, so it ends). A file whose own
    # changed lines are past the call's limit is never read: the check says
    # so in one row for the change, never skips it in silence.
    paths = [
        r["path"]
        for r in scope("sql.read", {
            "sql": "SELECT f.path FROM v_change_file f WHERE f.change_id = :c AND f.deletions > 0 AND NOT EXISTS (SELECT 1 FROM v_file_kind_rule k WHERE k.kind IN ('generated', 'vendored', 'lockfile') AND glob_match(k.glob, f.path)) ORDER BY f.path",
            "params": {"c": change_id},
        })
        if language_of(r["path"]) != None
    ]
    diffs = []
    too_large = []
    todo = paths
    for _ in range(len(paths)):
        if not todo:
            break
        got = change_diff(change_id, paths = todo)
        diffs.extend(got["files"])
        too_large.extend(got["too_large"])
        todo = got["rest"]

    # Each name once, with where its call was removed.
    removed = {}
    for f in diffs:
        # Words that look like calls and aren't are the file's language's
        # (its pack's `keywords:`), not a list of every language's.
        keywords = {}
        for w in language_keywords(language_of(f["path"]) or ""):
            keywords[w] = True
        added = {}
        for h in f["hunks"]:
            for l in h["added"]:
                for n in call_names(l["text"]):
                    added[n] = True
        for h in f["hunks"]:
            for l in h["removed"]:
                for n in call_names(l["text"]):
                    if n in keywords or n in added or n in removed:
                        continue
                    removed[n] = "%s:%d" % (f["path"], l["line"])

    rows = []
    for name in sorted(removed.keys())[:MOST_NAMES]:
        # The change's head tree decides, whatever the worktree holds now:
        # every whole-word mention of the name there, and which of them is
        # the start of a function with that name.
        hits = grep_tree(name, max = 50)
        parsed = {}
        definitions = {}
        mentions = 0
        for h in hits:
            path = h["path"]
            language = language_of(path)
            if language == None:
                mentions += 1  # a file oxplow can't parse: a mention, never a definition
                continue
            if path not in parsed:
                text = files(path)
                parsed[path] = code_metrics(text[0]["text"], language, path) if text else []
            starts = [m for m in parsed[path] if m["name"] == name and m["start_line"] == h["line"]]
            if starts:
                if "first" not in definitions:
                    definitions["first"] = starts[0]
                    definitions["path"] = path
            else:
                mentions += 1
        if not definitions:
            continue  # a library call: the head defines no such function
        if mentions:
            continue
        first = definitions["first"]
        rows.append({
            "id": "%d|%s|%d|possibly-unused" % (change_id, definitions["path"], first["start_line"]),
            "change_id": change_id,
            "path": definitions["path"],
            "line": first["start_line"],
            "signal": "possibly-unused",
            "title": "Possibly unused now: %s" % name,
            "why": "The change removed the last call to %s (%s); no other mention was found (a text search)." % (name, removed[name]),
            "uri": first["uri"],
        })

    if too_large:
        rows.append({
            "id": "%d|||too-large-to-check" % change_id,
            "change_id": change_id,
            "path": None,
            "line": None,
            "signal": "too-large-to-check",
            "title": "%d file%s too large to check" % (len(too_large), "" if len(too_large) == 1 else "s"),
            "why": "%s changed lines are past the 2 MB the unused check reads, so %s checked: %s." % (
                "Its" if len(too_large) == 1 else "Their",
                "it wasn't" if len(too_large) == 1 else "they weren't",
                ", ".join(too_large),
            ),
            "uri": None,
        })

    keep = [r["id"] for r in rows]
    return {
        "entities": {"review_unused": rows},
        "deleted": {"review_unused": [e["id"] for e in earlier if e["id"] not in keep]},
    }
