# oxplow.rust.unsafe_blocks — find each `unsafe { … }` blocks across the Rust tree.
# A tree-derived collector: reads the snapshot via files() and the AST via
# ast_query(). Deterministic (no I/O) → observed. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files.
#
# Inverted substrate: an `oxplow.ast_hit` FACT
# per file (rule="unsafe_block", its count, the lines in detail, severity warning) — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def _per_file(hits, rule, severity, language):
    """One fact per file: the count (what the metric sums) and the hits'
    lines in `detail` (what the change review matches to changed lines).
    One fact per hit would multiply what the metric cube folds."""
    facts = []
    for path in sorted(hits.keys()):
        lines = hits[path]
        facts.append({"measure": "oxplow.ast_hit", "value": len(lines), "rule": rule, "severity": severity, "detail": json.encode({"lines": lines}), "subject": uri.build("oxplow/file", path = path), "path": path, "dims": {"oxplow.language": language}})
    return facts

def transform(input):
    hits = {}
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", "(unsafe_block) @u"):
            hits.setdefault(f["path"], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "unsafe_block", "warning", "rust")}
