# oxtask/repair-disabled: when an extension's contribution is disabled
# after repeated failures, file one task asking for it to be repaired, and
# comment on that task when the contribution is disabled again while it is
# still open. oxplow never sends the task to an agent; a person does.
#
# The task is the contribution's: it is linked `about` the contribution's
# ref, which is what the open one is found by (and what
# `v_oxplow_contribution_repair` shows). Its title is for people.

_REASON_MAX = 80
_ERRORS_SHOWN = 5

# The contribution's open repair task: one of this store's, linked `about` it,
# not done or canceled.
_STATE = """
SELECT (SELECT w.native_id FROM v_oxplow_task_link l JOIN v_oxplow_task w ON w.uri = l.from_uri
         WHERE l.to_uri = :contribution AND l.link_type = 'about'
           AND w.provider = 'oxtask'
           AND w.state NOT IN ('done', 'canceled')
         ORDER BY w.uri DESC LIMIT 1) AS open_item
"""

# Where it is declared, and its recent failures, newest first: a
# collector's failed syncs, an effect's failed reactions (each reaction at
# its latest attempt: a failure a retry got past isn't one any more).
_FAILURES = """
SELECT (SELECT file || ':' || coalesce(line, 1) FROM v_contribution WHERE uri = :uri) AS declared,
       (SELECT extension FROM v_contribution WHERE uri = :uri) AS extension,
       (SELECT json_group_array(error) FROM (
          SELECT json_extract(payload, '$.error') AS error FROM v_event
           WHERE :kind = 'collector' AND type = 'collector.synced'
             AND json_extract(payload, '$.collector') = :namespace || '/' || :contribution
             AND json_extract(payload, '$.status') = 'error'
             AND json_extract(payload, '$.error') IS NOT NULL
           ORDER BY seq DESC LIMIT 5)) AS collector_errors,
       (SELECT json_group_array(error) FROM (
          SELECT error FROM v_effect_run
           WHERE :kind = 'effect' AND effect = :namespace || '/' || :contribution
             AND outcome = 'failed' AND error IS NOT NULL AND latest = 1
           ORDER BY id DESC LIMIT 5)) AS effect_errors,
       (SELECT last_error FROM v_contribution_health
         WHERE namespace = :namespace AND contribution = :contribution AND kind = :kind) AS last_error
"""

def _list(text):
    return json.decode(text) if text else []

def _title(prefix, reason):
    first = reason.split("\n")[0]
    if len(first) > _REASON_MAX:
        first = first[:_REASON_MAX].rstrip() + "…"
    return "%s %s" % (prefix, first)

def _quote(reason):
    return "\n".join(["> " + md_text(line) for line in reason.split("\n")])

# An effect's commands run in one transaction, and a store's own commands
# write outside it, so the effect writes oxtask's file the way those
# commands do: through the extension store's write.
def _write(batch):
    return {"name": "oxplow.extension_store.write", "input": {"batch": batch}}

def transform(x):
    event = x["event"]
    p = event["payload"]
    named = uri.contribution(p["contribution"])
    # A contribution is named in its namespace; the extension that
    # declares it now is what a person checks and tests.
    namespace, contribution, kind = named["namespace"], named["id"], named["kind"]
    reason = p["reason"]
    prefix = "Repair %s %s:" % (namespace, contribution)

    state = scope("sql.read", {"sql": _STATE, "params": {"contribution": p["contribution"]}})[0]
    found = scope("sql.read", {
        "sql": _FAILURES,
        "params": {"uri": p["contribution"], "namespace": namespace, "contribution": contribution, "kind": kind},
    })[0]
    extension = found["extension"] or namespace
    errors = _list(found["collector_errors"]) + _list(found["effect_errors"])
    if not errors and found["last_error"]:
        errors = [found["last_error"]]

    if state["open_item"]:
        lines = ["Disabled again:", "", _quote(reason)]
        if len(errors) > 1:
            lines.extend(["", "Recent failures (newest first):", ""])
            lines.extend(["- " + md_text(e) for e in errors[:_ERRORS_SHOWN]])
        return {"commands": [_write([{
            "sql": "INSERT INTO task_comment (task_id, author_kind, body) VALUES (?1, 'system', ?2)",
            "params": [state["open_item"], "\n".join(lines)],
            "expect_changes": 1,
        }])]}

    body = [
        "The %s `%s` of the extension `%s` was disabled on this machine after it kept failing:" % (kind, contribution, extension),
        "",
        _quote(reason),
    ]
    if found["declared"]:
        body.extend(["", "## Where it is declared", "", "`%s`" % found["declared"]])
    if errors:
        body.extend(["", "## Recent failures (newest first)", ""])
        body.extend(["- " + md_text(e) for e in errors[:_ERRORS_SHOWN]])
    body.extend([
        "",
        "## What to do",
        "",
        "1. Find why it fails: read the failures and its source (its intent is in the manifest), and reproduce one (`oxplow extension test %s`)." % extension,
        "2. Fix it, keeping its purpose and examples.",
        "3. Check it: `oxplow extension check %s` and `oxplow extension test %s` must be clean." % (extension, extension),
        "4. Say what you changed on this task. A person enables it again (`oxplow.contribution.enable`, Settings → Extensions); you can't.",
    ])
    return {"commands": [_write([
        {
            "sql": "INSERT INTO task (title, body, stage, author_kind) VALUES (?1, ?2, 'ready', 'system')",
            "params": [_title(prefix, reason), "\n".join(body)],
            "expect_changes": 1,
        },
        # The link goes to the task just inserted (the highest number: the
        # write is one transaction, and numbers only grow).
        {
            "sql": "INSERT INTO task_link (task_id, to_uri, type) SELECT id, ?1, 'about' FROM task ORDER BY n DESC LIMIT 1",
            "params": [p["contribution"]],
            "expect_changes": 1,
        },
    ])]}
