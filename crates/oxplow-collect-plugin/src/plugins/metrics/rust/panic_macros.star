# oxplow.rust.panic_macros — count `panic!` / `unimplemented!` / `todo!` /
# `unreachable!` macro invocations (deliberate-abort sites). Matches both the
# bare form (`panic!`) and the path-qualified form (`std::panic!`,
# `core::todo!`), where the macro is a `scoped_identifier`. Emits the repo-total
# ("tree:.") plus a per-file sample ("file:<path>", nonzero only).
def transform(input):
    panicky = ["panic", "unimplemented", "todo", "unreachable"]
    total = 0
    per_file = []
    q = "(macro_invocation macro: (identifier) @name) " + \
        "(macro_invocation macro: (scoped_identifier name: (identifier) @name))"
    for f in files("**/*.rs"):
        c = 0
        for m in ast_query(f["text"], "rust", q):
            if m["text"] in panicky:
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + f["path"], "dims": {"language": "rust"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "rust"}}] + per_file}
