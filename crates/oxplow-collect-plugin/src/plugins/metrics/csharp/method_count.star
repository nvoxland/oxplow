# oxplow.csharp.method_count — total method declarations across the C# tree (a
# coarse size/structure gauge). Counts `method_declaration` nodes; constructors,
# local functions and lambdas are not included. Emits the repo-total ("tree:.")
# plus a per-file sample ("file:<path>", nonzero only) for effort attribution.
def transform(input):
    total = 0
    per_file = []
    for f in files("**/*.cs"):
        c = len(ast_query(f["text"], "csharp", "(method_declaration) @m"))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "csharp"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "csharp"}}] + per_file}
