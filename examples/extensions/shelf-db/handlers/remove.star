# shelf_db.book.remove { id }: mark the row deleted (a tombstone with the
# next change number, so the collector sees the removal).
# The sqlite trigger of the collector removes the model row.
#
# The script also returns the `inverse` that undoes the run: the extension's
# own add command with the book's fields, read here (`store.sql`) before the
# write. A store write leaves oxplow's records, so the host keeps the
# `inverse` as the run's undo. book.add revives a tombstone of the same id.

def transform(x):
    id = x["input"]["id"]
    write = {
        "sql": "UPDATE book SET deleted = 1, seq = (SELECT COALESCE(MAX(seq), 0) + 1 FROM book) " +
               "WHERE id = ?1 AND deleted = 0",
        "params": [id],
    }
    out = {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
    found = scope("store.sql", {
        "sql": "SELECT title, author, rating FROM book WHERE id = ?1 AND deleted = 0",
        "params": [id],
    })["rows"]
    if len(found) == 1:
        book = {"id": id, "title": found[0]["title"]}
        if found[0]["author"] != None:
            book["author"] = found[0]["author"]
        if found[0]["rating"] != None:
            book["rating"] = found[0]["rating"]
        out["inverse"] = {"commands": [{"name": "shelf_db.book.add", "input": book}]}
    return out
