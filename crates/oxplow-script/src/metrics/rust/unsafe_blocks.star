# oxplow.rust.unsafe_blocks — count `unsafe { … }` blocks across the Rust tree.
# A tree-derived collector: reads the snapshot via files() and the AST via
# ast_query(). Deterministic (no I/O) → observed. Its facts are per-file
# (subject "file:<path>", nonzero only), so an effort's change attributes
# through its files.
#
# Inverted substrate (epic tsk12): a per-file `oxplow.ast_hit` FACT
# (rule="unsafe_block", value=the file's count) — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        c = len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
        if c > 0:
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "unsafe_block", "subject": "file:" + f["path"], "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
