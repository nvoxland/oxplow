# triage/todos.in_file: the TODO comments in one file and what each asks for
# — a tool the `summarize` job's model calls to look closer at a file.

# A command composes inside oxplow's transaction, where `metric_findings()`
# can't be computed, so it reads the facts themselves: the TODOs of the
# latest capture (each snapshot's run reads every source file).

def transform(x):
    path = x["input"]["path"]
    rows = scope("sql.read", {
        "sql": "SELECT line, rule AS kind FROM v_fact WHERE measure_key = 'triage.todos' AND path = :path "
            + "AND capture_id = (SELECT max(capture_id) FROM v_fact WHERE measure_key = 'triage.todos') "
            + "ORDER BY line",
        "params": {"path": path},
    })
    return {"commands": [], "result": {"path": path, "todos": rows}}
