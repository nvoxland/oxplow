# oxwiki.page.restore { id, body }: put a page's file back as it was (`body`
# null: there was none, so remove it). The undo of page.write and
# page.delete, and what undoing it does in turn.

FOLDER = ".oxplow/oxwiki"

def transform(x):
    slug = x["input"]["id"]
    body = x["input"]["body"]
    path = FOLDER + "/" + slug + ".md"
    listed = scope("files.list", {"path": FOLDER, "glob": slug + ".md"})["files"]
    now = None
    if len(listed) > 0:
        now = scope("files.read", {"path": path})["body"]
    undo = {"commands": [{"name": "oxwiki.page.restore", "input": {"id": slug, "body": now}}]}
    if body == None:
        if now == None:
            return {"commands": [], "result": {"id": slug}, "inverse": undo}
        return {
            "commands": [{"name": "oxplow.extension_files.delete", "input": {"path": path}}],
            "result": {"id": slug},
            "inverse": undo,
        }
    return {
        "commands": [{"name": "oxplow.extension_files.write", "input": {"path": path, "body": body}}],
        "result": {"id": slug},
        "inverse": undo,
    }
