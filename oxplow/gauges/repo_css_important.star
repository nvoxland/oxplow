# repo.css_important — `!important` declarations (the specificity escape
# hatch, see .context/theming.md). Re-added single-concern (tsk61). CSS files
# are scanned whole-text (no AST grammar for css here); TS/TSX only inside
# string/template nodes, where an inline-style `!important` would live.
def transform(input):
    facts = []
    for f in files("**/*.css"):
        c = len(regex_find(r"!important", f["text"]))
        if c > 0:
            facts.append({"measure": "repo.css_important", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "css"}})
    q = "(string) @s (template_string) @s"
    for lang_glob in [("**/*.ts", "typescript"), ("**/*.tsx", "tsx")]:
        for f in files(lang_glob[0]):
            c = 0
            for s in ast_query(f["text"], lang_glob[1], q):
                c += len(regex_find(r"!important", s["text"]))
            if c > 0:
                facts.append({"measure": "repo.css_important", "value": c,
                              "subject": "file:" + f["path"], "path": f["path"],
                              "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
