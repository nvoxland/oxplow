# shelf_db.book.remove { id }: mark the row deleted (a tombstone with the
# next change number, so the collector sees the removal).
# The sqlite trigger of the collector removes the model row.

def transform(x):
    id = x["input"]["id"]
    write = {
        "sql": "UPDATE book SET deleted = 1, seq = (SELECT COALESCE(MAX(seq), 0) + 1 FROM book) " +
               "WHERE id = ?1 AND deleted = 0",
        "params": [id],
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
