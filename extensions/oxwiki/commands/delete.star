# oxwiki.page.delete { id }: delete a page's file; the undo writes it back.

FOLDER = ".oxplow/oxwiki"

def transform(x):
    slug = x["input"]["id"]
    path = FOLDER + "/" + slug + ".md"
    listed = scope("files.list", {"path": FOLDER, "glob": slug + ".md"})["files"]
    if len(listed) == 0:
        return {"refuse": "oxwiki has no page `" + slug + "`"}
    before = scope("files.read", {"path": path})["body"]
    return {
        "commands": [{"name": "oxplow.extension_files.delete", "input": {"path": path}}],
        "result": {"id": slug, "deleted": True},
        "inverse": {"commands": [{"name": "oxwiki.page.restore", "input": {"id": slug, "body": before}}]},
    }
