# oxwiki's pages are the markdown files of .oxplow/oxwiki: on start and
# whenever one changes, state them all as the rows of the knowledge model's
# `oxwiki` store (a snapshot: a file that is gone is a page that is gone).
# A file whose name isn't a slug (`Notes.md`) is no page.

FOLDER = ".oxplow/oxwiki"

def valid(slug):
    if slug == "":
        return False
    for part in slug.split("-"):
        if part == "":
            return False
        for c in part.elems():
            if not ((c >= "a" and c <= "z") or (c >= "0" and c <= "9")):
                return False
    return True

def title_of(body, fallback):
    fenced = False
    for line in body.split("\n"):
        t = line.lstrip()
        if t.startswith("```") or t.startswith("~~~"):
            fenced = not fenced
            continue
        if fenced:
            continue
        if t.startswith("# "):
            h = t[2:].strip()
            if h != "":
                return h
    return fallback

# A page file bigger than this is skipped (and said so), as is one whose
# bytes are not UTF-8: one such file never fails the whole snapshot.
MAX_BYTES = 1048576

def transform(x):
    rows = []
    problems = []
    for f in scope("files.list", {"path": FOLDER, "glob": "*.md"})["files"]:
        parts = f["path"].split("/")
        # Only a file directly in the folder is a page.
        if len(parts) != len(FOLDER.split("/")) + 1:
            continue
        slug = parts[-1][:-3]
        if not valid(slug):
            continue
        if f["size"] > MAX_BYTES:
            problems.append("`%s` is bigger than %d bytes: skipped" % (f["path"], MAX_BYTES))
            continue
        read = scope("files.read", {"path": f["path"], "lossy": True})
        if read["lossy"]:
            problems.append("`%s` is not UTF-8 text: skipped" % f["path"])
            continue
        body = read["body"]
        rows.append({
            "id": slug,
            "title": title_of(body, slug),
            "body": body,
            "kind": "note",
            "updated_at": f["modified_at"],
        })
    return {
        "models": [{"model": "oxplow/knowledge_record", "store": "oxwiki", "snapshot": rows}],
        "problems": problems,
    }
