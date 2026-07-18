# Working in this repo

`.context/` is the project's durable knowledge base — the authoritative
home for project decisions, system mechanics, gotchas, and conventions.

1. **Read the relevant doc before touching its subsystem.** They're
   short on purpose — skipping them costs more than reading them.
2. **Update the relevant doc in the same commit as your change.** Docs
   that drift from code are worse than no docs.
3. **Capture new knowledge in `.context/`, not in agent memory.** A
   non-obvious decision, a recurring gotcha, an undocumented convention
   → write it into the matching doc.

Read the relevant `.context/<name>.md` with the `Read` tool before
touching its subsystem. The "concrete update triggers" checklist lives in
the full guide below.

**Full contributor guide: `.context/working-in-this-repo.md`** — repo
layout, test/lint policy, and the complete task-filing discipline. The
rules below are the always-on essentials; that doc has the detail and
rationale. Read it (or the linked subsystem doc) when you need the *why*
or exact mechanics.

## Always-on rules

- **`.context/architecture.md`** — the high-level stance. Don't violate
  the workspace isolation rule without an explicit decision to revisit it.
- **`.context/usability.md`** — UI rules (Enter submits, Escape cancels,
  drop-target highlighting, right-click for destructive actions, etc.).
  Read before adding *any* UI.
- **Filing enforcement (PreToolUse hook).** Every Edit / Write /
  MultiEdit / NotebookEdit on project files requires an `in_progress`
  task first — no trivial-edit carve-out. File one (or flip a `ready`
  row to in_progress), then re-issue the edit. Bash and edits made mid
  git-operation (`MERGE_HEAD` / `REBASE_HEAD` / `CHERRY_PICK_HEAD` /
  `REVERT_HEAD`) are exempt. Full rationale:
  `.context/working-in-this-repo.md` + `.context/agent-model.md`.
- **Asking the user a question.** When your reply ends with a real
  clarifying question, A/B/C choice, or any ask where the user owns the
  next move, call `mcp__oxplow__await_user({ threadId, question })` and
  end your turn — the Stop hook suppresses every directive until they
  reply. Not for rhetorical asides.
- **Closing a task → `bun run test:collect`** (not bare `cargo test` /
  `bun test`): it's the only run that emits the JUnit + lcov reports
  oxplow parses into the effort's coverage panel. See
  `.context/collection.md`.
- **After editing any `.rs`** run `cargo fmt --all` then `bun run
  lint:collect` before ending the turn — CI treats warnings as errors,
  and `lint:collect` (clippy with `-D warnings`, JSON to
  `target/clippy.json`) is what feeds the `oxplow.analysis.*` metrics.
  On failure re-run plain `cargo clippy --workspace --all-targets -- -D
  warnings` for readable diagnostics. Don't `#[allow(...)]` a real lint.
- **Task-filing discipline.** File a durable `in_progress` task before
  editing; one user-visible concern per row; multiple independent asks
  in one prompt → multiple tasks; every new (non-correction) ask gets
  its own item (a correction/redo reopens the existing one); a mid-turn
  user prompt is a new-ask boundary; file backlog ideas as `ready` when
  you spot them. Full rules + rationale:
  `.context/working-in-this-repo.md`.
- **Plan mode** for multi-subsystem work (3+ areas touched) or ambiguous
  requirements; skip it for single-file changes, typos, renames, narrow
  refactors.

## Subsystem docs — when to read which

| If you're touching… | Read first |
|---|---|
| Language support — the `Language` enum, `LanguagePlugin` registry, per-language specs (analysis/merge/LSP/metrics) | `.context/language-plugins.md` |
| Tables, stores, work queue, sort_index, migrations | `.context/data-model.md` |
| The agent process, Stop hook, MCP tools, write guard, agent prompt config | `.context/agent-model.md` |
| Adding a new persisted operation (store + IPC + UI), event bus, cross-store updates | `.context/ipc-and-stores.md` |
| Background colors, tier hierarchy, adding a new color variable | `.context/theming.md` |
| `.git` watching, blame, branch changes, commit execution | `.context/git-integration.md` |
| Smart conflict auto-resolution (Tier-1 token diff3; Tier-2 AST scoped) | `.context/smart-merge.md` |
| `EditorPane`, Monaco models/decorations/context menu, blame overlay, diff editor, LSP bridge | `.context/editor-and-monaco.md` |
| `RichTextField`, Tiptap surface, MermaidBlock + InternalLink extensions, mermaidRender helper | `.context/rich-text-editor.md` |
| `TerminalPane`, xterm.js setup, file-path link provider | `.context/terminal.md` |
| LSP (session manager, document mirror, Mason installer, lsp RPCs/events, server config, MCP lsp tools) | `.context/lsp.md` |
| Code quality scans (in-process metrics + duplication detector + findings store + Code quality panel) | `.context/code-quality.md` |
| Effort-scoped collection (test-run + diff-coverage observations, the coverage parser, the `collection:` profile, `/oxplow:configure`) | `.context/collection.md` |
| Unified metric substrate (metric_definition/run/sample/finding, dual-write producers, MCP/IPC reads, Metrics page) | `.context/metrics.md` |
| User-created dashboards (dashboard/dashboard_item stores, DashboardsChanged event, MCP authoring tools, custom-dashboard page + tile grid) | `.context/dashboards.md` |
| Tab store, page chrome, rail HUD (in-flight IA redesign) | `.context/pages-and-tabs.md` |
| External URL tabs, sandboxed webview, allowlist, partition policy | `.context/external-url-tabs.md` |
| Remote daemon mode (oxplow-rpc dispatch, oxplow-daemon, transport switch, connect flow, reconnect banner) | `.context/remote-daemon.md` |
| Blog posts, user docs, release notes, README copy — anything reader-facing | `.context/writing-tone.md` |
| Repo layout, test/lint policy, full task-filing discipline | `.context/working-in-this-repo.md` |
