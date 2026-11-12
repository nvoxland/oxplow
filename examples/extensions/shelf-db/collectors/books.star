# The books of this store are the rows of the book table.
# On start (and a first run, with no cursor): state a snapshot, the whole
# table. On every later change of the file: the rows written since the
# cursor, as upserts, and the removed ones, as deletes. Every write command
# gives its row the next `seq`; a removal keeps the row as a tombstone
# (deleted = 1), so the cursor, the last `seq` seen, misses nothing.

def book(r):
    row = {"id": r["id"], "title": r["title"], "author": r["author"]}
    if r["rating"] != None:
        row["fields"] = {"rating": r["rating"]}
    return row

def transform(x):
    cursor = x["cursor"]
    since = cursor["seq"] if cursor != None else None
    if x["trigger"] == "start" or since == None:
        found = scope("store.sql", {"sql": "SELECT id, title, author, rating, seq FROM book WHERE deleted = 0 ORDER BY seq"})["rows"]
        top = scope("store.sql", {"sql": "SELECT COALESCE(MAX(seq), 0) AS top FROM book"})["rows"][0]["top"]
        return {
            "models": [{"model": "shelf/book", "store": "shelf-db", "snapshot": [book(r) for r in found]}],
            "cursor": {"seq": top},
        }
    found = scope("store.sql", {
        "sql": "SELECT id, title, author, rating, seq, deleted FROM book WHERE seq > ?1 ORDER BY seq",
        "params": [since],
    })["rows"]
    top = since
    for r in found:
        top = max(top, r["seq"])
    return {
        "models": [{
            "model": "shelf/book",
            "store": "shelf-db",
            "upsert": [book(r) for r in found if not r["deleted"]],
            "delete": [r["id"] for r in found if r["deleted"]],
        }],
        "cursor": {"seq": top},
    }
