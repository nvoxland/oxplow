# triage/todos.decide: record a person's call on one TODO comment in the
# extension's private state. The whole decisions map is one value; the
# write is a compare-and-swap on the stamp read here, so a decision made
# meanwhile by someone else refuses this one instead of being lost.

def transform(x):
    i = x["input"]
    snapshot = scope("state.read", {"scope": "project", "area": "state"})
    decisions = dict((snapshot or {}).get("values", {}).get("decisions", {}))
    decisions[i["path"] + ":" + str(i["line"])] = i["decision"]
    return {
        "commands": [{
            "name": "triage.state.set",
            "input": {
                "scope": "project",
                "area": "state",
                "expected": snapshot["stamp"] if snapshot else None,
                "schema_version": 1,
                "values": {"decisions": decisions},
            },
        }],
        "result": {"path": i["path"], "line": i["line"], "decision": i["decision"]},
    }
