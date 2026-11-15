# Each independent setting is written at project level. A local workflow
# remains effective for this person; report it without rewriting it.
def transform(x):
    input = x["input"]
    tasks = {}
    commands = []
    for field, setting in [("workflow", "workflow"), ("definition_of_done", "definitionOfDone")]:
        if input.get(field) != None:
            tasks[setting] = input[field]
            commands.append({
                "name": "oxplow.config.set",
                "input": {"key": "oxtask.tasks." + setting, "value": input[field], "level": "project"},
            })
    result = {"tasks": tasks}
    if tasks.get("workflow") != None:
        local = scope("config.read", {"key": "oxtask.tasks.workflow", "level": "local"})
        if local["isSet"]:
            result["local_workflow"] = local["value"]
    return {"commands": commands, "result": result}
