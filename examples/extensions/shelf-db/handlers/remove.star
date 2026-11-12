# shelf_db.book.remove { id }: delete the row.
# The sqlite trigger of the collector removes the model row.

def transform(x):
    id = x["input"]["id"]
    write = {"sql": "DELETE FROM book WHERE id = ?1", "params": [id]}
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
