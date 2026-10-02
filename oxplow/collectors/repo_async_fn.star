# repo.rust_async_fn — `async fn` definitions (async surface density). One
# ast_query pass over the .rs tree (see repo_shared_state.star for why). The
# `function_modifiers` node carries the async/const/unsafe/extern keywords.
def transform(input):
    facts = []
    q = "(function_modifiers) @m"
    for f in files("**/*.rs"):
        c = 0
        for m in ast_query(f["text"], "rust", q):
            if "async" in m["text"]:
                c += 1
        if c > 0:
            facts.append({"measure": "repo.rust_async_fn", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
