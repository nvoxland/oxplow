# Working in this repo

`.context/` is the project's durable knowledge base — the authoritative
home for project decisions, system mechanics, gotchas, and conventions.

Read the relevant section of the linked subsystem doc before editing and
update it in the same commit when behavior changes. The
[contributor guide](.context/working-in-this-repo.md) has the full conventions.

## Keep verification focused

- Reuse completed checks across turns and restarts while their inputs are
  current. Check the commit and changed files before rebuilding.
- Finish shared source edits before building. Batch focused checks, keep
  full logs on disk, and read the final summary or failing excerpts.
- Run the checks the change warrants. Retry a known machine or harness
  failure when a relevant condition has changed.

## Always-on rules

- **`.context/architecture.md`** — the high-level stance. Don't violate
  the workspace isolation rule without an explicit decision to revisit it.
- **`.context/usability.md`** — UI rules (Enter submits, Escape cancels,
  drop-target highlighting, right-click for destructive actions, etc.).
  Read before adding *any* UI.
- **Tasks from the command line.** oxtask, oxplow's own task store,
  ships in `oxplow-bundled` and is a store of the shared task model. With
  oxtask enabled in the project, or oxplow not running, file and update
  tasks with `cargo run -q -p oxplow-dev -- task …`
  (`.context/working-in-this-repo.md` "oxplow-dev"); it works over oxtask's
  own file when the store is available. This checkout explicitly enables
  the agent-managed task workflow.
