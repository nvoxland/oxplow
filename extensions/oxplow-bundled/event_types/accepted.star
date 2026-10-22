# v1 named the effort by its URI (`oxplow:oxplow/effort?id=eff12`); v2
# carries the bare id (`eff12`), as every field named for one resource does.
def transform(x):
    payload = dict(x["payload"])
    payload["effort"] = uri.param(payload["effort"], "id")
    return payload
