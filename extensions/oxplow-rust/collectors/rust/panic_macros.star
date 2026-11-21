# oxplow.rust.panic_macros — find each `panic!` / `unimplemented!` / `todo!` /
# `unreachable!` macro invocations (deliberate-abort sites). Matches both the
# bare form (`panic!`) and the path-qualified form (`std::panic!`,
# `core::todo!`), where the macro is a `scoped_identifier`. Its facts are per-file
# (subject "file:<path>", nonzero only), so a request's change attributes
# through its files. One `oxplow.ast_hit` FACT per hit
# (value 1, line, severity warning; rule="panic_macro") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule.
def transform(input):
    panicky = ["panic", "unimplemented", "todo", "unreachable"]
    facts = []
    q = "(macro_invocation macro: (identifier) @name) " + \
        "(macro_invocation macro: (scoped_identifier name: (identifier) @name))"
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", q):
            if m["text"] in panicky:
                facts.append({"measure": "oxplow.ast_hit", "value": 1, "rule": "panic_macro", "severity": "warning", "line": m["start_row"] + 1, "subject": uri.build("oxplow/file", path = f["path"]), "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
