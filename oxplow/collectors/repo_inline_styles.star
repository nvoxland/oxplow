# repo.inline_styles — `style={…}` JSX attributes (inline style-object
# density; this app leans on them heavily — a migration-pressure signal, see
# .context/theming.md). Re-added single-concern (tsk61). The attribute name is
# a property_identifier under jsx_attribute.
def transform(input):
    facts = []
    q = "(jsx_attribute (property_identifier) @p)"
    for f in files("**/*.tsx"):
        c = 0
        for p in ast_query(f["text"], "tsx", q):
            if p["text"] == "style":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.inline_styles", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