- **Testing.** Use `bun run test:ext [<name>]` for extension fixtures,
  `bun run test:fast -p <crate> [filter]` for Rust, and
  `bun run test:fast:ts` for the frontend. Run report-emitting checks in
  the foreground. Use `bun run test:collect` when coverage is warranted.
  Run `cargo fmt --all` after Rust edits and verify Clippy before pushing.
  See the [test guide](.context/working-in-this-repo.md#tests) for prerequisites
  and report handling.
- **Run oxplow from source with `bun run run:dev`** (oxplow quit): it
  builds (`bun run build:dev`), deploys the extensions whose source
  changed (`oxplow-dev extension deploy`; boot deploys nothing from
  source) and starts `./target/debug/oxplow .`. Never `cargo build --bin
  oxplow`: building the shell copies the *staged* daemon next to it, so a
  shell built without restaging runs an old backend. `DEV.md` "Run from
  source".
- **Extension work and core work go live differently.**
  - **Extensions** (`extensions/oxplow-foundation`, `extensions/oxplow-bundled`,
    any `.oxplow/extensions/<name>`): write them the way oxplow's shipped
    `oxplow-extension` skill says — the same skill users' agents get. Its
    `references/development-cycle.md` is the loop: edit the source, check
    it, then `oxplow.extension.deploy { name }` replaces what runs in the
    running app. It deploys the main stream's copy, uncommitted changes
    included (deploy for the person to look, commit when they're happy);
    a worktree stream's copy is a draft that never deploys. Never restart
    oxplow for an extension change, and never ask the person to.
    **An extension's behavior is tested by a fixture in its extension,
    written first** — a page, a command, an event's effects, a job, a
    collector, an advisory, a query over its models, a custom component's
    own code (`ui:` steps): `fixtures/<example>.yaml`, `given` → `when` →
    `expect`, `steps` for a scenario (the skill's `references/testing.md`
    has the kinds). It runs on a throwaway oxplow with only the data the
    fixture makes and only the extensions this one declares (`requires:`),
    never the project's database. Rust tests are for core, never for an
    extension's behavior (the page tests are fixtures now). How the app
    draws a page (select, expand, forms, slots, embeds, a component's frame) is the generic
    `tests-e2e/specs/pages/*` suite's: add a per-page or per-component e2e
    spec only for what only the app's shell does.
  - **Core** (any Rust crate, `apps/desktop/src`): it runs only after the
    person quits oxplow and runs `bun run run:dev` (build, deploy, start):
    say so when you finish. That also deploys foundation when its source
    changed with core (a removed built-in, a renamed operation).
  - **Recovering** when oxplow boots without foundation (no agents, no
    Alerts, no Settings; the shell shows "oxplow-foundation isn't
    running") or an extension is held: `cargo run -q -p oxplow-dev --
    extension status` (read-only, safe while oxplow runs) says what runs
    and why something is held; fix the source; quit oxplow; `… extension
    deploy [<name>]`; start oxplow. `.context/working-in-this-repo.md`
    "oxplow-dev".
- **Plan mode** for multi-subsystem work (3+ areas touched) or ambiguous
  requirements; skip it for single-file changes, typos, renames, narrow
  refactors.

## Subsystem docs — when to read which

| If you're touching… | Read first |
|---|---|
| Language support — language packs (`languages:` entries of extensions), the `oxplow_code_metrics::Languages` set, per-language specs (analysis/merge/LSP/metrics) | `.context/language-support.md` |
| Tables, stores, tasks, sort_index, migrations | `.context/data-model.md` |
| The agent process, Stop hook, MCP tools, write guard, agent prompt config | `.context/agent-model.md` |
| How oxplow infers the agent's work (turns and requests, no gates, hints) and the capabilities direction (swappable snapshots) | `.context/work-tracking.md` |
| Adding a new persisted operation (store + IPC + UI), event bus, cross-store updates | `.context/ipc-and-stores.md` |
| Background colors, tier hierarchy, adding a new color variable | `.context/theming.md` |
| `.git` watching, blame, branch changes, commit execution | `.context/git-integration.md` |
| Filesystems — the `FileSystem` trait and registry, `oxplow:local` over `Trees`, the `filesystem` param of file URIs, reading/stat/listing a file by filesystem and `rev` | `.context/filesystems.md` |
| The VCS capability — `Vcs` trait, `GitProvider`, `WorktreeRouter`, `WorkspaceFiles`, branch reconciler, conformance | `.context/vcs.md` |
| Smart conflict auto-resolution (Tier-1 token diff3; Tier-2 AST scoped) | `.context/smart-merge.md` |
| `EditorPane`, Monaco models/decorations/context menu, blame overlay, diff editor, LSP bridge | `.context/editor-and-monaco.md` |
| Markdown anywhere — the one `Markdown` component (viewer and editor), `RichTextField`, Tiptap, MermaidBlock + InternalLink, mermaidRender, links | `.context/markdown.md` |
| `TerminalPane`, xterm.js setup, file-path link provider | `.context/terminal.md` |
| LSP (session manager, document mirror, Mason installer, lsp RPCs/events, server config, MCP lsp tools) | `.context/lsp.md` |
| Code quality scans (in-process metrics + duplication detector + findings store + Code quality panel) | `.context/code-quality.md` |
| Request-scoped collection (test-run + diff-coverage observations, report collectors and their bundled parsers, the `testing:` profile, `/oxplow:configure`) | `.context/collection.md` |
| Resources and their URIs — building, reading, storing or linking any `<kind>:<id>` string in Rust, SQL, Starlark or TS (builders, readers, `uri` columns, dead links); the grammar and type registry; resource / renderer / view | `.context/resources.md` "Using refs" |
| Settings — the shared registry, namespaced keys, local/project/global YAML, secure values and consumers | `.context/settings.md` |
| Commands — the command bus (spec, actors, validate → policy → confirm → run + audit + `command.executed` in one transaction, undo), adding a command | `.context/commands.md` |
| Knowledge — pages as rows of the shared knowledge model (several stores at once; oxwiki is oxplow's own extension), bundled's `v_oxplow_knowledge_*` datasets and points, the store write guard | `.context/knowledge.md` |
| External providers — the JSON-RPC/NDJSON protocol crate and meta-model, the fake provider, the host (consent, spawn, handshake), instances and health, Settings → Integrations, the conformance kit and `oxplow extension test` | `.context/providers.md` |
| Tasks — the `oxplow/task_record` model, bundled's `v_oxplow_task` datasets and task points, oxtask | `.context/tasks.md` |
| Shared models (contract, being built) — `models:` / `fills:`, `model.write`, fields, markdown bodies, how tasks and knowledge become observed systems | `.context/models.md` |
| The semantic layer — sources (entities + facts), dimensions, metrics, the `v_*` read contract, `query_sql`, user/extension sources (target design) | `.context/semantic-layer.md` |
| Extensions — the terms (extension, contribution, capability, provider, scope, access, effect) and the index of the extensions docs | `.context/extensions.md` |
| Extensions — `extension.yaml` (manifest v2), contribution descriptors, `configuration:`, private state, the design rules | `.context/extensions-manifest.md` |
| Extensions — how pages run today: page files, ids, live set, params, checks, IPC/MCP surfaces, panels, desktop UI | `.context/extensions-page-host.md` |
| Extensions — the page YAML (metadata, params, loaders, view nodes, actions, checks), change links, page layout | `.context/extensions-pages.md` |
| Extensions — slots, replacements (`ui.mounts`), commands in core menus, decorators | `.context/extensions-slots.md` |
| Extensions — custom components (frame, protocol, testing one) | `.context/extensions-components.md` |
| Extensions — sources and deployments, builds, `extensionSources`, sharing, disabling, reviewing by impact | `.context/extensions-deploy.md` |
| Extensions — health, disable and repair; candidate publication, vocabulary generations, runtime adapters, ingress | `.context/extensions-activation.md` |
| Extensions — permissions and approval, the run context, services, grants, secret references | `.context/extensions-permissions.md` |
| Extensions — the event envelope, event types, resources, ordering, effects | `.context/extensions-events.md` |
| Extensions — collectors (sources), contributing metrics, jobs, AI profiles | `.context/extensions-jobs.md` |
| Extensions — extension commands, capability implementations, skills, advisories, the agent MCP surface | `.context/extensions-commands.md` |
| Extensions — the SDK (`oxplow extension new|check|test`), fixtures, answerability | `.context/extensions-sdk.md` |
| Extensions — `oxplow-foundation`, `oxplow-bundled`, the review packet, `examples/extensions/` | `.context/extensions-bundled.md` |
| AI providers & roles — API model access, role→model mapping, keychain, recorded computations (`AiCompute`, `ai_result`), the `ai_*` collector builtins | `.context/ai-providers.md` |
| Fact substrate (measure/dimension/metric_spec/capture/fact, cube, fact collectors, MCP/IPC reads, Metrics page) | `.context/metrics.md` |
| Profiling (the `cube_equivalence` harness, samply traps), what's already optimized, what measurement ruled out | `.context/performance.md` |
| Dashboards — pages in `category: Dashboards` (grid of embeds), the Dashboards index, `new_dashboard`, Pin to Dashboard | `.context/dashboards.md` |
| Tab store, page chrome, rail HUD, renderers (which page draws a URI) + tab id format | `.context/pages-and-tabs.md` |
| External URL tabs, sandboxed webview, allowlist, partition policy | `.context/external-url-tabs.md` |
| Remote daemon mode (oxplow-rpc dispatch, oxplow-daemon, transport switch, connect flow, reconnect banner) | `.context/remote-daemon.md` |
| Blog posts, user docs, release notes, README copy — anything reader-facing | `.context/writing-tone.md` |
| Repo layout, test/lint policy, full task-filing discipline | `.context/working-in-this-repo.md` |
