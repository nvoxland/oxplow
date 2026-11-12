# shelf-db.book.remove { id }: delete the row.
# The sqlite trigger of the collector removes the model row.

def transform(x):
    scope("store.sql", {"sql": "DELETE FROM book WHERE id = ?1", "params": [x["input"]["id"]]})
    return {"commands": [], "events": [], "result": {"id": x["input"]["id"]}}
