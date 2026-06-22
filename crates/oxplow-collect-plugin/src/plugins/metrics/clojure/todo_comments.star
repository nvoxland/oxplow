# oxplow.clojure.todo_comments — count TODO / FIXME markers in Clojure
# comments (`;` line comments parse as `comment` nodes).
def _clj_files():
    out = []
    for glob in ["**/*.clj", "**/*.cljs", "**/*.cljc"]:
        for f in files(glob):
            out.append(f["text"])
    return out

def transform(input):
    n = 0
    for text in _clj_files():
        for c in ast_query(text, "clojure", "(comment) @c"):
            n += len(regex_find(r"(?i)\b(TODO|FIXME)\b", c["text"]))
    return {"samples": [{"value": n, "dims": {"language": "clojure"}}]}
