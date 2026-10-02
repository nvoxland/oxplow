# Gets {"rows": [...]} from the collector's `input` query: every open task
# with `last_touched_at` (its latest edit, comment activity or effort) and
# `days_idle` (whole days since then, computed by the query — the sandbox
# has no clock). Keeps the ones idle for a week or more.
STALE_AFTER_DAYS = 7

def transform(x):
    stale = []
    for r in x["rows"]:
        days = r.get("days_idle")
        if days == None or days < STALE_AFTER_DAYS:
            continue
        stale.append({
            "id": r["id"],
            "title": r["title"],
            "status": r["status"],
            "priority": r["priority"],
            "thread_id": r.get("thread_id"),
            "stream_id": r.get("stream_id"),
            "last_touched_at": r["last_touched_at"],
            "days_idle": days,
        })
    return {"entities": {"stale_task": stale}}
