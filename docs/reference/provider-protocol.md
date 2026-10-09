# Provider protocol

A provider is a program that implements one of oxplow's capabilities
for it. Today that's the work list: a provider can make oxplow's tasks
live in another tracker. oxplow starts the program, talks to it over
stdin/stdout, and reads and writes through it.

```sh
oxplow extension new provider my-tracker   # declarations, a stub program, a test config
oxplow extension new provider my-policy --capability effort_policy
oxplow extension new provider my-agent --capability agent_harness
oxplow extension new provider my-shutter --capability snapshots
oxplow extension test my-tracker           # handshake, examples, conformance suite
```

An agent harness that can tell its subagents apart declares the
`subagents` feature and the two verbs it enables: `prompt { body }`
answers `{ prompt }` — `{ kind: person, text }`, or `{ kind: handback,
subagent: { id, kind? } }` when a subagent's report arrives as a prompt
(it opens no turn) — and `subagent { body }` answers `{ subagent: { id,
kind? } }` for a subagent hook. Its `tool_use` answer may carry the same
`subagent` for a call a subagent made. Without the feature a prompt is a
person's. The conformance suite checks that a plain `{ "prompt": … }`
body reads as a person's and an empty body names no subagent.

A snapshots provider keeps the project's snapshots in its own way and
declares the `snapshots` capability with a `contents` feature. It answers
three verbs, each `confirm: never`, `access: record`, over inputs that
refuse unknown keys (`read_at` only with `contents`):

- `mark { stream, worktree, trigger, thread?, turn?, effort?, budget_ms?,
  parent? }` → `{ handle, unchanged, file_count, branch?, revision? }`:
  it names the worktree's state with its own `handle`. `parent` is the
  handle of the last state it marked (`null` when there is none, and the
  whole tree is then new); `unchanged` says the tree equals the parent's.
- `changed { stream, from, to }` → `{ changes: [{ path, kind, identity,
  size }] }`: `kind` is `added`, `modified` or `deleted` (no `identity` or
  `size` for a deletion), `path` is relative to the worktree with `/`, and
  `identity` is the file's xxh3-128 as 32 lower-case hex digits. A `from`
  of `null` is the empty tree.
- `read_at { handle, path }` → `{ bytes }`, base64: a file's bytes at a
  marked state. An unknown handle or path is `InvalidInput`.

A knowledge provider keeps the project's pages in its own store and
declares the `knowledge` capability. It answers `write_page { slug, title?,
body, verified_refs, removed_refs }` → `{ page }`, `delete_page { slug }`
and `link { page, target }` → `{}`, and emits `knowledge.page.recorded@1`
with `{ page: { ref: "wiki:<slug>", title, body, refs, updated_at,
deleted? } }` for every page it writes or deletes (each verb answers it,
and a `knowledge_pages` collector over the entity `knowledge_page`
streams them all). `refs` states the page's outbound references in
oxplow's grammar (`file:<path>`, `wiki:<slug>`); oxplow never parses a
foreign body.

## Declaring one

```yaml
# oxplow/extensions/my-tracker/extension.yaml
providers:
  - id: tracker
    capability: work_items
    entry: bin/provider            # the program, inside the folder
    declarations: provider.json    # what `initialize` answers, checked in
    env: [HOME]                    # environment variables it may read
    credentials: [TRACKER_TOKEN]   # set per instance; reach it as env vars
    network: [api.tracker.example] # hosts it may reach (enforced where the OS can)
    needs: [sql.read]              # what it may call of oxplow (host/call)
```

- The program runs from a verified copy of the extension folder, with
  a scrubbed environment: `PATH`, `HOME`, the `env` names, its
  credentials, `OXPLOW_EXTENSION_DIR` and `OXPLOW_PROVIDER_ID` (the
  instance it runs as, which is the middle of its refs:
  `work_item:<id>:…`).
- It runs only once a person approves it in Settings → Data. The
  approval covers every file in the folder and the grants above, so any
  change needs approving again.
