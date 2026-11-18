# oxplow.duplicate_lines — lines in duplicated blocks, across every supported
# language. Runs over the whole tree (a slice can't say what's duplicated in
# it), so its capture restates the tree: a clean tree records no facts and
# clears the metric.
#
# One fact per side of each duplicate block: its line count, on the block,
# with where its copy is (`detail`: `path:start-end`).
MIN_LINES = 5

def transform(input):
    facts = []
    for b in duplicate_blocks(MIN_LINES):
        for side, other in [("a", "b"), ("b", "a")]:
            path = b[side + "_path"]
            start = b[side + "_start_line"]
            end = b[side + "_end_line"]
            copy = "%s:%d-%d" % (b[other + "_path"], b[other + "_start_line"], b[other + "_end_line"])
            facts.append({
                "measure": "oxplow.duplicate_lines",
                "value": b["line_count"],
                "subject": uri.build("oxplow/block", path = path, lines = "%d-%d" % (start, end)),
                "path": path,
                "line": start,
                "detail": copy,
            })
    return {"facts": facts}
