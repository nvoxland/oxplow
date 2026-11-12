# shelf_db.book.add { title, author?, rating? }: insert a row.
# The sqlite trigger of the collector adds the model row.

def slug(text):
    out = ""
    dash = False
    for c in text.lower().elems():
        if c.isalnum():
            out += c
            dash = False
        elif not dash and out != "":
            out += "-"
            dash = True
    return out.rstrip("-")

def transform(x):
    i = x["input"]
    id = slug(i["title"])
    write = {
        "sql": "INSERT INTO book (id, title, author, rating) VALUES (?1, ?2, ?3, ?4)",
        "params": [id, i["title"], i.get("author"), i.get("rating")],
    }
    return {
        "commands": [{"name": "oxplow.extension_store.write", "input": {"batch": [write]}}],
        "events": [],
        "result": {"id": id},
    }
