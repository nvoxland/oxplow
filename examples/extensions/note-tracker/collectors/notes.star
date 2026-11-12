# The tasks of the notes store are the markdown files in notes/.
# id = the file name without .md (`note-buy-milk`). title = the first "# "
# heading (else the id). stage = a line `stage: jotted|working|settled` (else
# jotted). body = the file's text after the heading and the stage line.
#
# On start: list the folder and state a snapshot (the full reconcile).
# On files: upsert changed files, delete removed ones.

MODEL = "oxplow/task_record"
STORE = "notes"
STAGES = {
    "jotted": ["Jotted", "todo"],
    "working": ["Working", "in_progress"],
    "settled": ["Settled", "done"],
}

def note_id(path):
    name = path.split("/")[-1]
    return name[:-3] if name.endswith(".md") else name

def read_row(path):
    text = scope("files.read", {"path": path})["body"]
    id = note_id(path)
    title = id
    stage = "jotted"
    body = []
    for line in text.splitlines():
        if line.startswith("# ") and title == id:
            title = line[2:].strip()
        elif line.startswith("stage:"):
            named = line[len("stage:"):].strip()
            if named in STAGES:
                stage = named
        else:
            body.append(line)
    chosen = STAGES[stage]
    return {
        "id": id,
        "title": title,
        "body": "\n".join(body).strip(),
        "state": chosen[1],
        "stage": stage,
        "stage_title": chosen[0],
    }

def transform(x):
    if x["trigger"] == "start":
        listed = scope("files.list", {"path": "notes", "glob": "*.md"})["files"]
        rows = [read_row(f["path"]) for f in listed]
        return {"models": [{"model": MODEL, "store": STORE, "snapshot": rows}]}
    upsert = []
    delete = []
    for c in x["changed"]:
        if c["op"] == "removed":
            delete.append(note_id(c["path"]))
        else:
            upsert.append(read_row(c["path"]))
    return {"models": [{"model": MODEL, "store": STORE, "upsert": upsert, "delete": delete}]}
