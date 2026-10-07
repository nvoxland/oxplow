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
layout, test/lint policy, and how work is tracked. The
rules below are the always-on essentials; that doc has the detail and
rationale. Read it (or the linked subsystem doc) when you need the *why*
or exact mechanics.

## Always-on rules

- **`.context/architecture.md`** — the high-level stance. Don't violate
  the workspace isolation rule without an explicit decision to revisit it.
- **`.context/usability.md`** — UI rules (Enter submits, Escape cancels,
  drop-target highlighting, right-click for destructive actions, etc.).
  Read before adding *any* UI.
- **Tracking is oxplow's.** oxplow opens, links and closes efforts from
  what it observes (`.context/work-tracking.md`); there's nothing to
  file before editing. File a task when it helps the person follow the
  work: a multi-step plan, or a follow-up spotted along the way (as
  `ready`).
- **Asking the user a question.** When your reply needs the user's
  answer, end it with the question itself: a final message that ends in
  a question shows the thread as waiting on them.
- **Which tests and lint to run, and when, is your call** — run what
  the change warrants. Nothing requires a full run before a commit. The
  commands, all in the foreground (a backgrounded run's reports are
  never ingested), never bare `cargo test` / `bun test` (they emit no
  reports):
  - `bun run test:fast` (filtered: `-p oxplow-app some_test`) and
    `bun run test:fast:ts` — quick, write the JUnit reports the
    effort's Tests panel shows.
  - `bun run test:collect` — the full suite with coverage, the only run
    that emits lcov for the coverage panel. See `.context/collection.md`.
  - `bun run lint:collect` — clippy with `-D warnings` (JSON to
    `target/clippy.json`, which feeds the `oxplow.analysis.*` metrics).
    CI treats warnings as errors, so Rust changes need it clean before
    they're pushed. Plain `cargo clippy --workspace --all-targets -- -D
    warnings` gives readable diagnostics. Run `cargo fmt --all` after
    editing `.rs`. Don't `#[allow(...)]` a real lint.
- **Plan mode** for multi-subsystem work (3+ areas touched) or ambiguous
  requirements; skip it for single-file changes, typos, renames, narrow
  refactors.

## Subsystem docs — when to read which

| If you're touching… | Read first |
|---|---|
| Language support — the `Language` enum, `LanguagePlugin` registry, per-language specs (analysis/merge/LSP/metrics) | `.context/language-plugins.md` |
| Tables, stores, work queue, sort_index, migrations | `.context/data-model.md` |
| The agent process, Stop hook, MCP tools, write guard, agent prompt config | `.context/agent-model.md` |
| How oxplow infers the agent's work (efforts, policies, no gates, hints) and the swappable-pieces direction (capabilities, work list, snapshots) | `.context/work-tracking.md` |
| Adding a new persisted operation (store + IPC + UI), event bus, cross-store updates | `.context/ipc-and-stores.md` |
| Background colors, tier hierarchy, adding a new color variable | `.context/theming.md` |
| `.git` watching, blame, branch changes, commit execution | `.context/git-integration.md` |
| The VCS capability — `Vcs` trait, `GitProvider`, `WorktreeRouter`, `WorkspaceFiles`, branch reconciler, conformance | `.context/vcs.md` |
| Smart conflict auto-resolution (Tier-1 token diff3; Tier-2 AST scoped) | `.context/smart-merge.md` |
| `EditorPane`, Monaco models/decorations/context menu, blame overlay, diff editor, LSP bridge | `.context/editor-and-monaco.md` |
| `RichTextField`, Tiptap surface, MermaidBlock + InternalLink extensions, mermaidRender helper | `.context/rich-text-editor.md` |
| `TerminalPane`, xterm.js setup, file-path link provider | `.context/terminal.md` |
| LSP (session manager, document mirror, Mason installer, lsp RPCs/events, server config, MCP lsp tools) | `.context/lsp.md` |
| Code quality scans (in-process metrics + duplication detector + findings store + Code quality panel) | `.context/code-quality.md` |
| Effort-scoped collection (test-run + diff-coverage observations, report collectors and their bundled parsers, the `testing:` profile, `/oxplow:configure`) | `.context/collection.md` |
| Refs — the canonical `<kind>:<id>[@rev][#frag]` grammar, the kind registry, what replaces tab ids / `page_ref` kinds / `[[…]]` shapes | `.context/refs.md` |
| Commands — the command bus (spec, actors, validate → policy → confirm → run + audit + `command.executed` in one transaction, undo), adding a command | `.context/commands.md` |
| Knowledge (the wiki) — `knowledge.write_page` and its sibling commands, pins, hand-edit convergence, the wiki write guard | `.context/knowledge.md` |
| External providers — the JSON-RPC/NDJSON protocol crate and meta-model, the fake provider, the host (consent, spawn, handshake, `ExternalWorkItems`), instances and health, Settings → Integrations, the conformance kit and `oxplow plugin test` | `.context/providers.md` |
| Work items — `v_work_item`, the `WorkItemsProvider` capability and registry, the oxplow provider, the conformance suite | `.context/work-items.md` |
| The semantic layer — sources (entities + facts), dimensions, metrics, the `v_*` read contract, `query_sql`, user/extension sources (target design, epic tsk275) | `.context/semantic-layer.md` |
| Extensions — `extension.yaml`, lenses (user/agent-built UI), slots, actions/alerts, the bundled `oxplow-bundled` extension, what moves out of core (target design) | `.context/extensions.md` |
| AI providers & roles — API model access, role→model mapping, keychain, recorded computations (`AiCompute`, `ai_result`), the `ai_*` collector builtins, inferred decisions | `.context/ai-providers.md` |
| Fact substrate (measure/dimension/metric_spec/capture/fact, cube, fact collectors, MCP/IPC reads, Metrics page) | `.context/metrics.md` |
| Profiling (the `cube_equivalence` harness, samply traps), what's already optimized, what measurement ruled out | `.context/performance.md` |
| User-created dashboards (dashboard/dashboard_item stores, the `dashboard.*` commands, custom-dashboard page + tile grid) | `.context/dashboards.md` |
| Tab store, page chrome, rail HUD, page kinds + tab id format | `.context/pages-and-tabs.md` |
| External URL tabs, sandboxed webview, allowlist, partition policy | `.context/external-url-tabs.md` |
| Remote daemon mode (oxplow-rpc dispatch, oxplow-daemon, transport switch, connect flow, reconnect banner) | `.context/remote-daemon.md` |
| Blog posts, user docs, release notes, README copy — anything reader-facing | `.context/writing-tone.md` |
| Repo layout, test/lint policy, full task-filing discipline | `.context/working-in-this-repo.md` |
