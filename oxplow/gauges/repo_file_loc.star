# repo.file_loc — lines of code per source file (.rs/.ts/.tsx). Cheapest
# possible full-tree scan: a line count, no AST. Feeds two metrics —
# repo.total_loc (sum) and repo.max_file_loc (max, a god-file / generated-blob
# detector). One fact per file (every file has >= 1 line).
def _lang(path):
    if path.endswith(".rs"):
        return "rust"
    return "typescript"

def transform(input):
    facts = []
    for pat in ["**/*.rs", "**/*.ts", "**/*.tsx"]:
        for f in files(pat):
            n = len(f["text"].split("\n"))
            facts.append({"measure": "repo.file_loc", "value": n,
                          "subject": "file:" + f["path"], "path": f["path"],
                          "dims": {"oxplow.language": _lang(f["path"])}})
    return {"facts": facts}
