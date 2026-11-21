# oxplow.rust.unsafe_blocks — find each `unsafe { … }` blocks across the Rust tree.
# A tree-derived collector: reads the snapshot via files() and the AST via
# ast_query(). Deterministic (no I/O) → observed. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files.
#
# Inverted substrate: an `oxplow.ast_hit` FACT
# per hit (rule="unsafe_block", value 1, line, severity warning) — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", "(unsafe_block) @u"):
            facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "unsafe_block", "severity": "warning", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = f["path"]), "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
