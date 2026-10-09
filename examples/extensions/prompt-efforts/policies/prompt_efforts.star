# One effort per prompt. Gets {"event": {id, type, v, seq, source, subject,
# payload, anchors}}, one of core's events an effort policy hears, and returns
# the commands to run as the policy, or {"skip": "why"}. It reads oxplow only
# through `scope("sql.read", ...)`.

def _thread_id(thread):
    # The anchors name a thread as `thr<n>`.
    return int(thread[3:])

def _open_effort(thread):
    rows = scope("sql.read", {
        "sql": "SELECT id, work_item, started_at FROM v_effort WHERE thread_id = :t AND ended_at IS NULL",
        "params": {"t": _thread_id(thread)},
    })
    return rows[0] if rows else None

def _effort(row):
    return "effort:eff" + str(row["id"])

def _checkpoint(e):
    p = e["payload"]
    a = e["anchors"]
    if not p.get("changed") or p.get("writing_tools", 0) == 0:
        return {"skip": "a turn that changed nothing gets no effort of its own"}
    thread = a.get("thread_id")
    turn = a.get("turn_id")
    if thread == None or turn == None:
        return {"skip": "a checkpoint with no thread or turn"}
    turns = scope("sql.read", {
        "sql": "SELECT started_at FROM v_agent_turn WHERE id = :t",
        "params": {"t": turn},
    })
    if not turns:
        return {"skip": "its turn isn't recorded"}
    since = turns[0]["started_at"]
    current = _open_effort(thread)
    if current != None and current["started_at"] >= since:
        return {"skip": "an effort opened during this turn already covers it"}
    commands = []
    opened = {"thread": "thread:" + thread, "adopt_since": since}
    if current != None:
        # The previous prompt's effort ends where this turn began; this one
        # carries on its work item.
        commands.append({"name": "oxplow.effort.close",
                         "input": {"effort": _effort(current), "as_of": since, "reason": "switch"}})
        if current["work_item"] != None:
            opened["work_item"] = current["work_item"]
    commands.append({"name": "oxplow.effort.open", "input": opened})
    return {"commands": commands}

def _item_moved(e):
    item = e["payload"]["work_item"]
    to = e["payload"]["to"]
    thread = e["anchors"].get("thread_id")
    if to == "in_progress" and thread != None:
        current = _open_effort(thread)
        if current == None:
            return {"commands": [{"name": "oxplow.effort.open",
                                  "input": {"thread": "thread:" + thread, "work_item": item}}]}
        if current["work_item"] == item:
            return {"skip": "the effort is already the item's"}
        return {"commands": [{"name": "oxplow.effort.link",
                              "input": {"effort": _effort(current), "work_item": item}}]}
    if to == "done" or to == "canceled":
        rows = scope("sql.read", {
            "sql": "SELECT id FROM v_effort WHERE work_item = :w AND ended_at IS NULL",
            "params": {"w": item},
        })
        if not rows:
            return {"skip": "the item has no open effort"}
        return {"commands": [{"name": "oxplow.effort.close",
                              "input": {"effort": _effort(r), "reason": "switch"}} for r in rows]}
    return {"skip": "a move that changes no effort"}

def transform(x):
    e = x["event"]
    if e["type"] == "thread.checkpoint":
        return _checkpoint(e)
    if e["type"] == "work_item.state_changed":
        return _item_moved(e)
    return {"skip": "an event this policy doesn't act on"}
