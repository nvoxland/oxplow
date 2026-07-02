# oxplow.rust.unsafe_blocks — count `unsafe { … }` blocks across the Rust tree.
# A tree-derived gauge: reads the snapshot via files() and the AST via
# ast_query(). Deterministic (no I/O) → observed. Emits the repo-total
# (subject "tree:.") plus one per-file sample (subject "file:<path>", nonzero
# only) so an effort's change can be attributed via its claimed files.
#
# Inverted substrate (epic tsk12): also emits a per-file `oxplow.ast_hit` FACT
# (rule="unsafe_block", value=the file's count) — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule. Dual-written until reads flip.
def transform(input):
    total = 0
    per_file = []
    facts = []
    for f in files("**/*.rs"):
        c = len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"oxplow.language": "rust"}})
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "unsafe_block", "subject": "file:" + f["path"], "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"oxplow.language": "rust"}}] + per_file, "facts": facts}
