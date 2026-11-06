# An effort's churn: the lines its change added and deleted, recorded as
# one fact when the effort finishes. Gets {"rows": [...]} — the effort's
# change files (`input`, bound to the finished effort) — and returns
# {"facts": [...]}. No rows (the change wasn't analyzed) is no fact.
def transform(x):
    if not x["rows"]:
        return {"facts": []}
    lines = 0
    for r in x["rows"]:
        lines += r["additions"] + r["deletions"]
    return {"facts": [{"measure": "oxplow_bundled.effort_churn_lines", "value": lines}]}
