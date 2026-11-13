# oxwiki.page.write { id?, title?, body }: write a page's file in the
# primary checkout's .oxplow/oxwiki folder (a files service resolves against
# the project root, never the caller's worktree), refusing a body whose
# [[links]] don't resolve. The undo puts the file back as it was.

FOLDER = ".oxplow/oxwiki"

def slugify(title):
    out = ""
    for c in title.lower().elems():
        if (c >= "a" and c <= "z") or (c >= "0" and c <= "9"):
            out += c
        elif out != "" and not out.endswith("-"):
            out += "-"
    out = out.rstrip("-")
    return out if valid(out) else None

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

def current(slug):
    listed = scope("files.list", {"path": FOLDER, "glob": slug + ".md"})["files"]
    if len(listed) == 0:
        return None
    return scope("files.read", {"path": FOLDER + "/" + slug + ".md"})["body"]

def transform(x):
    i = x["input"]
    slug = i.get("id")
    if slug == None:
        title = i.get("title")
        if title == None:
            return {"refuse": "an oxwiki page needs an `id` (kebab-case) or a `title` to make one from"}
        slug = slugify(title)
        if slug == None:
            return {"refuse": "`" + title + "` makes no page id: give an `id` (kebab-case)"}
    elif not valid(slug):
        return {"refuse": "`" + slug + "` isn't an oxwiki page id (kebab-case: `some-page`)"}
    before = current(slug)
    # A page the title names is new: one already there is replaced only when
    # the write names it.
    if i.get("id") == None and before != None:
        return {"refuse": "oxwiki already has a page `" + slug + "`: pass `id: " + slug + "` to replace it, or another title"}
    # A page whose file is there but whose row isn't yet (the collector follows
    # the folder a moment after a write) is no dangling link.
    warnings = [
        w for w in scope("links.check", {"body": i["body"], "self": "oxplow:oxplow/knowledge?id=" + slug + "&store=oxwiki"})["warnings"]
        if not (valid(w["target"]) and current(w["target"]) != None)
    ]
    if len(warnings) > 0:
        return {"refuse": "a link doesn't resolve: " + "; ".join(["[[" + w["target"] + "]]: " + w["reason"] for w in warnings])}
    path = FOLDER + "/" + slug + ".md"
    return {
        "commands": [{"name": "oxplow.extension_files.write", "input": {"path": path, "body": i["body"]}}],
        "result": {"id": slug, "page": "oxplow:oxplow/knowledge?id=" + slug + "&store=oxwiki"},
        "inverse": {"commands": [{"name": "oxwiki.page.restore", "input": {"id": slug, "body": before}}]},
    }