- `declarations` is the provider's answer to `initialize`, checked in.
  If the running program says anything else, it doesn't start.
- A server that speaks MCP can be a provider through oxplow's adapter
  instead of an `entry`: `adapter: { mcp: { command: [bin/server] } }`,
  or `{ mcp: { url: … } }` for one already running. A mapping file says
  which of its tools are the capability's verbs.

## The wire

JSON-RPC 2.0, one message per line (NDJSON), on the program's stdin and
stdout. Its stderr goes to oxplow's log. Protocol version `4`.

Host → provider:

| Method | Params → result |
|---|---|
| `initialize` | `{ protocol_version, host }` → its declarations: `{ protocol_version, provider, capabilities, commands, event_types, collectors, config_schema }` |
| `check` | `{ config, credentials }` → `{ problems, handle? }`. A clean check returns a `handle` the other calls carry. `credentials` are names only; the values are in its environment |
| `discover` | `{ handle }` → `{ entities }` |
| `invoke` | `{ handle, command, input, idempotency_key? }` → `{ result, events, inverse? }` |
| `read` | `{ handle, collector, state? }` → `{ records }`, after streaming the records |
| `shutdown` | → `null`. Finish what's in flight, persist, answer, and exit |

Provider → host:

| Method | |
|---|---|
| `host/call` (request) | `{ key?, scope, args }` → the scope's answer. `sql.read` with `{ sql, params? }` reads oxplow's `v_*` models. Only scopes in `needs` |
| `host/changed` (notification) | `{ collectors? }`: something you read changed (a webhook arrived, say). oxplow reads those collectors now instead of at the next poll |

Notifications about a request in flight, by its `id`:

- `$/cancel { id }` (host → provider): stop; answer `Cancelled`.
- `$/progress { id, message?, fraction? }`: shown while a read runs.
- `$/record { id, entity, row }`: one row of a `read`.
- `$/state { id, state }`: a checkpoint. oxplow commits the records
  before it, with the checkpoint, and the next read resumes from it.

The JSON Schema of every message is checked in under
`crates/oxplow-provider-protocol/schemas/`, and `oxplow extension test`
validates every message against them.

### Errors

The standard JSON-RPC codes, plus:

| Code | Name | `data` |
|---|---|---|
| -32001 | `NotConfigured` | |
| -32002 | `Auth` | `credential`: the one the service refused, if you know |
| -32003 | `RateLimited` | `retry_after_ms` |
| -32004 | `InvalidInput` | `field`: a JSON pointer |
| -32800 | `Cancelled` | |

## What oxplow expects

- **Writes are idempotent** when you declare
  `features.idempotent_writes`: an `invoke` sent again with the same
  `idempotency_key` does the write once and answers the same. oxplow
  re-sends a write whose reply was lost.
- **Reads checkpoint.** Send `$/state` at least every 10,000 records.
  A read that streams more without one fails. Records after the last
  checkpoint of a read that succeeds still land.
- **Calls are bounded.** `check` and `invoke` time out after 60
  seconds, and a read that sends nothing for 60 seconds is cancelled.
  A `RateLimited` reply waits and retries once when the wait is short.
- **Lines are bounded.** A message over 16 MiB ends the connection.
- **Stopping is orderly.** oxplow sends `shutdown` and kills the
  process if it hasn't exited 5 seconds later.
- **Exits are noticed.** A process that exits on its own is restarted
  after a backoff. Three failures in a row (calls, starts or exits)
  disable the instance until a person turns it back on.
- **You emit only your capability's events.** A work list's are
  `work_item.recorded`, for its own items.

## Testing

`oxplow extension test` runs each declared provider against a test
config (`fixtures/provider-<id>.yaml`): the handshake against
`declarations`, every intent example, every message against the
schemas, a golden transcript (`--bless` writes it) and the
capability's conformance suite. The work-items suite creates, updates,
transitions, links, comments on and deletes an item through you, and
reads each back.
