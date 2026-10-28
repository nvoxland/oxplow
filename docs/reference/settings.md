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
read-only. Right-click a thread in the navigator → **Make writer**.
Switching kicks any in-flight write attempt on the old writer
back through the hook (which fails it cleanly).

### Stream and thread custom prompts

Each stream and thread has its own settings page (open from the
context menu → Settings) with a custom prompt field appended to the
agent's system prompt at launch. Use it for stream-specific
framing ("you're on the migration branch, priority is not
breaking schema") or thread-specific framing ("research only —
never edit").

### Agents

Per-project `agents` controls which agent implementations are offered
when an agent session is opened. Configure it from the project Settings page
or in `.oxplow/project.yaml`:

```yaml
agents:
  - claude
  - codex
```

Built-in terminal harness keys:

| Value | CLI Oxplow launches |
| --- | --- |
| `claude` | Claude Code (`claude`) |
| `codex` | Codex (`codex`) |
| `opencode` | OpenCode (`opencode`) |

The list order matters: the first available entry is the default for new
sessions. The Settings page's **Up** and **Down** buttons change that
order. Extension implementations add other registered keys.

The setting must contain at least one value and cannot contain
duplicates. Omitting it offers the registered harnesses. Names are
registry keys rather than a fixed enum; an unavailable implementation
cannot launch a session. Empty explicit lists and duplicates are rejected.

Sessions store their selected harness and resume identity. A thread can
have none or several, including different harnesses. Reordering the
project list does not rewrite existing sessions. A personal new-thread
preference can choose a default session or show the session picker.

The singular `agent:` key is not accepted: a file that has it is
refused as having an unknown key. Use `agents`.

The selected CLI must be installed, authenticated, and available on
`PATH`. See [Agents](../guide/agents.md) for session selection, runtime
integration, and concurrent operation.

### Snapshot retention

Snapshot **content** is expired on a 24-hour schedule
(orphaned blobs in `.oxplow/snapshots/` are GC'd at the same
time). Tune the retention window from the project's settings
page if the default doesn't fit (most users never touch this).

### Keeping snapshots

Settings → Capabilities chooses how snapshots are kept:

- **Keep every version** (the default) reads any file at any point in
  its history; the blob store grows with your edits, trimmed by the
  retention window above.
- **Track changes only** records what changed at every point and keeps
  no file contents: what changed, never what it said, and no blob store
  growth. Anything that needs a file's text from a past snapshot (diff
  coverage, the search index, code-quality scans and metrics over
  snapshots, wiki drift, change analysis) is skipped with a note in the
  log, and a read of such a file says the snapshot kept no contents. It
  applies from the next snapshot on; nothing already kept is deleted.
  It is offered by the bundled extension, so it appears once that is
  enabled.

### Other project keys

Everything else `.oxplow/project.yaml` accepts, with its default:

