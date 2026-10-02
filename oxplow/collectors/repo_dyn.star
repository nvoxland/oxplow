# repo.rust_dyn — `dyn Trait` dynamic-dispatch usages (Box<dyn>, &dyn — an
# indirection / arch signal, the runtime-dispatch counterpart to generics).
# One ast_query pass over the .rs tree; `dyn Trait` is a `dynamic_type` node.
def transform(input):
    facts = []
    q = "(dynamic_type) @d"
    for f in files("**/*.rs"):
        c = len(ast_query(f["text"], "rust", q))
        if c > 0:
            facts.append({"measure": "repo.rust_dyn", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
