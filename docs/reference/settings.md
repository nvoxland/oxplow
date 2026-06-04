# Settings

Oxplow stores per-project state in `.oxplow/state.sqlite` and a
small amount of per-user state in your OS user-config directory.
Most settings are toggles in the UI; almost nothing requires
editing a file.

## Where state lives

- **Per-project** — everything that matters lives under
  `.oxplow/` inside the project root:

    ```
    .oxplow/
      state.sqlite        # tasks, threads, snapshots, settings
      wiki/               # wiki pages (markdown)
      snapshots/          # per-effort file snapshots (Local History)
      runtime/            # Claude Code plugin oxplow installs per project
      lsp/                # cached LSP server binaries (Mason packages)
    ```

    Add `.oxplow/` to `.gitignore`; oxplow does not commit
    project state.

- **Per-user** — the Tauri app's data dir under your OS config
  location: `~/Library/Application Support/oxplow/` (macOS),
  `%APPDATA%/oxplow/` (Windows), or `~/.config/oxplow/` (Linux).
  Window position, recent projects, theme preference. Wipe
  freely — it rebuilds.

There is no global config file you need to edit to get started.
Sensible defaults; opinionated product.

## Settings worth knowing

### Writer thread

Per-stream. Exactly one thread is the writer. Other threads are
read-only. Switch the writer from the thread tab kebab.
Switching kicks any in-flight write attempt on the old writer
back through the hook (which fails it cleanly).

### Stream and thread custom prompts

Each stream and thread has its own settings page (open from the
tab kebab → Settings) with a custom prompt field appended to the
agent's system prompt at launch. Use it for stream-specific
framing ("you're on the migration branch, priority is not
breaking schema") or thread-specific framing ("research only —
never edit").

### Agent kind

Per-thread. Default is Claude Code. `copilot` is also supported
but skips the oxplow plugin plumbing — no filing-enforcement, no
Stop directives, no MCP tools.

### tmux mode

Per-thread. Default on. The agent process runs inside a tmux
session so it survives oxplow restarts. Toggle from the agent
tab kebab. The tmux session name is requested when you switch a
thread into tmux mode.

### Snapshot retention

Snapshots from closed tasks are pruned on a 24-hour schedule
(orphaned blobs in `.oxplow/snapshots/` are GC'd at the same
time). Tune the retention window from the project's settings
page if the default doesn't fit (most users never touch this).

### Generated paths

`oxplow.yaml` carries a `generated` list of paths the project
should treat as build output / generated content. Anything on
the list is invisible to fs-watch, snapshot capture, the
startup sweep, code-quality scans, and every snapshot list view
in the UI. Entries can be:

- A **single-segment name** (no `/`) — matches anywhere in the
  path. `target` filters every `target/` directory in the tree.
- A **repo-relative path** — matches the exact path or
  everything under it. `apps/desktop/dist` filters that one
  directory, not unrelated `dist/` elsewhere. `docs/generated/output.txt`
  filters just that file.

```yaml
# oxplow.yaml
generated:
  - target
  - node_modules
  - .idea
  - apps/desktop/dist
```

Two paths are *always* ignored regardless of config: `.git/`,
and everything under `.oxplow/` except `.oxplow/wiki/`. Build
dirs that used to be hardcoded (`node_modules`, `target`,
`.next`, …) are now the user's call — projects without a
`generated` list will see them captured.

You can edit `oxplow.yaml` directly, or right-click any
directory in the file tree → **Mark as generated** to append
its name to the list. Read-side filtering applies on every
read, so paths added after they were already captured drop
out of the UI immediately — no rescan, no purge.

### Collection (tests & coverage)

The `collection` block tells oxplow how to attach test results and
diff coverage to each effort. Point it at the report files your test
run already emits; oxplow parses each one fresher than the effort
start, so a polyglot repo lights up per stack. Run
`/oxplow:configure` to wire this up automatically.

```yaml
# oxplow.yaml
collection:
  testCommand: bun run test:collect    # informational; surfaced to the agent
  reports:
    - { path: target/coverage/lcov.info, format: lcov }
    - { path: target/nextest/default/junit.xml, format: junit }
    - { path: apps/desktop/test-report.xml, format: junit }
  testRunPatterns: [bun test]          # extra substrings that count as a test run
```

Built-in formats are `lcov`, `cobertura`, `jacoco-xml` (coverage) and
`junit` (the per-test tree). Coverage numbers come only from oxplow
parsing the report, never from the agent, so they stay trustworthy.

#### Adding a format with a plugin

For a format oxplow doesn't parse out of the box, add a
`collection.plugins` entry — no recompile, no change to oxplow. A
plugin is a small script that maps a report into oxplow's
coverage/test shape:

```yaml
collection:
  reports:
    - { path: target/clover.xml, format: clover }
  plugins:
    - name: clover
      kind: coverage        # coverage | test
      formats: [clover]     # format name(s) this plugin claims
      runtime: jaq          # jaq (jq) | starlark | exec
      input: xml            # host pre-parse: text | json | xml | lcov | lines
      entry: |              # jq program: parsed input (.) -> output shape
        { files: ... }
```

The host pre-parses the report for you (per `input`), so the script
only reshapes JSON. `jaq` (jq) is the simplest for XML/JSON; `starlark`
covers logic jq can't express; `exec` runs an external program (raw
report on stdin, JSON on stdout) as a last resort. jaq and starlark
run in-process and sandboxed, so their output is trusted as measured;
`exec` can do I/O, so its output is flagged lower-trust in the UI.

The full authoring reference — host helpers, the exact output schemas,
a worked example — lives in `.context/collection.md` in the oxplow
source.

### LSP servers

Auto-managed. Oxplow's bundled LSP installer fetches Mason
packages on first use, caches them under `.oxplow/lsp/`, and the
proxy hands the right binary to whichever stream asked. There is
no `lsp.json` to maintain — supported languages are
auto-detected from project content (file extensions, root
markers).

If a server you need isn't yet supported, file a task; the
installer's manifest lives in
`crates/oxplow-lsp/src/installer/`.

### Theme

Dark only. Oxplow is dark-only on purpose — Monaco is pinned to
`vs-dark` and the UI tokens are calibrated for it.

### Telemetry

Off. Always. There is no telemetry to configure.

## Settings the agent can change

None. The MCP surface deliberately does not expose product
settings — the agent operates on intent, files, tasks, and
wiki pages. Configuration is the human's job.

## Resetting

Wipe `.oxplow/` to reset a project. Wipe the user-config
directory to reset everything. Both are safe; oxplow rebuilds
what it needs on next launch (your task history goes with
the project state, though, so don't do it casually).
