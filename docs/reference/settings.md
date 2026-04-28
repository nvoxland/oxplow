# Settings

Oxplow stores per-project state in `.oxplow/state.sqlite` and
per-user state in your OS user-config directory. Most settings
are toggles in the UI; a handful live in JSON files for
advanced use.

## Where settings live

- **Per-project** — `.oxplow/state.sqlite`. Add `.oxplow/` to
  `.gitignore`; oxplow does not commit project state.
- **Per-user** — `~/Library/Application Support/oxplow/`
  (macOS) or the platform equivalent. Window position, recent
  projects, theme preference.

There is no global config file you need to edit to get started.
Sensible defaults; opinionated product.

## Settings worth knowing

### Writer thread

Per-stream. Exactly one thread is the writer. Other threads
are read-only. Switch the writer from the thread tab kebab.
Switching kicks any in-flight write attempt on the old writer
back through the hook (which fails it cleanly).

### Stream and thread custom prompts

Each stream and thread has its own settings page (open from
the tab kebab → Settings) with a custom prompt field appended
to the agent's system prompt at launch. Use it for
stream-specific framing ("you're on the migration branch,
priority is not breaking schema") or thread-specific framing
("research only — never edit").

### Agent kind

Per-thread. Default is Claude Code. `copilot` is also
supported but skips the oxplow plugin plumbing — no
filing-enforcement, no Stop directives, no MCP tools.

### tmux mode

Per-thread. Default on. The agent process runs inside a tmux
session so it survives oxplow restarts. Toggle from the agent
tab kebab.

### Snapshot pruning

Global. Snapshots from closed work items are pruned after a
configurable retention window. Default: 30 days. Set to `0` to
disable pruning entirely.

### LSP servers

Per-language. Oxplow auto-detects common servers
(`typescript-language-server`, `gopls`, `rust-analyzer`,
`pyright`, etc.) on `PATH`. To override, edit
`~/.config/oxplow/lsp.json` (or platform equivalent):

```json
{
  "typescript": {
    "command": "typescript-language-server",
    "args": ["--stdio"]
  }
}
```

### Theme

Dark only. Oxplow is dark-only on purpose — Monaco is pinned
to `vs-dark` and the UI tokens are calibrated for it.

### Telemetry

Off. Always. There is no telemetry to configure.

## Settings the agent can change

None. The MCP surface deliberately does not expose product
settings — the agent operates on intent, files, work items,
and notes. Configuration is the human's job.

## Resetting

Wipe `.oxplow/` to reset a project. Wipe the user-config
directory to reset everything. Both are safe; oxplow rebuilds
what it needs on next launch (your work-item history goes with
the project state, though, so don't do it casually).
