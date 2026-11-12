# shelf_folder.book.new { title, body? }: write shelf/<slug>.md.
# The files trigger of the collector adds the row.

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
    title = x["input"]["title"]
    body = x["input"].get("body", "")
    path = "shelf/" + slug(title) + ".md"
    return {
        "commands": [{"name": "oxplow.extension_files.write", "input": {"path": path, "body": "# " + title + "\n\n" + body + "\n"}}],
        "events": [],
        "result": {"path": path},
    }
