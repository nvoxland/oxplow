# oxplow-bundled/effort.infer_decisions: ask a model which decisions the
# agent made during one effort without recording them. The one place the
# prompt lives (the `infer-decisions` effect and a person's Infer Again
# both come here). It reads what the agent did — the conversation in the
# effort's window, its tool calls, the decisions it did record — into one
# digest, newest kept when it's long, and asks the `decisions` job. The
# intent key names the effort (`decisions:<effort ref>`), so asking twice
# returns the same job; `again` asks a new one (`…@<n>`), which a changed
# binding or new activity answers afresh. The `store-decisions` effect
# stores what comes back.

# Longest digest, longest kept prompt or answer, longest tool detail, and
# the share of the digest tool calls may take — in characters.
MAX_DIGEST = 40000
MAX_TURN = 2000
MAX_DETAIL = 200
MAX_TOOLS = MAX_DIGEST // 4
ROWS = 1000

SYSTEM = """You review a coding agent's work session. Find the decisions it made without asking: forks where it picked one approach over another (where to put code, which library, what to change or leave, how to handle a case). Only real forks with a plausible alternative; skip routine steps. Don't repeat decisions it already recorded. For each: `question` is what had to be decided, `choice` what it chose, `alternatives` the options not taken, `why` its reasoning if visible, and `confidence` (low, medium or high) how sure you are it was a real decision. At most 8; an empty list is fine.

Reply with JSON only, no prose and no code fence:
{"decisions": [{"question": "...", "choice": "...", "alternatives": ["..."], "why": "...", "confidence": "low"}]}"""

def clip(text, most):
    text = text or ""
    return text if len(text) <= most else text[:most] + "…"

def newest_that_fit(items, budget):
    """The most recent of `items` (oldest first) whose total fits `budget`, oldest first."""
    kept = []
    used = 0
    for item in reversed(items):
        size = len(item) + 1
        if used + size > budget:
            break
        used += size
        kept.append(item)
    return list(reversed(kept))

def read(sql, effort_id):
    return scope("sql.read", {"sql": sql, "params": {"id": effort_id}})

def transform(x):
    effort = x["input"]["effort"]
    if not ref.is_ref(effort) or ref.kind(effort) != "effort":
        return {"refuse": "`effort` is an effort ref, `effort:<id>`"}
    found = scope("sql.read", {
        "sql": "SELECT e.id, e.work_item, coalesce(w.title, '') AS title FROM v_effort e "
        + "LEFT JOIN v_work_item w ON w.ref = e.work_item WHERE e.ref = :ref",
        "params": {"ref": effort},
    })
    if not found:
        return {"refuse": "no effort " + effort}
    effort_id = found[0]["id"]
    # Newest first under the row cap, so a long effort keeps its latest
    # activity; reversed to oldest first below. The turns are the effort's
    # thread's that overlap its window (a turn has no effort of its own).
    turns = read(
        "SELECT a.prompt, coalesce(a.answer, '') AS answer FROM v_agent_turn a, v_effort e "
        + "WHERE e.id = :id AND a.thread_id = e.thread_id "
        + "AND a.started_at <= coalesce(e.ended_at, '9999') "
        + "AND coalesce(a.ended_at, '9999') >= e.started_at "
        + "ORDER BY a.started_at DESC, a.id DESC LIMIT " + str(ROWS), effort_id)
    tools = read(
        "SELECT tool, path, detail, ok FROM v_tool_call WHERE effort_id = :id "
        + "ORDER BY id DESC LIMIT " + str(ROWS), effort_id)
    if not turns and not tools:
        return {"commands": [], "result": {"skipped": "nothing to read for " + effort}}
    recorded = read(
        "SELECT question FROM v_decision WHERE effort_id = :id AND provenance = 'recorded' ORDER BY id",
        effort_id)

    head = "Work item: " + found[0]["title"] + "\n"
    if recorded:
        head += "\nDecisions it already recorded (don't repeat these):\n"
        for r in recorded:
            head += "- " + r["question"] + "\n"
    tool_lines = []
    for t in reversed(tools):
        line = t["tool"]
        if t["path"]:
            line += " " + t["path"]
        if t["detail"]:
            line += ": " + clip(t["detail"], MAX_DETAIL)
        if t["ok"] == 0:
            line += " (failed)"
        tool_lines.append(line)
    tool_lines = newest_that_fit(tool_lines, MAX_TOOLS)
    blocks = [
        "Human: " + clip(t["prompt"], MAX_TURN) + "\nAgent: " + clip(t["answer"], MAX_TURN) + "\n"
        for t in reversed(turns)
    ]
    room = MAX_DIGEST - len(head)
    for line in tool_lines:
        room -= len(line) + 1
    blocks = newest_that_fit(blocks, room)
    digest = (head + "\nConversation (oldest first):\n" + "\n".join(blocks)
              + "\nTool calls (oldest first):\n" + "\n".join(tool_lines))

    key = "decisions:" + effort
    if x["input"].get("again"):
        asked = scope("sql.read", {
                "sql": "SELECT count(*) AS n FROM v_job_run WHERE intent_key = :key OR intent_key LIKE :key || '@%'",
                "params": {"key": key},
            })
        key += "@" + str(asked[0]["n"] + 1)
    return {"commands": [{
        "name": "oxplow.decisions.ask",
        "input": {
            "job": "decisions",
            "intentKey": key,
            "request": {
                "schemaVersion": 1,
                "role": "decisions",
                "messages": [
                    {"role": "system", "parts": [{"type": "text", "text": SYSTEM}]},
                    {"role": "user", "parts": [{"type": "text", "text": digest}]},
                ],
            },
        },
    }]}
