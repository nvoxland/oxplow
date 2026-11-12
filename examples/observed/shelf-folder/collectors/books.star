# The books of this store are the markdown files in shelf/.
# id = file name without .md. title = first "# " heading (else the id).
# body = the whole file.
#
# On start: list the folder and state a snapshot (the full reconcile).
# On files: upsert changed files, delete removed ones.

MODEL = "shelf/book"
STORE = "shelf-folder"

def book_id(path):
    name = path.split("/")[-1]
    return name[:-3] if name.endswith(".md") else name

def read_row(path):
    body = scope("files.read", {"path": path})["body"]
    id = book_id(path)
    title = id
    for line in body.splitlines():
        if line.startswith("# "):
            title = line[2:].strip()
            break
    return {"id": id, "title": title, "body": body}

def transform(x):
    if x["trigger"] == "start":
        listed = scope("files.list", {"path": "shelf", "glob": "*.md"})["files"]
        rows = [read_row(f["path"]) for f in listed]
        done = scope("model.write", {"model": MODEL, "store": STORE, "snapshot": rows})
        return {"result": done}
    upsert = []
    delete = []
    for c in x["changed"]:
        if c["op"] == "removed":
            delete.append(book_id(c["path"]))
        else:
            upsert.append(read_row(c["path"]))
    done = scope("model.write", {"model": MODEL, "store": STORE, "upsert": upsert, "delete": delete})
    return {"result": done}
