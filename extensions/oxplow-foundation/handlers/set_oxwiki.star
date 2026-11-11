# oxplow.knowledge.set_oxwiki { on }: turn oxwiki — oxplow's own knowledge
# store — on or off for the project. Off, its pages stay recorded and are
# hidden, and its files are left alone; on again, they're back. It reads the
# settings' stamps and composes foundation's compare-and-swap write, so a
# change made since is refused, never overwritten. When the person's own
# layer says otherwise, the result names it (it still wins for them).

def transform(x):
    on = x["input"]["on"]
    read = scope("extension_config.read", {})
    # A write replaces the whole layer: the rest of the project's settings
    # are written back.
    layer = dict(read["layers"]["project"] or {})
    layer["oxwiki"] = on
    result = {"oxwiki": on}
    personal = (read["layers"]["personal"] or {}).get("oxwiki")
    if personal != None and personal != on:
        result["personal_oxwiki"] = personal
    return {
        "commands": [{
            "name": "oxplow.settings.write",
            "input": {
                "schema_version": read["schema_version"],
                "project": {"expected": read["project"], "values": layer},
                "personal": None,
            },
        }],
        "result": result,
    }
