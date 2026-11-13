# Whether a request that changed files read a knowledge page, recorded each
# time one of its turns ends while none of its turns runs (a later turn
# records it again, replacing the earlier figure).
#
# Gets {"rows": [...]}: one row, {request, read, placed_at}, when the request
# has stopped and its turns changed files; none otherwise. Returns {"facts":
# [...], "capture": {at, replaces}}: 1 when an agent read a page in one of its
# turns, else 0, as one capture placed at the request's start that replaces
# the request's earlier one, so it counts once, in the period it started in.

def transform(x):
    if not x["rows"]:
        return {"facts": []}
    r = x["rows"][0]
    value = 1 if r["read"] else 0
    fact = {"measure": "knowledge.read_requests", "value": value, "subject": r["request"], "num": value, "den": 1}
    return {"facts": [fact], "capture": {"at": r["placed_at"], "replaces": r["request"]}}
