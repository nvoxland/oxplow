# repo.focused_tests — `.only` focused tests (a committed `it.only` /
# `describe.only` silently disables the rest of the suite). Scanned in *.test.*
# files only. Match `.only` as a MEMBER access, not a bare property_identifier —
# else object keys like `{ only: [...] }` false-positive (they're pairs, not
# member expressions).
def transform(input):
    facts = []
    q = "(member_expression property: (property_identifier) @p)"
    for pat in ["**/*.test.ts", "**/*.test.tsx"]:
        for f in files(pat):
            lang = "tsx" if f["path"].endswith(".tsx") else "typescript"
            c = 0
            for p in ast_query(f["text"], lang, q):
                if p["text"] == "only":
                    c += 1
            if c > 0:
                facts.append({"measure": "repo.focused_tests", "value": c,
                              "subject": "file:" + f["path"], "path": f["path"],
                              "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
