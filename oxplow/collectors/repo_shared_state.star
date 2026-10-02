# repo.rust_shared_state — Arc / Mutex / RwLock type usages (shared-state /
# concurrency-complexity signal). ONE ast_query pass over the .rs tree, mirroring
# the bundled idiom gauges: a gauge must stay inside the Starlark SandboxBudget on
# a FULL-TREE scan, and two broad queries in one gauge does not (tsk42).
def transform(input):
    facts = []
    q = "(type_identifier) @t"
    names = ["Arc", "Mutex", "RwLock"]
    for f in files("**/*.rs"):
        c = 0
        for t in ast_query(f["text"], "rust", q):
            if t["text"] in names:
                c += 1
        if c > 0:
            facts.append({"measure": "repo.rust_shared_state", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
