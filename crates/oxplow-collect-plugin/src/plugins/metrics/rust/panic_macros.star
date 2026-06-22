# oxplow.rust.panic_macros — count `panic!` / `unimplemented!` / `todo!` /
# `unreachable!` macro invocations (deliberate-abort sites). Matches both the
# bare form (`panic!`) and the path-qualified form (`std::panic!`,
# `core::todo!`), where the macro is a `scoped_identifier`.
def transform(input):
    panicky = ["panic", "unimplemented", "todo", "unreachable"]
    n = 0
    q = "(macro_invocation macro: (identifier) @name) " + \
        "(macro_invocation macro: (scoped_identifier name: (identifier) @name))"
    for f in files("**/*.rs"):
        for m in ast_query(f["text"], "rust", q):
            if m["text"] in panicky:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
