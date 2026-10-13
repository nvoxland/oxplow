# oxplow-bundled/repair-disabled: when an extension's contribution is
# disabled after repeated failures, file one work item asking for it to be
# repaired — on the active work list, whichever provider that is — and
# comment on that item when the contribution is disabled again while it is
# still open. oxplow never sends the item to an agent; a person does.
#
# The item is the contribution's: its title starts `Repair <extension>
# <contribution>:`, which is what the open one is found by (and what
# `v_contribution_health.repair_item` shows).

_REASON_MAX = 80
_ERRORS_SHOWN = 5

# The active work list's provider, when one is running (`none` files
# nowhere), and the contribution's open repair item.
_STATE = """
SELECT (SELECT provider FROM v_capability_provider
         WHERE capability = 'work_items' AND active = 1 AND available = 1
           AND provider <> 'none') AS provider,
       (SELECT ref FROM v_work_item
         WHERE state NOT IN ('done', 'canceled')
           AND substr(title, 1, length(:prefix)) = :prefix
         ORDER BY ref DESC LIMIT 1) AS open_item
"""

# Where it is declared, and its recent failures, newest first: a
# collector's failed syncs, an effect's failed reactions (each reaction at
# its latest attempt: a failure a retry got past isn't one any more).
_FAILURES = """
SELECT (SELECT file || ':' || coalesce(line, 1) FROM v_contribution
         WHERE extension = :extension AND kind = :kind AND local = :contribution) AS declared,
       (SELECT json_group_array(error) FROM (
          SELECT json_extract(payload, '$.error') AS error FROM v_event
           WHERE :kind = 'collector' AND type = 'collector.synced'
             AND json_extract(payload, '$.collector') = (SELECT ref FROM v_contribution
                                                           WHERE extension = :extension AND kind = :kind AND local = :contribution)
             AND json_extract(payload, '$.status') = 'error'
             AND json_extract(payload, '$.error') IS NOT NULL
           ORDER BY seq DESC LIMIT 5)) AS collector_errors,
       (SELECT json_group_array(reason) FROM (
          SELECT reason FROM v_effect_run
           WHERE :kind = 'effect' AND effect = :extension || '/' || :contribution
             AND state = 'failed' AND reason IS NOT NULL AND latest = 1
           ORDER BY id DESC LIMIT 5)) AS effect_errors,
       (SELECT last_error FROM v_contribution_health
         WHERE extension = :extension AND contribution = :contribution AND kind = :kind) AS last_error
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

def transform(x):
    event = x["event"]
    p = event["payload"]
    named = ref.contribution(p["contribution"])
    extension, contribution, kind = named["extension"], named["id"], named["kind"]
    reason = p["reason"]
    prefix = "Repair %s %s:" % (extension, contribution)

    state = scope("sql.read", {"sql": _STATE, "params": {"prefix": prefix}})[0]
    if not state["provider"]:
        return {"skip": "no work list is running to file a repair item on"}

    found = scope("sql.read", {
        "sql": _FAILURES,
        "params": {"extension": extension, "contribution": contribution, "kind": kind},
    })[0]
    errors = _list(found["collector_errors"]) + _list(found["effect_errors"])
    if not errors and found["last_error"]:
        errors = [found["last_error"]]

    if state["open_item"]:
        lines = ["Disabled again:", "", _quote(reason)]
        if len(errors) > 1:
            lines.extend(["", "Recent failures (newest first):", ""])
            lines.extend(["- " + md_text(e) for e in errors[:_ERRORS_SHOWN]])
        return {"commands": [{
            "name": "oxplow.work_item.comment",
            "input": {"ref": state["open_item"], "body": "\n".join(lines)},
        }]}

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
        "4. Say what you changed on this item. A person enables it again (`oxplow.contribution.enable`, Settings → Extensions); you can't.",
    ])
    return {"commands": [{
        "name": "oxplow.work_item.create",
        "input": {"title": _title(prefix, reason), "body": "\n".join(body)},
    }]}
