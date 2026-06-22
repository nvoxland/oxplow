# oxplow.rust.panic_macros — count `panic!` / `unimplemented!` / `todo!` /
# `unreachable!` macro invocations (deliberate-abort sites).
def transform(input):
    panicky = ["panic", "unimplemented", "todo", "unreachable"]
    n = 0
    for f in files("**/*.rs"):
        q = "(macro_invocation macro: (identifier) @name)"
        for m in ast_query(f["text"], "rust", q):
            if m["text"] in panicky:
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "rust"}}]}
