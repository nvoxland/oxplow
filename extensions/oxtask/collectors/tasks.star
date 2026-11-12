# The tasks of this store are the rows of the task table in oxtask's file.
# On start (or with no cursor) state a snapshot, the whole
# store. On every later change of the file: the tasks written since the
# cursor as upserts (each with its whole sets of links and comments), and
# the soft-deleted ones as deletes. The file's triggers give every write the
# next `seq`, so a write from anywhere is seen.

STAGES = {
    "backlog": ["Backlog", "backlog"],
    "ready": ["Ready", "todo"],
    "in_progress": ["In progress", "in_progress"],
    "blocked": ["Blocked", "blocked"],
    "done": ["Done", "done"],
    "canceled": ["Canceled", "canceled"],
}

def task_uri(id):
    return "oxplow:oxplow/task?id=" + id + "&provider=oxtask"

def sets(where, params):
    links = {}
    for r in scope("store.sql", {
        "sql": "SELECT task_id, to_uri, type FROM task_link WHERE deleted = 0 AND task_id IN (SELECT id FROM task WHERE " + where + ") ORDER BY seq",
        "params": params,
    })["rows"]:
        links.setdefault(r["task_id"], []).append({"to": r["to_uri"], "type": r["type"]})
    comments = {}
    for r in scope("store.sql", {
        "sql": "SELECT id, task_id, author, at, body FROM task_comment WHERE deleted = 0 AND task_id IN (SELECT id FROM task WHERE " + where + ") ORDER BY n",
        "params": params,
    })["rows"]:
        c = {"id": r["id"], "at": r["at"], "body": r["body"]}
        if r["author"] != None:
            c["author"] = r["author"]
        comments.setdefault(r["task_id"], []).append(c)
    return links, comments

def row(r, links, comments):
    stage = STAGES[r["stage"]]
    out = {
        "id": r["id"],
        "title": r["title"],
        "state": stage[1],
        "stage": r["stage"],
        "stage_title": stage[0],
        "created_at": r["created_at"],
        "updated_at": r["updated_at"],
        "links": links.get(r["id"], []),
        "comments": comments.get(r["id"], []),
    }
    for k in ["body", "closed_at"]:
        if r[k] != None:
            out[k] = r[k]
    if r["parent"] != None:
        out["parent"] = task_uri(r["parent"])
    fields = {}
    for k in ["priority", "author"]:
        if r[k] != None:
            fields[k] = r[k]
    if len(fields) > 0:
        out["fields"] = fields
    return out

COLS = "id, title, body, stage, parent, priority, author, created_at, updated_at, closed_at, seq, deleted"

def transform(x):
    cursor = x["cursor"]
    since = cursor["seq"] if cursor != None else None
    top = scope("store.sql", {"sql": "SELECT v AS top FROM change_seq"})["rows"][0]["top"]
    if x["trigger"] == "start" or since == None:
        links, comments = sets("deleted = 0", [])
        found = scope("store.sql", {"sql": "SELECT " + COLS + " FROM task WHERE deleted = 0 ORDER BY n"})["rows"]
        return {
            "models": [{"model": "oxplow/task_record", "store": "oxtask", "snapshot": [row(r, links, comments) for r in found]}],
            "cursor": {"seq": top},
        }
    links, comments = sets("seq > ?1 AND deleted = 0", [since])
    found = scope("store.sql", {
        "sql": "SELECT " + COLS + " FROM task WHERE seq > ?1 AND seq <= ?2 ORDER BY seq",
        "params": [since, top],
    })["rows"]
    return {
        "models": [{
            "model": "oxplow/task_record",
            "store": "oxtask",
            "upsert": [row(r, links, comments) for r in found if not r["deleted"]],
            "delete": [r["id"] for r in found if r["deleted"]],
        }],
        "cursor": {"seq": top},
    }
