# repo_scan_rust — Rust code-health gauge (tsk40). ast_query (native
# tree-sitter) over each .rs file, one per-file fact per concern; the repo.*
# specs sum them. Uses the AST (not whole-file regex) so it stays within the
# Starlark sandbox's wall-clock budget over the whole tree.
#   - repo.rust_clone: `.clone()` method calls (allocation/perf awareness).
#   - repo.rust_allow: `#[allow(...)]` attributes (lints being hidden).

def transform(input):
    facts = []
    clone_q = "(call_expression function: (field_expression field: (field_identifier) @m))"
    allow_q = "(attribute_item (attribute (identifier) @a))"
    for f in files("**/*.rs"):
        p = f["path"]
        t = f["text"]

        clones = 0
        for m in ast_query(t, "rust", clone_q):
            if m["text"] == "clone":
                clones += 1
        if clones > 0:
            facts.append({"measure": "repo.rust_clone", "value": clones,
                          "subject": "file:" + p, "path": p,
                          "dims": {"oxplow.language": "rust"}})

        allows = 0
        for a in ast_query(t, "rust", allow_q):
            if a["text"] == "allow":
                allows += 1
        if allows > 0:
            facts.append({"measure": "repo.rust_allow", "value": allows,
                          "subject": "file:" + p, "path": p,
                          "dims": {"oxplow.language": "rust"}})

    return {"facts": facts}
