# The books of this store are the rows of the book table.
# On start and on every change of the file: state a snapshot.
# The rating goes in the store's own field. The cursor is the max rowid.

def transform(x):
    found = scope("store.sql", {"sql": "SELECT rowid AS rid, id, title, author, rating FROM book ORDER BY rowid"})["rows"]
    rows = []
    top = 0
    for r in found:
        row = {"id": r["id"], "title": r["title"], "author": r["author"]}
        if r["rating"] != None:
            row["fields"] = {"rating": r["rating"]}
        rows.append(row)
        top = max(top, r["rid"])
    return {
        "models": [{"model": "shelf/book", "store": "shelf-db", "snapshot": rows}],
        "cursor": {"rowid": top},
    }
