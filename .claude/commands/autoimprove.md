---
description: Review this session's transcript and propose concrete newde improvements
---

# /autoimprove — reflect on the session, propose newde upgrades

You are going to review **your own transcript** for the current batch and produce a focused critique of newde itself. The goal is to find changes to newde's code, prompts, tooling, or UX that would make future sessions faster, cheaper, more correct, and more collaborative.

Remember: newde is a shipped product that other users run too. Fixes belong in newde's code, prompts, hooks, or docs — not in your auto-memory.

## Step 1 — Read the transcript

- Use `ls ~/.claude/projects/<YOUR DIR>/*.jsonl -t | head -1` (adjust the project slug if the cwd differs) to find the most recent transcript file, or pull `sessionId` from `newde__get_batch_context` and read `~/.claude/projects/<project-slug>/<sessionId>.jsonl`.
- The transcript is JSONL — one record per line. Skim for `type:"user"`, `type:"assistant"`, and tool-call records. Don't try to quote the whole thing; sample the interesting stretches.
- Also call `newde__list_agent_turn` and `newde__list_batch_work` for this batch so your analysis is grounded in what actually shipped vs what was asked.

## Step 2 — Analyze across five lenses

Write the report against each lens. Cite specific turns / tool calls as evidence — "in turn N I did X, which …" — not vague impressions.

1. **Flow friction** — Where did the session stall, re-ask, or backtrack? Which steps felt like boilerplate vs real work?
2. **Effort & token efficiency** — Where did you (or hooks / prompts) pull more context than needed? Where could a targeted tool have replaced a broad Read/Grep/Agent call? Where did a mistaken route burn tokens before self-correcting?
3. **Correctness & verification** — Where did you assume instead of verify? Did any change ship without a reasonable test or runtime check? Were there claims ("all tests pass", "UI works") that weren't actually verified?
4. **Agent ↔ user collaboration** — Were requests clear enough? Where did you guess instead of ask? Where did the user have to re-course-correct? Were the work-item notes / batch summaries useful to them?
5. **newde tooling itself** — Missing MCP tools, confusing prompt instructions, noisy hooks, stop-hook behavior, work-item statuses, permissions friction, anything in the harness that got in the way. Be specific about *which file* to change.

## Step 3 — Produce a prioritized improvement list

For each proposed change:

- **Title** (imperative, ≤ 70 chars).
- **Evidence** — the transcript/session observation that motivated it.
- **Proposed change** — file path(s) and a one-sentence intervention.
- **Impact** — which of the five lenses it helps and roughly how much.
- **Risk / cost** — anything the change breaks, or effort to implement.

Sort the list by impact-per-cost, highest first.

## Step 4 — File the top items as work items

Use `newde__create_work_item` for each top-tier proposal you're confident about. Put the evidence in `description`, the intervention summary in the title, and an acceptance-criteria checklist (plain text, one per line) that a future agent could actually verify against. Do NOT create work items for vague ideas — if you can't write acceptance criteria, the idea isn't ready.

Link related items with `newde__link_work_items` (`relates_to` by default; `discovered_from` if the proposal surfaced while you were on another ticket).

Leave the work items in `waiting` status so the user can triage. Don't set them to `in_progress` or `human_check`.

## Step 5 — End-of-turn report

Respond to the user with:

- A short prose summary (5–10 sentences) highlighting what worked well and the top 3 proposed changes.
- A bulleted list of every work item you filed with its id.
- An honest note on anything you *didn't* analyze (e.g. segments you skipped, transcript sections you couldn't parse).

Keep final text under ~400 words — the work items carry the detail.
