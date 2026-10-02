# oxplow.rust.panic_macros — count `panic!` / `unimplemented!` / `todo!` /
# `unreachable!` macro invocations (deliberate-abort sites). Matches both the
# bare form (`panic!`) and the path-qualified form (`std::panic!`,
# `core::todo!`), where the macro is a `scoped_identifier`. Its facts are per-file
# (subject "file:<path>", nonzero only), so an effort's change attributes
# through its files. and a per-file
# `oxplow.ast_hit` FACT (rule="panic_macro") — the metric is the SPEC
# Sum(oxplow.ast_hit) filtered by that rule (epic tsk12).
def transform(input):
    panicky = ["panic", "unimplemented", "todo", "unreachable"]
    facts = []
    q = "(macro_invocation macro: (identifier) @name) " + \
        "(macro_invocation macro: (scoped_identifier name: (identifier) @name))"
    for f in files("**/*.rs"):
        c = 0
        for m in ast_query(f["text"], "rust", q):
            if m["text"] in panicky:
                c += 1
        if c > 0:
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "panic_macro", "subject": "file:" + f["path"], "path": f["path"], "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
