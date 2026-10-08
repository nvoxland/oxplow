# oxplow.doc_coverage — % of PUBLIC functions/methods that carry a doc comment
# (or, for Python/Clojure, a docstring), across EVERY supported language.
# Language-agnostic: sweeps source_files() and uses code_metrics()'s has_doc +
# visibility (per-language doc detection lives in oxplow-code-metrics, not here).
#
# One per-FILE ratio FACT on oxplow.doc_coverage: num = documented public items,
# den = public items, value = %. A file with no public items emits nothing
# (0/0 is "no data", not "0% documented"). The metric SPEC re-derives the
# headline as Sum(num)/Sum(den) over the tree.
def transform(input):
    facts = []
    for f in source_files():
        lang = f["language"]
        public = 0
        documented = 0
        for m in code_metrics(f["text"], lang):
            if m["visibility"] == "public":
                public += 1
                if m["has_doc"]:
                    documented += 1
        if public > 0:
            facts.append({
                "measure": "oxplow.doc_coverage",
                "value": documented * 100.0 / public,
                "num": documented,
                "den": public,
                "subject": "file:" + f["path"],
                "path": f["path"],
                "dims": {"oxplow.language": lang},
            })
    return {"facts": facts}
