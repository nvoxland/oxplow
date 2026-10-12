# triage/todos.in_file: the TODO comments in one file and what each asks for
# — a tool the `summarize` job's model calls to look closer at a file.

def transform(x):
    path = x["input"]["path"]
    rows = scope("sql.read", {
        "sql": "SELECT line, rule AS kind FROM metric_findings('triage.todos') WHERE path = :path ORDER BY line",
        "params": {"path": path},
    })
    return {"commands": [], "result": {"path": path, "todos": rows}}
