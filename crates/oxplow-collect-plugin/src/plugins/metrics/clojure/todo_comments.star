# oxplow.clojure.todo_comments — count TODO / FIXME markers in Clojure
# comments (`;` line comments parse as `comment` nodes). Emits the repo-total
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
        for cm in ast_query(pair[1], "clojure", "(comment) @c"):
            c += len(regex_find(r"(?i)\b(TODO|FIXME)\b", cm["text"]))
        total += c
        if c > 0:
            per_file.append({"value": c, "subject": "file:" + pair[0], "dims": {"language": "clojure"}})
    return {"samples": [{"value": total, "subject": "tree:.", "dims": {"language": "clojure"}}] + per_file}
