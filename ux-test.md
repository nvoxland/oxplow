# UX Test Scenarios for Playwright-Driven Agent

## Context

newde has no E2E/Playwright tests. The user wants to hand a Playwright-capable
agent a list of realistic user tasks, let it exercise the UI, and have it
surface (and fix) UX problems it encounters along the way. This plan is that
list: a catalog of discrete, self-contained tasks phrased the way a real user
would describe them — each one exercises a specific surface, with emphasis on
recently-changed areas most likely to have rough edges.

Each task is written so the agent can:
1. Attempt the task in a fresh project.
2. Note anything confusing, broken, or missing (dead clicks, bad affordances,
   unclear error states, keyboard shortcuts that don't work, visual bugs).
3. Propose and implement fixes in the relevant source files.

## How to use this list

Feed tasks one at a time (or in small batches). For each:

1. **Setup**: launch newde against a throwaway project dir (see Verification).
2. **Execute the task** via Playwright, narrating each step.
3. **Record friction**: every place the agent had to guess, retry, or where
   behavior didn't match expectation. This is the real output — the task
   succeeding is less interesting than where it stumbled.
4. **Propose fixes** for friction points, grouped by subsystem, then implement
   the highest-value ones. Update the matching `.context/*.md` doc in the same
   commit per CLAUDE.md rules.

## Task catalog

Ordered roughly by risk surface — earlier tasks hit the newest/roughest code.

### Work-panel & queue (highest-churn surface)

1. **Create a work item from scratch.** Open the "new work item" flow, fill in
   title, kind, priority, and acceptance criteria. Save. Verify it appears in
   the correct status section. Then create a second one using "Save and
   Another" and confirm the form resets cleanly.

2. **Inline-edit a work item's title, status, and priority** without opening a
   detail pane. Tab between fields. Hit Esc mid-edit and confirm nothing was
   saved. Re-edit and hit Enter and confirm it persists. Look for focus/blur
   race conditions.

3. **Drag a work item between status sections** (To Do → Human Check → Done).
   Confirm the drop-target highlight is clear, the status actually changes, and
   the item lands in the right place. Try dragging onto an empty section.

4. **Reorder items within a status section using Shift+↑/↓** keyboard nav.
   Confirm selection moves as expected and `sort_index` updates visibly.

5. **Move a work item between batches** by dragging it onto another batch tab
   in the BatchRail. Then drag an item from the backlog into the active batch.
   Verify it appears in the queue immediately.

6. **Delete a work item** via right-click (per usability.md's destructive-
   actions convention). Confirm the confirmation UX is clear and undo/recovery
   options (if any) are obvious.

### Batch & stream lifecycle

7. **Create a new batch**, rename it, promote it to writer, then mark it
   complete. Watch for stale UI state when the active batch changes mid-flow.

8. **Switch streams** via the StreamRail. Confirm the editor, work queue, and
   agent pane all update to the new stream's state. Switch back and verify
   nothing was lost.

### Editor & file tree

9. **Open a file from the file tree**, edit it, save with Cmd/Ctrl+S, confirm
   the dirty indicator clears. Try closing an unsaved file and verify the
   prompt.

10. **Use Cmd/Ctrl+P (quick open)** to jump to a file by fuzzy match. Then use
    Cmd/Ctrl+F to search inside it.

11. **View git blame inline** by toggling blame on an edited file. Click a
    blame annotation and confirm "reveal in history" works.

### History, snapshots, diffs

12. **Open the History panel**, pick a turn, expand its changed-files tree,
    click a file, and view the diff. Then compare two turns against each other.
    Look for slow loads on large diffs or broken tree expand/collapse state.

13. **Approve a commit point** with an edited commit message. Confirm the
    commit actually lands in git log.

### Command palette & settings

14. **Open the command palette (Cmd/Ctrl+K)**, fuzzy-search for "settings",
    open settings, change the snapshot retention days, save, reopen settings,
    confirm the value persisted.

15. **Trigger a wait point** from inside the agent pane and confirm the next
    user prompt resumes the queue.

## Critical files the agent will touch when fixing issues

Keep this pinned — friction usually traces back to one of these:

- `src/ui/components/Plan/PlanPane.tsx`, `WorkGroupList.tsx`,
  `WorkItemDetail.tsx` — work panel, inline edit, drag-to-status
- `src/ui/components/BatchRail.tsx`, `StreamRail.tsx` — batch/stream tabs
- `src/ui/components/LeftPanel/FileTree.tsx` — file tree
- `src/ui/components/EditorPane.tsx` — Monaco host
- `src/ui/components/History/HistoryPanel.tsx`,
  `components/Diff/DiffPane.tsx` — history + diff
- `src/ui/components/CommandPalette/CommandPalette.tsx` — palette
- `src/ui/components/SettingsModal.tsx` — settings
- `.context/usability.md` — must consult before any UI change
- `.context/*.md` — must update alongside any subsystem change

## Verification

End-to-end smoke for the test harness itself:

1. **Launch against a scratch project**: `mkdtemp` a dir, `git init` it, point
   newde at it. This mirrors what `runtime.test.ts` does.
2. **Confirm Playwright can attach to the Electron main window** and query
   stable selectors (add `data-testid` attributes if missing — that's itself a
   likely outcome of task 1 or 2).
3. **Run 2–3 tasks end-to-end** before handing the full list to the agent, to
   validate the test scaffolding works.
4. **Each fix commit** must: (a) update the matching `.context/` doc, (b) add a
   colocated `bun:test` where logic changed, (c) not bundle unrelated
   cleanup.

## Backlog — discovered during dogfood sessions

Append new friction-driven scenarios here as they come up. Current
additions from the 2026-04-19 Playwright dogfood pass:

- **B1.** Re-probe whether Cmd+K palette actually receives the
  shortcut when Monaco has focus. Hypothesis: Monaco eats the
  keydown before the `window` handler in App.tsx sees it. If so,
  either register via `document` with `capture: true` or route the
  shortcut through the native menu.
- **B2.** Walk the "open a file from the tree and edit it in
  Monaco" path end-to-end via Playwright. Confirm dirty indicator,
  Cmd+S save, unsaved-close warning. The `📁src` folder did not
  appear in the initial file-tree dump — verify tree ordering /
  scrolling behavior when there are more siblings than fit in the
  viewport.
- **B3.** Verify drag-between-status-sections actually persists. The
  outline shows the "◐" row stack for status — need a testid per
  section and per work-item to drive a drag-drop test.
- **B4.** Commit approval flow via newde's UI — never got this far
  in the first dogfood pass. Set up a fresh branch and try to land
  a tiny edit through newde's own commit UX.
- **B5.** Investigate the `MaxListenersExceededWarning` at startup:
  11 listeners on `newde:event` — likely one store per subscription.
  Centralize the subscription or raise the limit at the preload.
- **B6.** Trace "rejected unauthorized mcp websocket" — legitimate
  defense or spurious noise?

## Out of scope (intentional)

- Building a full Playwright test *suite* that runs in CI. This plan is about
  exploratory UX testing + fixes, not locked-in regression coverage.
- Multi-agent / concurrency scenarios.
- Performance / load testing on large repos.
