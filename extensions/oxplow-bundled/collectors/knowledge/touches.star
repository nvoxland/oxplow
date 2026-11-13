# Which thread wrote a knowledge page, kept as a row of the
# `knowledge_touch_record` model (store `oxplow`), one per (thread, page) with
# the last time: the Work panel's Finished list shows the pages a thread
# touched. A store's command writes the page inside the thread's run (its
# change event carries the thread), while a person's edit of the file or a
# rescan has no thread, so a change with no thread touches nothing.
#
# Gets {"rows": [...], "event": {...}}: one row {thread_uri} when the
# change's thread is known.

def transform(x):
    if not x["rows"]:
        return {}
    e = x["event"]
    page = e["payload"]["uri"]
    thread = x["rows"][0]["thread_uri"]
    return {"models": [{"model": "oxplow/knowledge_touch_record", "store": "oxplow", "upsert": [{
        "key": thread + "|" + page,
        "page": page,
        "thread": thread,
        "last_seen": e["at"],
    }]}]}
