# oxplow.tasks.set_workflow { workflow?, definition_of_done? }: how this
# project's agents use tasks (oxtask's workflow preset) and what "done"
# means here, written to the project's settings. It reads the settings'
# stamps and composes the extension's compare-and-swap write, so a change
# made since is refused, never overwritten. When the person's own layer
# sets the workflow too, the result names it (it still wins for them).

def transform(x):
    input = x["input"]
    tasks = {}
    if input.get("workflow") != None:
        tasks["workflow"] = input["workflow"]
    if input.get("definition_of_done") != None:
        tasks["definitionOfDone"] = input["definition_of_done"]
    # The read gives the stamps at its top level (`schema_version`,
    # `project`) and each layer as stored (`layers`): a write replaces the
    # whole layer, so the rest of the project's settings are written back.
    read = scope("extension_config.read", {})
    layer = dict(read["layers"]["project"] or {})
    merged = dict(layer.get("tasks") or {})
    merged.update(tasks)
    layer["tasks"] = merged
    result = {"tasks": tasks}
    # A person's own workflow still wins for them: say so.
    personal = (read["layers"]["personal"] or {}).get("tasks") or {}
    if tasks.get("workflow") != None and personal.get("workflow") != None:
        result["personal_workflow"] = personal["workflow"]
    return {
        "commands": [{
            "name": "oxplow.review_policy.write",
            "input": {
                "schema_version": read["schema_version"],
                "project": {"expected": read["project"], "values": layer},
                "personal": None,
            },
        }],
        "result": result,
    }
