// Reference material shipped with the Claude Code plugin (in the
// plugin-dir the agent has local `Read` access to) instead of inlined
// in the system prompt. Trimming the prompt by ~400 chars improves
// cross-session cache reuse without hiding information the agent
// genuinely needs — the agent can fetch this file on demand when it
// actually uses link types or needs to pick a work-item kind.

export const AGENT_GUIDE_FILENAME = "AGENT_GUIDE.md";

export function buildAgentGuide(): string {
  return `# oxplow agent guide

Reference catalog the agent can read on demand — you shouldn't need to
quote this back, just use the right values when calling oxplow MCP tools.

## Work-item kinds

- **epic** — multi-step feature that decomposes into tasks / subtasks.
- **task** — a concrete unit of work that ships end-to-end in one or a
  few turns. The default for most user requests.
- **subtask** — a small step inside a task; only create these when the
  task itself is large enough that tracking sub-steps in the work-item
  log would help the user review progress.
- **bug** — a defect to fix, as distinct from new work. Titles should
  read as an observation ("X doesn't do Y") not a prescription.
- **note** — an observation that doesn't need execution (retrospective
  findings, open questions, decisions to revisit). Appears in the
  history panel but never enters the ready queue.

## Link types (\`oxplow__link_work_items\`)

- **blocks** — from-item must finish before to-item can start. Use
  this for hard ordering (migration before feature that uses it).
- **discovered_from** — from-item was uncovered while working on
  to-item. Preferred escape hatch for scope creep: file the new
  thing separately, link it back, keep the original scoped.
- **relates_to** — general association with no enforced ordering.
  The catch-all when none of the stronger semantics fit.
- **duplicates** — from-item is the same work as to-item. Close or
  supersede the duplicate after linking.
- **supersedes** — from-item replaces to-item. The older target is
  stale and should not be worked on.
- **replies_to** — from-item is a threaded note/response to
  to-item. Useful for layered conversations about a proposal.
`;
}
