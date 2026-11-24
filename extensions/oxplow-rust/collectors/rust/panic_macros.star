# oxplow.rust.panic_macros — find each `panic!` / `unimplemented!` / `todo!` /
# `unreachable!` macro invocations (deliberate-abort sites). Matches both the
# bare form (`panic!`) and the path-qualified form (`std::panic!`,
# `core::todo!`), where the macro is a `scoped_identifier`. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per file, the hits' lines in its detail
# (value 1, line, severity warning; rule="panic_macro") — the metric is the SPEC
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
    panicky = ["panic", "unimplemented", "todo", "unreachable"]
    hits = {}
    q = "(macro_invocation macro: (identifier) @name) " + \
        "(macro_invocation macro: (scoped_identifier name: (identifier) @name))"
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", q):
            if m["text"] in panicky:
                hits.setdefault(f["path"], []).append(m["start_row"] + 1)
    return {"facts": _per_file(hits, "panic_macro", "warning", "rust")}
