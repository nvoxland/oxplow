# oxplow.csharp.method_count — total method declarations across the C# tree (a
# coarse size/structure gauge). Counts `method_declaration` nodes; constructors,
# local functions and lambdas are not included.
def transform(input):
    n = 0
    for f in files("**/*.cs"):
        n += len(ast_query(f["text"], "csharp", "(method_declaration) @m"))
    return {"samples": [{"value": n, "dims": {"language": "csharp"}}]}
