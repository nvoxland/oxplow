# Agents

Oxplow supports **Claude Code**, **Codex**, and **OpenCode** as agent
backends. A
project can enable either one or both, and every thread records which
agent it runs.

The agent is a property of the thread, not the stream. That means a
Claude thread and a Codex thread can run at the same time in one
project, including on the same stream. The stream still has exactly one
writer thread; all other threads are read-only regardless of which
agent they use.

## Enable agents for a project

Open the project **Settings** page and enable any combination of
Claude, Codex, and OpenCode.
Use **Up** and **Down** to set their priority. The first enabled agent is
the default when a new thread is created.

The same setting lives in `.oxplow/project.yaml`:

```yaml
agents:
  - claude
  - codex
```

The supported values are `claude` and `codex`. At least one must be
enabled, and an agent cannot appear twice. When the setting is omitted,
Oxplow defaults to:

```yaml
agents:
  - claude
```

See [Settings](../reference/settings.md#agents) for validation and
legacy configuration details.

## Choose an agent for a thread

When more than one agent is enabled, the new-thread row includes an
agent selector. Its initial value is the first entry in `agents`. When
only one agent is enabled, Oxplow uses it without showing a selector.

The assignment is fixed after the thread is created. Thread Settings
shows the assigned agent, but does not change it. Create another thread
to switch agent implementations while preserving the original thread's
session and history.

Changing the project-level `agents` setting affects the choices for new
threads. It does not rewrite existing threads, so a previously-created
thread keeps its assigned agent even if that agent is later removed
from the enabled list.

## What Oxplow provides to each agent

Both agents receive the same project and thread context and connect to
the same Oxplow MCP control plane. They can work with tasks, threads,
follow-ups, wiki pages, LSP tools, snapshots, and the other project
primitives exposed through MCP.

Oxplow generates the integration files needed by each CLI under
`.oxplow/runtime/` whenever it starts an agent session:

- Claude Code uses `.oxplow/runtime/claude-plugin/` for hooks, skills,
  commands, and MCP configuration.
- Codex uses `.oxplow/runtime/codex-plugin/` for hooks, skills, and MCP
  configuration.

The generated files are implementation details and are rewritten as
needed. Do not edit them as project configuration.

Each thread also has its own terminal session and resume identity. A
Claude session is never resumed as Codex, or vice versa. Both CLIs run
with the stream worktree as their working directory.

## Control rules are agent-independent

Agent selection does not change Oxplow's safety model:

- Only the stream's writer thread may edit project files.
- A writer needs an `in_progress` task before authored edits.
- Lifecycle hooks feed activity and stop events into Oxplow.
- MCP calls are scoped to the current thread and stream.
- Efforts and Local History attribute changes to the same task model.

The generated integration translates each CLI's hook and configuration
format into those shared rules. See [Agent control](agent-control.md)
for the full lifecycle.

## CLI prerequisites

Oxplow launches the selected agent's command from your environment. The
corresponding CLI must already be installed, authenticated, and
available on `PATH`:

- `claude` for Claude Code
- `codex` for Codex
- `opencode` for OpenCode

Enabling an agent in `.oxplow/project.yaml` does not install its CLI or configure
its provider credentials.
