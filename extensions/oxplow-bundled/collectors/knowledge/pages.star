# How many knowledge pages there are, and how many are stale, restated each
# time a page is written, recorded or deleted, and when a snapshot changes a
# file a page cites (`knowledge.page.drifted`: that is what makes a page
# stale; a snapshot of an uncited file can't move the counts).
#
# Gets {"rows": [...]}: one row per (store, kind) that has pages,
# {placed_at, day, store, kind, pages, stale}; one row with a NULL store
# when no enabled store has a page. Returns {"facts": [...], "capture": {at,
# replaces}}: a pages fact and a stale-pages fact per row, sliced by store and
# kind, as one capture that replaces the earlier capture of the same day, so
# a day holds its last count. With no page left the capture holds no fact, so
# the day's earlier counts go.

def transform(x):
    first = x["rows"][0]
    facts = []
    for r in x["rows"]:
        if r["store"] == None:
            continue
        dims = {"oxplow.knowledge_store": r["store"], "oxplow.knowledge_kind": r["kind"]}
        facts.append({"measure": "knowledge.pages", "value": r["pages"], "dims": dims})
        facts.append({"measure": "knowledge.stale_pages", "value": r["stale"], "num": r["stale"], "den": r["pages"], "dims": dims})
    return {"facts": facts, "capture": {"at": first["placed_at"], "replaces": "knowledge-counts:" + first["day"]}}
