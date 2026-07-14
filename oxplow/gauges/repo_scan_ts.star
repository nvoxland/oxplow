# repo_scan_ts — TypeScript code-health gauge (tsk40). ast_query over each
# .ts/.tsx file, regex only on small comment nodes (not whole files) so it
# stays within the Starlark sandbox budget over the whole tree.
#   - repo.react_use_effect: `useEffect(...)` call sites (re-render signal).
#   - repo.eslint_disable: `eslint-disable` directives (lints being hidden).

def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    call_q = "(call_expression function: (identifier) @f)"
    for tri in _ts_files():
        p = tri[0]
        t = tri[1]
        lang = tri[2]

        use_effect = 0
        for m in ast_query(t, lang, call_q):
            if m["text"] == "useEffect":
                use_effect += 1
        if use_effect > 0:
            facts.append({"measure": "repo.react_use_effect", "value": use_effect,
                          "subject": "file:" + p, "path": p,
                          "dims": {"oxplow.language": "typescript"}})

        eslint = 0
        for cm in ast_query(t, lang, "(comment) @c"):
            eslint += len(regex_find(r"eslint-disable", cm["text"]))
        if eslint > 0:
            facts.append({"measure": "repo.eslint_disable", "value": eslint,
                          "subject": "file:" + p, "path": p,
                          "dims": {"oxplow.language": "typescript"}})

    return {"facts": facts}
