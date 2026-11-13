# An agent's read of a knowledge page, as a row of the `knowledge_read_record`
# model (store `oxplow`): core logs `record.read` for every read of a row of
# a shared model (through `oxplow_read`, or its own read of the file a store
# keeps it in); this keeps the ones that are of knowledge pages.
#
# Gets {"rows": [...], "event": {...}}: one row {thread_uri} when the read's
# thread is known. Returns the read, keyed by what it read, when and how.

PAGES = "oxplow:oxplow/knowledge?"

def transform(x):
    e = x["event"]
    p = e["payload"]
    if not p["record"].startswith(PAGES):
        return {}
    thread = x["rows"][0]["thread_uri"] if x["rows"] else None
    turn = e["anchors"].get("turn_id")
    return {"models": [{"model": "oxplow/knowledge_read_record", "store": "oxplow", "upsert": [{
        "key": p["record"] + "|" + e["at"] + "|" + p["via"] + "|" + str(turn),
        "page": p["record"],
        "thread": thread,
        "turn_id": turn,
        "via": p["via"],
        "at": e["at"],
    }]}]}
