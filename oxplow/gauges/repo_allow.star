# repo.rust_allow — `#[allow(...)]` attributes: Rust lints being hidden.
# One ast_query pass over the .rs tree (see repo_clone.star for why).
#
# BOTH node kinds: `#[allow(...)]` is an `attribute_item`, `#![allow(...)]` is an
# `inner_attribute_item`. Naming only the first undercounts crate/module-level
# suppressions — the ones that matter most, since an inner attribute mutes a lint
# for a whole file (tsk44). Node names pinned by a test in oxplow-code-metrics.
def transform(input):
    facts = []
    q = "(attribute_item (attribute (identifier) @a)) " + \
        "(inner_attribute_item (attribute (identifier) @a))"
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
