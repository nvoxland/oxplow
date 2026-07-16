# repo.ts_hooks — React hook call sites (useState/useEffect/useMemo/useCallback/
# useRef/useContext/useReducer/useLayoutEffect) — a re-render / complexity
# density signal, broader than repo.react_use_effect. One ast_query pass.
def _ts_files():
    out = []
    for f in files("**/*.ts"):
        out.append((f["path"], f["text"], "typescript"))
    for f in files("**/*.tsx"):
        out.append((f["path"], f["text"], "tsx"))
    return out

def transform(input):
    facts = []
    q = "(call_expression function: (identifier) @f)"
    hooks = ["useState", "useEffect", "useMemo", "useCallback", "useRef",
             "useContext", "useReducer", "useLayoutEffect"]
    for tri in _ts_files():
        c = 0
        for m in ast_query(tri[1], tri[2], q):
            if m["text"] in hooks:
                c += 1
        if c > 0:
            facts.append({"measure": "repo.ts_hooks", "value": c,
                          "subject": "file:" + tri[0], "path": tri[0],
                          "dims": {"oxplow.language": "typescript"}})
    return {"facts": facts}
