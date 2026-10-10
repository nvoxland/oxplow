# Each TODO comment in the snapshot's source files, classified, and the
# number of open work items beside them.

KINDS = ["bug", "cleanup", "feature"]

def transform(input):
    rows = scope("sql.read", {
        "sql": "SELECT count(*) AS n FROM v_work_item WHERE state NOT IN ('done', 'canceled')",
    })
    facts = [{"measure": "triage.open_items", "value": rows[0]["n"]}]
    for f in source_files():
        for m in markers(f["text"], f["language"]):
            if m["kind"] != "TODO":
                continue
            kind = ai_classify(m["text"], KINDS)["label"]
            facts.append({
                "measure": "triage.todos",
                "value": 1,
                "subject": "file:" + f["path"],
                "path": f["path"],
                "line": m["line"],
                "rule": kind,
            })
    return {"facts": facts}