| Key | Default | What it does |
| --- | --- | --- |
| `projectName` | the directory name | Display name for the project. |
| `snapshotRetentionDays` | `7` | Days to keep snapshot *content*. `0` disables expiry entirely. |
| `metricRetentionDays` | `0` | Days to keep raw metric facts. `0` means **keep everything** — pruning is opt-in. |
| `metricDetailMaxPerProducer` | `100` | Keep each run's drill-in payload (`detail_json`) for only the newest N runs *per producer*. `0` disables the cap. |
| `metricDetailRetentionDays` | `30` | Also drop drill-in payloads older than this many days. `0` disables. |
| `snapshotMaxFileBytes` | `5242880` (5 MiB) | Files larger than this are recorded but their bytes aren't stored. |
| `scripts.maxConcurrent` | `min(4, CPU cores)` | How many script workers run at once, across collectors, effects, command scripts and the rest. A run past it waits for a slot. Minimum `1`. Only a person can set it. |
| `scripts.memoryMb` | `512` | The memory one script worker may use, in MiB, before it is stopped. Minimum `64`. Only a person can set it. |
| `injectSessionContext` | `true` | Whether oxplow prepends the session-context block (stream / worktree / branch / thread) to the agent's prompt. |
| `agentPromptAppend` | *empty* | Free text appended to every agent system prompt for this project. |
| `agentModels` | *empty* | Per-agent model override, e.g. `opencode: github-copilot/gpt-5-mini`. |
| `ai.roles` | *empty* | This project's [AI role](../guide/ai-models.md) assignments, over your own, e.g. `summarize: { provider: openrouter, model: openai/gpt-5-mini }`. Add `restrict: [tools, sql, streaming, images]` to forbid a role's model any of those. |
| `lsp.servers` | *empty* | Per-project language-server pins. |
| `iconTint` | *unset* | Hex colour composited behind this project's app icon, so concurrent windows are tellable apart. See below. |
| `extensions` | `[oxplow-bundled]` | The extensions on for the whole team. `oxplow-foundation` is always on and never listed. Only a person can set it. |
| `extensionSources` | *empty* | Where an extension's source is when it isn't `.oxplow/extensions/<name>/`: `{ <name>: <folder> }`. Only a person can set it. |
| `enableBundledExtensions` | `true` | `false` runs oxplow's own extensions from their `extensionSources` folders, deployed like any extension, instead of the copies built into oxplow — how oxplow develops itself. Honored only by a development build; read when oxplow starts. Your own `.oxplow/personal.yaml` may set it too. Only a person can set it. |

#### `iconTint`

Several oxplow windows open at once look identical in the Dock. `iconTint`
paints a colour behind this project's icon:

```yaml
iconTint: "#c2410c"
```

Accepts `#rrggbb`, the `#rgb` shorthand, or either without the `#`. A value
that isn't a colour is a startup error rather than a silent no-op. Omit the key
to leave the stock icon alone.

**macOS only** — it's a no-op elsewhere. It changes the Dock icon of the
running app; it does not alter the installed `.app` on disk, so it applies
equally to a release build and a `cargo`-built one.

The four metric blocks — `measures`, `collectors`, `metrics`, and
`dimensions` — are covered in [Metrics](../guide/metrics.md), which
also explains why you normally let the agent write them rather than
editing them by hand.

### Metric retention vs. detail compaction

These two do different things, which is why their defaults differ.

`metricRetentionDays` **deletes** old captures, and their facts go with
them — so a metric's history gets shorter. That's a real trade
(per-test drill-down, flakiness horizon), so it defaults to `0` = keep
everything and you opt in.

`metricDetailMaxPerProducer` / `metricDetailRetentionDays` only
**compact**: they null out a run's `detail_json` — the verbatim payload
behind the effort Tests/Coverage panel — while the capture and every one
of its facts stay. No metric value, trend point, or count ever changes,
which is why compaction is on by default. All you lose is the ability to
drill into an *old* run's per-file/per-test detail.

The count cap is the one that matters on a busy repo. That payload runs
~0.5 MB per coverage run, so a project running tests dozens of times a
day accumulates hundreds of MB in weeks — faster than any age window
catches up with. The age bound is the complement: it reaches a project
that has gone quiet, which a count cap never does. Set either to `0` to
disable it; a capture is compacted if *either* bound matches.

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

1. `.git/` is always ignored, and so is oxplow's own state under
   `.oxplow/` — with two exceptions. `.oxplow/wiki/` is always watched
   (your wiki pages get Local History even though they aren't
   committed), and anything `.oxplow/.gitignore` un-ignores with a `!`
   is watched too. That last one is why `project.yaml` shows up in
   history and effort attribution: oxplow writes `!project.yaml` there
   so the config is shareable. Negate another file yourself and it
   becomes visible the same way.
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

### Testing (tests & coverage)

The `testing` block says how your tests run; **report collectors**
read what a run writes and attach it to each effort: the individual
tests, diff coverage and analyzer findings. Point a collector at a
report file your test run already emits; after each test run oxplow
sees, it reads the reports that run wrote, so a polyglot repo lights
up per stack. Run `/oxplow:configure` to wire this up automatically.

