# oxplow.clojure.defn_count — count `defn` / `defn-` definitions across the
# Clojure tree. tree-sitter-clojure represents every form as a generic
# `list_lit`; a def is a list whose HEAD symbol is `defn`/`defn-`. Anchoring to
# the head (`.`) avoids counting a symbol that merely happens to be named `defn`
# elsewhere — e.g. a local binding `(let [defn 1] defn)`. Emits the repo-total
# ("tree:.") plus a per-file sample ("file:<path>", nonzero only).
def _clj_files():
    out = []
    for glob in ["**/*.clj", "**/*.cljs", "**/*.cljc"]:
        for f in files(glob):
            out.append((f["path"], f["text"]))
    return out

def transform(input):
    total = 0
    per_file = []
    for pair in _clj_files():
        c = 0
        for s in ast_query(pair[1], "clojure", "(list_lit . (sym_lit) @s)"):
            if s["text"] == "defn" or s["text"] == "defn-":
                c += 1
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + pair[0], "dims": {"language": "clojure"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "clojure"}}] + per_file}
