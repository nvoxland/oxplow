# effort_review/effort.evidence: what oxplow collected about one effort, in
# one answer a model can read. Every list is capped (the job's tool answers
# are limited to 64 KiB); `truncated` names the ones that were cut.

CAP = 50

def read(sql, effort_id, limit):
    return scope("sql.read", {"sql": sql + " LIMIT " + str(limit + 1), "params": {"id": effort_id}})

def transform(x):
    effort = x["input"]["effort"]
    if not effort.startswith("eff"):
        return {"refuse": "`effort` is an effort's id, like `eff12`"}
    ids = scope("sql.read", {"sql": "SELECT id FROM v_effort WHERE uri = :uri",
                             "params": {"uri": uri.build("oxplow/effort", id = effort)}})
    if not ids:
        return {"refuse": "no effort " + effort}
    effort_id = ids[0]["id"]
    efforts = read(
        "SELECT work_item, title, summary, started_at, ended_at, closed_by FROM v_effort WHERE id = :id",
        effort_id, 1)
    if not efforts:
        return {"refuse": "no effort " + effort}
    truncated = []

    def capped(name, rows, limit):
        if len(rows) > limit:
            truncated.append(name)
            return rows[:limit]
        return rows

    # The effort's change, when change analysis has computed it.
    change = "(SELECT max(id) FROM v_change WHERE kind = 'effort' AND target = CAST(:id AS TEXT))"
    changed = capped("files", read(
        "SELECT e.path, e.change_kind, e.source, cf.additions, cf.deletions, cf.is_test "
        + "FROM v_effort_file e LEFT JOIN v_change_file cf ON cf.path = e.path AND cf.change_id = " + change
        + " WHERE e.effort_id = :id ORDER BY e.path", effort_id, CAP), CAP)
    functions = capped("functions", read(
        "SELECT path, container, name, status, signature_changed, complexity, complexity_delta, length_delta, is_test "
        + "FROM v_change_function WHERE change_id = " + change
        + " ORDER BY abs(coalesce(complexity_delta, 0)) DESC, path, name", effort_id, CAP), CAP)
    test_runs = capped("test_runs", read(
        "SELECT command, exit_code, passed, failed, skipped, total, provenance, captured_at "
        + "FROM v_test_run WHERE effort_id = :id ORDER BY id DESC", effort_id, 20), 20)
    # The latest observation of each kind; its detail (test cases, the
    # covered and uncovered lines) cut to 2000 characters.
    observations = read(
        "SELECT kind, provenance, metric_value, substr(payload_json, 1, 2000) AS detail, created_at "
        + "FROM v_effort_observation o WHERE effort_id = :id AND seq = "
        + "(SELECT max(seq) FROM v_effort_observation WHERE effort_id = o.effort_id AND kind = o.kind) "
        + "ORDER BY kind", effort_id, 20)
    metrics = capped("metrics", read(
        "SELECT key, title, direction, baseline, current, delta FROM v_effort_metric_delta "
        + "WHERE effort_id = :id AND changed = 1 ORDER BY abs(delta) DESC", effort_id, 20), 20)
    findings = capped("findings", read(
        "SELECT tool, path, start_line, end_line, kind, metric_value FROM v_code_quality_finding "
        + "WHERE scan_id = (SELECT max(id) FROM v_code_quality_scan) "
        + "AND path IN (SELECT path FROM v_effort_file WHERE effort_id = :id) "
        + "ORDER BY path, start_line", effort_id, CAP), CAP)
    return {"commands": [], "result": {
        "effort": effort,
        "workItem": efforts[0]["work_item"],
        "title": efforts[0]["title"],
        "summary": efforts[0]["summary"],
        "closedBy": efforts[0]["closed_by"],
        "files": changed,
        "functions": functions,
        "testRuns": test_runs,
        "observations": observations,
        "metrics": metrics,
        "findings": findings,
        "truncated": truncated,
    }}