```yaml
# .oxplow/project.yaml
testing:
  command: make test-report          # surfaced to the agent
  fastCommand: make test-fast        # the same without coverage, for iterating
  runPatterns: [pytest]              # extra substrings that count as a test run
  agentHint: Run tests with make test-report.
```

The collectors are declared in the project's own extension, not in
`project.yaml` (which takes no `collectors:`, `measures:`, `dimensions:`
or `metrics:` definitions, only `use:` entries and `enabled: false`
markers). Create `.oxplow/extensions/project/extension.yaml` with a
`manifest: 2`, `name: project` and `intent:` header when it is missing:

```yaml
# .oxplow/extensions/project/extension.yaml
collectors:
  - { id: tests.rust_coverage, records: coverage, entry: "oxplow:lcov", report: { path: target/coverage/lcov.info }, on: { run: test } }
  - { id: tests.rust_junit, records: tests, entry: "oxplow:junit", report: { path: target/nextest/default/junit.xml }, on: { run: test } }
  - { id: tests.desktop_junit, records: tests, entry: "oxplow:junit", report: { path: apps/desktop/test-report.xml }, on: { run: test } }
```

`records` is `tests`, `coverage` or `analysis`. The bundled parsers are
`oxplow:junit`, `oxplow:lcov`, `oxplow:cobertura`, `oxplow:jacoco`,
`oxplow:clippy` and `oxplow:eslint`. Coverage numbers come only from
oxplow parsing the report, never from the agent, so they stay
trustworthy. Each collector's last run shows in Settings → Data; one
whose report fails to parse three times in a row is turned off until
you turn it back on.

#### A report format with no bundled parser

Give the collector its own parser — no recompile, no change to oxplow.
A parser is a small script that maps a report into oxplow's
coverage/test/analysis shape:

```yaml
# .oxplow/extensions/project/extension.yaml
collectors:
  - id: tests.clover
    records: coverage
    runtime: jaq                        # jaq (jq) | starlark | exec
    entry: collectors/clover.jq         # the script file, relative to the extension folder
    report: { path: target/clover.xml, format: xml }   # pre-parse: text | json | xml | lcov | lines
    on: { run: test }
```

The script lives in its own file (`entry`, relative to the extension folder, e.g.
`.oxplow/extensions/project/collectors/clover.jq`; `report.path` stays
project-relative), not inline in the yaml. The host reads it and pre-parses the report for you
(per `format`), so the script only reshapes JSON. `jaq` (jq) is the
simplest for XML/JSON; `starlark` covers logic jq can't express; `exec`
runs an external program — `entry` is the executable — (raw report on
stdin, JSON on stdout) as a last resort. jaq and starlark run in-process
and sandboxed, so their output is trusted as measured; an `exec` parser
needs the extension to declare `permissions: { process: true }`, and its
output is flagged lower-trust in the UI.

An extension's permissions are what it may reach: programs (`process`),
hosts (`network`), credentials (`secrets`), models (`ai`), and so on. You
approve them when you deploy the extension (Settings → Extensions), and a
deploy that adds a permission asks again. `process` is declared, not
confined: a program reads and writes files as you, and its network is
confined only on macOS. Every run is recorded in `v_extension_run`, with
what each permission decided.

Ask the agent for the details: the `oxplow-collection` skill covers host
helpers, the exact output schemas and a worked example.

### LSP servers

Installs are **explicit**, not automatic. Use the **Language
Servers** section of the Settings page, or ask the agent: it proposes
the `oxplow.lsp.install_server` command, and the install runs once you
approve it. Nothing is fetched just because
you opened a file.

Servers install into `.oxplow/lsp/` (downloads are cached
separately in `.oxplow/lsp-cache/`), and the proxy hands the right
binary to whichever stream asked. Which server serves a file is
resolved by language id, not by root markers.

You can also pin servers per project under an `lsp.servers` block
in `.oxplow/project.yaml`. If a server you need isn't packaged, use the Mason-backed installer
(**Language servers** in Settings).

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
