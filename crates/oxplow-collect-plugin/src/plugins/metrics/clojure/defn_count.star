# oxplow.clojure.defn_count — count `defn` / `defn-` definitions across the
# Clojure tree. tree-sitter-clojure represents every form as a generic
# `list_lit`; a def is a list whose HEAD symbol is `defn`/`defn-`. Anchoring to
# the head (`.`) avoids counting a symbol that merely happens to be named `defn`
# elsewhere — e.g. a local binding `(let [defn 1] defn)`.
def _clj_files():
    out = []
    for glob in ["**/*.clj", "**/*.cljs", "**/*.cljc"]:
        for f in files(glob):
            out.append(f["text"])
    return out

def transform(input):
    n = 0
    for text in _clj_files():
        for s in ast_query(text, "clojure", "(list_lit . (sym_lit) @s)"):
            if s["text"] == "defn" or s["text"] == "defn-":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "clojure"}}]}
