# repo.rust_clone — `.clone()` method calls (allocation/perf awareness).
# ONE ast_query pass over the .rs tree, mirroring the bundled idiom gauges: a
# gauge must stay inside the 5s Starlark SandboxBudget on a FULL-TREE scan, and
# two broad queries in one gauge does not (tsk42).
def transform(input):
    facts = []
    q = "(call_expression function: (field_expression field: (field_identifier) @m))"
    for f in files("**/*.rs"):
        c = 0
        for m in ast_query(f["text"], "rust", q):
            if m["text"] == "clone":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.rust_clone", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
