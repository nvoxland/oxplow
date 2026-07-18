# Settings

Oxplow stores per-project state in `.oxplow/local.sqlite` and a
small amount of per-user state in your OS user-config directory.
Most settings are toggles in the UI; almost nothing requires
editing a file.

## Where state lives

- **Per-project** — everything that matters lives under
  `.oxplow/` inside the project root:

    ```
    .oxplow/
      project.yaml        # shared project config — commit this
      .gitignore          # written by oxplow on first run
      local.sqlite        # tasks, threads, snapshots, settings
      wiki/               # wiki pages (markdown)
      snapshots/          # per-effort file snapshots (Local History)
      runtime/            # generated agent runtime files
      lsp/                # installed LSP server binaries (appears after
                          #   an explicit install)
      lsp-cache/          # download cache for the above
    ```

    **Don't add `.oxplow/` to `.gitignore`.** On first run oxplow
    writes `.oxplow/.gitignore` containing:

    ```
    *
    !.gitignore
    !project.yaml
    ```

    so `project.yaml` is committable and shared with the team while
    the database, snapshots, wiki, and runtime files stay local to
    your machine. An existing `.gitignore` is left untouched.
    Ignoring the whole directory defeats that split.

    Note the wiki is *not* excepted — `.oxplow/wiki/` is local-only
    by default.

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

### Agents

Per-project `agents` controls which agent implementations are offered
when a thread is created. Configure it from the project Settings page
or in `.oxplow/project.yaml`:

```yaml
agents:
  - claude
  - codex
```

Supported values:

| Value | CLI Oxplow launches |
| --- | --- |
| `claude` | Claude Code (`claude`) |
| `codex` | Codex (`codex`) |

The list order matters: the first entry is the default for new threads.
The Settings page's **Up** and **Down** buttons change that order. If
only one agent is enabled, new threads use it automatically; if both
are enabled, the new-thread row shows a selector.

The setting must contain at least one value and cannot contain
duplicates. Omitting it defaults to `[claude]`. Oxplow rejects unknown
agent names, an empty list, and a list with duplicate entries.

Each thread stores its assigned agent at creation time. The assignment
cannot be changed from Thread Settings, and reordering or disabling
agents does not rewrite existing threads. This allows Claude and Codex
threads to run concurrently while preserving their independent
sessions and history.

Older configurations may contain the singular key:

```yaml
agent: codex
```

Oxplow still reads it as a one-entry enabled-agent list for migration.
Use `agents` for new configuration; specifying both `agent` and
`agents` is an error.

The selected CLI must be installed, authenticated, and available on
`PATH`. See [Agents](../guide/agents.md) for thread selection, runtime
integration, and concurrent operation.

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

**Your `.gitignore` is the baseline.** Oxplow reads the root
`.gitignore`, every nested `.gitignore`, and `.git/info/exclude`,
so `node_modules`, `target`, `dist` and friends are already
invisible without any oxplow config. Most projects need nothing
here.

The `generated` block is for the two cases `.gitignore` can't
express: build output that *is* committed, and ignored paths you
want oxplow to watch anyway.

```yaml
# .oxplow/project.yaml
generated:
  exclude:
    - target             # single segment — matches at any depth
    - apps/desktop/dist  # repo-relative — only this path
  include:
    - vendor/generated   # force back in, despite .gitignore
```

Both keys are optional, but `generated` is a **map**, not a list —
a bare sequence (`generated: [target, …]`) is rejected at load.

Entries in either list take one of two forms:

- A **single-segment name** (no `/`) — matches anywhere in the
  path. `target` filters every `target/` directory in the tree.
- A **repo-relative path** — matches the exact path or
  everything under it. `apps/desktop/dist` filters that one
  directory, not unrelated `dist/` elsewhere. `docs/generated/output.txt`
  filters just that file.

Precedence, highest first:

1. `.git/`, and everything under `.oxplow/` except `.oxplow/wiki/`,
   are *always* ignored — not overridable.
2. An `include` match is **kept**, overriding both `exclude` and
   `.gitignore`.
3. An `exclude` match is ignored.
4. Otherwise `.gitignore` decides.

Anything ignored is invisible to fs-watch, snapshot capture, the
startup sweep, code-quality scans, and every snapshot list view
in the UI.

You can edit `.oxplow/project.yaml` directly, or right-click any
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
# .oxplow/project.yaml
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
    - name: acme.clover   # namespaced; oxplow. is reserved
      kind: coverage        # coverage | test
      formats: [clover]     # format name(s) this plugin claims
      runtime: jaq          # jaq (jq) | starlark | exec
      input: xml            # host pre-parse: text | json | xml | lcov | lines
      entryFile: oxplow/plugins/clover.jq   # the script file (jq program here)
```

The script lives in its own file (`entryFile`, a project-relative path),
not inline in the yaml. The host reads it and pre-parses the report for
you (per `input`), so the script only reshapes JSON. `jaq` (jq) is the
simplest for XML/JSON; `starlark` covers logic jq can't express; `exec`
runs an external program — `entryFile` is the executable — (raw report on
stdin, JSON on stdout) as a last resort. jaq and starlark run in-process
and sandboxed, so their output is trusted as measured; `exec` can do
I/O, so its output is flagged lower-trust in the UI.

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
