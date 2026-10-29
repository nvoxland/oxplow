# Extensions

This doc covers how anything that **measures or visualizes** is added to
oxplow: the `extension.yaml` format, pages, slots, actions and alerts, and
`oxplow-bundled`, the one extension that ships with oxplow.

**Terms.** oxplow's one vocabulary for what it's built from. Code, docs,
skills and UI copy all use these words, with these meanings
([usability.md](./usability.md) "Copy names things as a person knows
them"):

| Term | Means |
|---|---|
| extension | the package: a folder with an `extension.yaml` |
| contribution | one thing an extension declares (a page, collector, command, effect, provider …) |
| capability | a swappable slot a project chooses an implementation of (`work_items`, `effort_policy`, `snapshots`; `vcs` is fixed) — [work-tracking.md](./work-tracking.md) "Capabilities" |
| provider | whatever implements a capability. Qualify it where it isn't obvious: an **external provider** is a process speaking the provider protocol ([providers.md](./providers.md)); an **AI provider** is API model access ([ai-providers.md](./ai-providers.md)) |
| scope | what a contribution may call on the host (`sql.read`, `threads.write`) — [commands.md](./commands.md) "Scopes" |
| access | `view` / `read` / `record` / `write`: what a command or a scope may do |
| effect | an extension's reaction to events (extensions-events.md "Effects") |

"Plugin" is only a Claude Code or opencode plugin. Two more terms keep
authority honest: a **need** is what a contribution says it requires (a
capability, a feature, a scope); a **grant** is what a person approved
(a program's hash and its declared access). A need is never a grant.


## Which doc to read

The extensions docs are split by concern. Read the one for what you are
touching; each is under about 700 lines. Section names in quotes are the
headings inside that doc.

| If you're touching… | Read |
|---|---|
| `extension.yaml` (manifest v2), contribution descriptors, `configuration:`, private state, the design rules | [extensions-manifest.md](./extensions-manifest.md) |
| How pages run today: page files, ids, reading from the live set, params, checks, IPC/MCP surfaces, panels, page actions, the desktop UI | [extensions-page-host.md](./extensions-page-host.md) |
| The page YAML: metadata, params, loaders, view nodes, actions, checks, agents; change links; page layout | [extensions-pages.md](./extensions-pages.md) |
| Slots, replacements (`ui.mounts`), commands in core menus, decorators | [extensions-slots.md](./extensions-slots.md) |
| Custom components (the frame, the protocol, testing one), Explore Data | [extensions-components.md](./extensions-components.md) |
| Sources and deployments, builds, `extensionSources`, sharing, disabling, reviewing by impact | [extensions-deploy.md](./extensions-deploy.md) |
| Health, disable and repair; candidate publication, vocabulary generations, runtime adapters, ingress | [extensions-activation.md](./extensions-activation.md) |
| Permissions and approval, the run context, services, grants, secret references | [extensions-permissions.md](./extensions-permissions.md) |
| The event envelope, event types, resources, ordering, effects | [extensions-events.md](./extensions-events.md) |
| Collectors (sources), contributing metrics, jobs, AI profiles | [extensions-jobs.md](./extensions-jobs.md) |
| Extension commands, capability implementations, skills, advisories, the agent MCP surface | [extensions-commands.md](./extensions-commands.md) |
| The SDK (`oxplow extension new|check|test`), fixtures, answerability | [extensions-sdk.md](./extensions-sdk.md) |
| `oxplow-foundation`, `oxplow-bundled`, the review packet, `examples/extensions/` | [extensions-bundled.md](./extensions-bundled.md) |

Code: `crates/oxplow-app/src/extension/` (the loader, activation,
catalog, deploy, jobs), `crates/oxplow-app/src/pages/` (page schema,
runner, actions, checks), `crates/oxplow-sdk` (the SDK). The user-facing
reference is `docs/reference/extensions.md`.
