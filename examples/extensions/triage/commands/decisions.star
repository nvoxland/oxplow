# triage/todos.decisions: the TODO comments, each with the decision a person made
# on it (or none). Decisions live in private state, not SQL, so a view
# reads them through this command rather than a lens.

# A command composes inside oxplow's transaction, where `metric_findings()`
# can't be computed, so it reads the facts themselves: the TODOs of the
# latest capture (each snapshot's run reads every source file).

def transform(x):
    snapshot = scope("state.read", {"scope": "project", "area": "state"})
    decisions = (snapshot or {}).get("values", {}).get("decisions", {})
    rows = scope("sql.read", {
        "sql": "SELECT path, line, rule AS kind FROM v_fact WHERE measure_key = 'triage.todos' "
            + "AND capture_id = (SELECT max(capture_id) FROM v_fact WHERE measure_key = 'triage.todos') "
            + "ORDER BY rule, path, line",
    })
    todos = [
        {"path": r["path"], "line": r["line"], "kind": r["kind"],
         "decision": decisions.get(r["path"] + ":" + str(r["line"]))}
        for r in rows
    ]
    return {"commands": [], "result": {"todos": todos, "decisions": decisions}}
