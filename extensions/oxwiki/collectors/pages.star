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
    for line in body.split("\n"):
        t = line.lstrip()
        if t.startswith("# "):
            h = t[2:].strip()
            if h != "":
                return h
    return fallback

def transform(x):
    rows = []
    for f in scope("files.list", {"path": FOLDER, "glob": "*.md"})["files"]:
        name = f["path"].split("/")[-1]
        slug = name[:-3]
        if not valid(slug):
            continue
        body = scope("files.read", {"path": f["path"]})["body"]
        rows.append({
            "id": slug,
            "title": title_of(body, slug),
            "body": body,
            "kind": "note",
            "updated_at": f["modified_at"],
        })
    return {"models": [{"model": "oxplow/knowledge_record", "store": "oxwiki", "snapshot": rows}]}
