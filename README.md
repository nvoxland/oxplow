# Oxplow

Oxplow helps software developers understand and steer development while
coding agents do the implementation work. Start with descriptions,
questions, evidence, and connected views of what is happening; drill down
to the real code when you need to verify an explanation or make a change.

Oxplow is explicitly not an IDE. Its pages, navigation, data model, and
extension system are built around understanding software and the work
that changes it. Monaco and language servers provide code drill-down;
the editor does not define the product's structure.

## Build the visibility you need

Data collection, analysis, metrics, and dimensions are central to oxplow.
Built-in integrations collect development activity, code and change
information, test results, and agent usage. Developers and their agents
can add sources and analyses, connect them to existing data, and build
the descriptions, reports, and workflows useful to their project.

Ask an agent a question first. When an answer or workflow is worth
keeping, have it build an extension: a collector, model, metric, page,
page, command, or event reaction. Extensions are ordinary project files
that can be personal or shared with a team. The long-term goal is to
make broader sharing practical too.

The foundation supplies shared identity, references, provenance, query
services, action gates, and execution infrastructure. Extensions should
focus on their own business value. Repository-supplied code is not
trusted merely because an agent wrote it or a teammate shared it:
programs and privileged contributions use explicit declarations and
human consent. These guardrails are part of the platform, with current
limits documented in the [extension reference](docs/reference/extensions.md).

## What it provides today

- **Connected evidence.** Turns, tool calls, changes, snapshots, test
  runs, metrics, claims, decisions, and knowledge can be read through
  published SQL models. Observations, claims, and AI inference retain
  their provenance.
- **Collectors, metrics, and dimensions.** Collect raw data, derive new
  entities and facts, aggregate measures, and compare them across the
  dimensions relevant to your system and development process.
- **Custom views and workflows.** Extensions contribute models, pages,
  pages, panels, guidance, commands, effects, and sandboxed components.
  Bundled features use these mechanisms as well.
- **Parallel streams.** A stream has its own branch and worktree. Each
  stream has one writer thread; other threads can research, review, and
  record understanding without editing that stream's worktree.
- **Threads and agent sessions.** A thread is a line of work, with its
  own tabs and context. It can have no agent session or several, using
  terminal harnesses.
- **Work tracking and history.** Activity is recorded without requiring
  task bookkeeping. Optional effort policies group work and link it to
  work items. Snapshots preserve working-tree history alongside git.
- **Code drill-down.** File, diff, history, and language-server views let
  you check the implementation behind an explanation.
- **Shared actions.** Commands validate inputs and apply permissions,
  confirmation, audit, and transaction rules. Agents use the same
  semantic data and command surfaces through MCP.

Commits are user-driven. Oxplow does not require a task before an edit
or refuse an agent's stop because bookkeeping is incomplete. Terminal
input originates from the person; recorded AI API
computations are a separate service.

## Architecture

A Tauri desktop shell opens project windows, each connected to a
per-project Rust daemon. The daemon owns SQLite state, the event log,
semantic models, integration services, and extension execution. The
React frontend renders the page graph, agent sessions, and
code drill-down. Read contracts connect data from different sources;
commands provide the action side.

See the [architecture vision](docs/philosophy/architecture-vision.md)
for the product direction and [.context/](.context/) for internal design
and implementation notes.

## Running

See [DEV.md](DEV.md) for building and running from source and
[the documentation](https://nvoxland.github.io/oxplow/) for usage.

## Status

Oxplow is early and actively evolving. Broader extension compatibility,
execution containment, and authoring ergonomics are still being improved.
Database migrations are the forward path for stored data. The next work
is specified in the [foundation roadmap](.context/foundation-roadmap.md).

## License

TBD.
