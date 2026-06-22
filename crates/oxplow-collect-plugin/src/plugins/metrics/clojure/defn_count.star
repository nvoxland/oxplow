# oxplow.clojure.defn_count — count `defn` / `defn-` definitions across the
# Clojure tree. tree-sitter-clojure represents every form as a generic
# `list_lit`; a def appears as a `sym_lit` head whose text is `defn`/`defn-`,
# so counting those symbols counts the definitions.
def _clj_files():
    out = []
    for glob in ["**/*.clj", "**/*.cljs", "**/*.cljc"]:
        for f in files(glob):
            out.append(f["text"])
    return out

def transform(input):
    n = 0
    for text in _clj_files():
        for s in ast_query(text, "clojure", "(sym_lit) @s"):
            if s["text"] == "defn" or s["text"] == "defn-":
                n += 1
    return {"samples": [{"value": n, "dims": {"language": "clojure"}}]}
