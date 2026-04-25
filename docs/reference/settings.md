# Settings

Oxplow stores per-project state in `.oxplow/state.sqlite` and
per-user state in your OS user-config directory. Most settings
are toggles in the UI; a handful live in JSON files for advanced
use.

## Where settings live

- **Per-project** — `.oxplow/state.sqlite` (committed: no — this
  is in your `.gitignore`).
- **Per-user** — `~/Library/Application Support/oxplow/`
  (macOS) or the platform equivalent. Includes window position,
  recent projects, theme preference.

There is no global config file you need to edit to get started.
Sensible defaults; opinionated product.

## Settings worth knowing

### Auto-commit mode

Per-stream toggle. When on, the agent commits at every Stop
boundary; when off, you manage commit points by hand. Default:
on.

Set from the stream's overflow menu in the rail.

### Writer thread

Per-stream. Exactly one thread is the writer. Other threads are
read-only. Switch the writer from the thread list in the Plan
pane. Switching kicks any in-flight write attempt on the old
writer back through the hook (which will fail it cleanly).

### Stop-hook task audit

Global. When on (default), every Stop fires a "verify your
`in_progress` items are still in progress" nudge to the writer.
You can disable it for streams where the audit is more friction
than it's worth.

### Snapshot pruning

Global. Snapshots from closed work items are pruned after a
configurable retention window. Default: 30 days. Set to `0` to
disable pruning entirely (your disk's problem after that).

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

Light / dark / system. Set from the **View** menu.

### Telemetry

Off. Always. There is no telemetry to configure.

## Settings the agent can change

None. The MCP surface deliberately does not expose product
settings — the agent operates on intent, files, and work items.
Configuration is the human's job.

## Resetting

Wipe `.oxplow/` to reset a project. Wipe the user-config
directory to reset everything. Both are safe; oxplow rebuilds
what it needs on next launch (your work-item history goes with
the project state, though, so don't do it casually).
