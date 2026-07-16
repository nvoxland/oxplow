# repo.skipped_tests — skipped / ignored tests (hidden test debt no pass/fail
# counter catches). Rust `#[ignore]` / `#![ignore]` attributes + TS `.skip` /
# `.todo` test-runner markers (scanned only in *.test.* files to avoid matching
# unrelated `.skip`/`.todo` property names). Node kinds mirror repo_allow.star.
def transform(input):
    facts = []
    # Rust: #[ignore] (attribute_item) and #![ignore] (inner_attribute_item).
    rq = "(attribute_item (attribute (identifier) @a)) " + \
         "(inner_attribute_item (attribute (identifier) @a))"
    for f in files("**/*.rs"):
        c = 0
        for a in ast_query(f["text"], "rust", rq):
            if a["text"] == "ignore":
                c += 1
        if c > 0:
            facts.append({"measure": "repo.skipped_tests", "value": c,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": "rust"}})
    # TS/JS: `.skip` / `.todo` markers, in test files only.
    tq = "(property_identifier) @p"
    for pat in ["**/*.test.ts", "**/*.test.tsx"]:
        for f in files(pat):
            lang = "tsx" if f["path"].endswith(".tsx") else "typescript"
            c = 0
            for p in ast_query(f["text"], lang, tq):
                if p["text"] == "skip" or p["text"] == "todo":
                    c += 1
            if c > 0:
                facts.append({"measure": "repo.skipped_tests", "value": c,
                              "subject": "file:" + f["path"], "path": f["path"],
                              "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
