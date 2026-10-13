# triage/todos.decisions: the TODO comments, each with the decision a person made
# on it (or none). Decisions live in private state, not SQL, so a view
# reads them through this command rather than a lens.

def transform(x):
    snapshot = scope("state.read", {"scope": "project", "area": "state"})
    decisions = (snapshot or {}).get("values", {}).get("decisions", {})
    rows = scope("sql.read", {
        "sql": "SELECT path, line, rule AS kind FROM metric_findings('triage.todos') ORDER BY rule, path, line",
    })
    todos = [
        {"path": r["path"], "line": r["line"], "kind": r["kind"],
         "decision": decisions.get(r["path"] + ":" + str(r["line"]))}
        for r in rows
    ]
    return {"commands": [], "result": {"todos": todos, "decisions": decisions}}
