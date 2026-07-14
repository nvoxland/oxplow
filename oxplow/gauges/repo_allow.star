# repo.rust_allow — `#[allow(...)]` attributes: Rust lints being hidden.
# One ast_query pass over the .rs tree (see repo_clone.star for why).
def transform(input):
    facts = []
    q = "(attribute_item (attribute (identifier) @a))"
    for f in files("**/*.rs"):
        c = 0
        for a in ast_query(f["text"], "rust", q):
            if a["text"] == "allow":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.rust_allow", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    return {"facts": facts}
