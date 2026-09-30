# External providers

A provider is a program oxplow talks to — an issue tracker's bridge, a
docs system's — that implements one of oxplow's capabilities (work items
first) outside the app (P5.D, `target-architecture.md` §10). This doc
covers the protocol (D1), the fake provider (D2) and the host with its
consent and spawn rules (D3); instances and health (D4) and the
conformance kit (D5) extend it as they land.

## The protocol (`crates/oxplow-provider-protocol`)

**JSON-RPC 2.0 over NDJSON on stdio**: one message per line
(`codec::Message` — request, response, error, notification). A small,
symmetric `Peer` (`peer.rs`) is both sides' connection: `start` /
`request` / `call` (typed) send a request and await its reply through an
outstanding-id table, `notify` and `cancel` send notifications, `respond`
answers the other side, and whatever isn't a reply arrives on the
`Incoming` channel. Ids are the sender's own, from 1. When the stream
ends, replies still awaited fail. It is deliberately not the
`agent-client-protocol` layer: that one is ACP-typed and actor-heavy.

**Notifications about an in-flight request:** `$/cancel { id }` (host →
provider: stop; it answers `Cancelled`), `$/progress { id, message?,
fraction? }`, `$/record { id, entity, row }` (a `read`'s rows, streamed
before its result) and `$/state { id, state }` (a read's checkpoint, the
cursor the next read resumes from).

**The meta-model** (`model.rs`) — Rust types defined once (serde +
schemars), host → provider:

| method | params → result | |
|---|---|---|
| `initialize` | `InitializeParams { protocol_version, host }` → `InitializeResult { protocol_version, provider, capabilities, commands, event_types, collectors, config_schema }` | what the provider is and declares: `CapabilityDecl { capability, features }`, `CommandDecl { name, summary, input_schema, confirm, effect, undoable }`, `EventTypeDecl { type, v, schema }`, `CollectorDecl { name, entity, description }` |
| `check` | `CheckParams { config, credentials }` → `CheckResult { problems, handle? }` | validate an instance's config (credentials by name — the values stay in the keychain and reach the provider through its environment); `Problem { path, message }`; a clean check returns the opaque `Handle` the other calls carry |
| `discover` | `{ handle }` → `DiscoverResult { entities }` | the entities the instance can read (`EntityDecl { name, description, schema }`) |
| `invoke` | `InvokeParams { handle, command, input }` → `InvokeResult { result, events, inverse? }` | run a declared command; the host logs its `EventDraft { type, v, payload, subject }`s |
| `read` | `ReadParams { handle, collector, state? }` → `ReadResult { records }` | run a collector, streaming `$/record` and `$/state` first |
| `shutdown` | → `null` | |

`PROTOCOL_VERSION` is `"1"`.

**Errors** (`errors.rs`): JSON-RPC's standard codes plus
`NotConfigured` (-32001), `Auth` (-32002), `RateLimited` (-32003,
`data.retry_after_ms`), `InvalidInput` (-32004, `data.field`) and
`Cancelled` (-32800). `ProtocolError` is the typed form; the conversion
keeps the data both ways.

**Schema goldens** (`schemas/<name>.json`, one per wire type — the
methods' params and results, the four notifications, the error object):
`tests/schemas.rs` keeps each equal to the schema generated from its Rust
type and every file owned by a wire type (the `event_schemas.rs`
discipline; `OXPLOW_BLESS=1` writes a new one). A shipped wire shape is a
contract: change it by bumping `PROTOCOL_VERSION`. `schemas::validate`
checks a message against its golden (`for_message` picks it by method),
which the conformance kit runs on every message. Generating TypeScript
bindings for what the UI shows comes with the host (D3/D4); there is no
stub generation for provider authors.

## The fake provider (`crates/oxplow-provider-fake`)

A scripted **work-items** provider, lib + bin (the `oxplow-acp-fake`
pattern), that the host's tests and the conformance kit drive over real
stdio. `declarations()` is its `InitializeResult`: the `work_items`
capability (hierarchy, comments and links; not
`in_progress_opens_effort`), the commands `create` / `update` /
`transition` / `link` / `comment` (effect `record`, no confirmation), the
`work_item.recorded@1` event type with the core schema, a `work_items`
collector over the `work_item` entity, and a config schema requiring
`team`. `check` returns handle `fake:<team>` or a `/team` problem. Items
live in memory as `work_item:fake:W-<n>` with native states `Backlog`,
`Doing`, `Stuck`, `Shipped`, `Dropped` (one per canonical state); every
`invoke` returns the `work_item.recorded` events the host logs. `read`
streams each item after the cursor as `$/record`, then `$/state
{ cursor }`, then `{ records }`.

**Script hooks** — `OXPLOW_FAKE_HOOKS` at spawn, or a `fake/hooks
{ hooks }` notification mid-session (comma-separated):
`fail-next:<n>` (the next n check/invoke/read fail `Internal`),
`slow:<ms>` (invoke and read wait first; `$/cancel` interrupts them with
`Cancelled`), `crash` (the next request drops the connection; the binary
exits 3) and `bad-declarations` (`initialize` answers something other
than `declarations()`, for the host's handshake check).
`tests/stdio.rs` pins all of it through a `Peer`, validating the streamed
notifications against the goldens.

## The host (`crates/oxplow-app/src/providers/`)

**The manifest kind** (`spec.rs`): `providers:` is an experimental kind,
so only a private extension's are loaded (onto `Extension.providers`; a
disabled extension has none):

```yaml
providers:
  - id: fake                     # the ref segment and command namespace
    capability: work_items       # the only one a provider implements today
    entry: bin/provider          # a program in the extension folder
    args: [--stdio]
    env: [TRACKER_URL]           # host variables passed through by name
    credentials: [token]         # keychain values, as env
    network: [api.example.com]   # hosts it may reach
    declarations: provider.json  # its InitializeResult, checked in
```

The loader refuses (into `errors`) an id that isn't lowercase
snake_case, is `oxplow` or a core namespace, or repeats; an unknown
capability; an entry or declarations path outside the folder (or the
manifest, or under `lenses/`); a bad host pattern; and declarations that
don't parse, speak another protocol version, lack the named capability,
or — for `work_items` — lack `create` / `update` / `transition` (and
`link` / `comment` when its features say so), or claim
`in_progress_opens_effort` (only oxplow's tasks do).

**Consent precedes execution** (`exec_consent`, `ProgramKind::Provider`,
key `provider:<ext>/<id>`): the approval hash covers every file in the
extension folder but the manifest and `lenses/` — the entry and the
declarations file among them — plus the entry path, `args`, `env` names,
`credentials` and `network`. So a changed declaration is a new version,
shown unapproved in Settings → Data → Programs (with its grants listed)
until a person approves it again. Every start re-checks it, restarts
included.

**The spawn** (`host.rs`, `connect`) mirrors an `exec` source: a
scrubbed environment (PATH, HOME, the declared `env` names, the
credentials from the keychain account `source:<project>:<ext>:<name>`,
`OXPLOW_EXTENSION_DIR`, `OXPLOW_PROVIDER_ID`), the egress proxy and
`sandbox-exec` where the OS enforces `network`, stderr to the log, and
`kill_on_drop`. Then **the handshake**: the live `initialize` must equal
the approved declarations (`HostError::DeclarationsChanged` names the
first difference), and `check` of the instance's config must return a
handle (`HostError::Unconfigured { problems }` otherwise). Requests from
the provider are answered `MethodNotFound`.

**The registry** (`registry.rs`, `Services.providers`): `enable(ext,
spec, config)` starts an instance and only then registers its declared
commands on the bus as `<id>.<name>` (`External`, `Experimental`, all
invokers; confirm / effect / undoable as declared) and its capability
provider (`ExternalWorkItems`) in `Services.work_items`; any refusal
registers nothing. `disable(id)` removes both and kills the process. An
id already used as a command namespace or provider is refused, and so is
a declared event type the host doesn't know with exactly that schema (a
provider emits core types only for now; its own types are P7). One
process per instance: a call that finds it dead (`Peer::is_closed`)
starts it again, and a failed start backs off exponentially (2 s … 60 s)
before the next try. A command's run invokes the process and hands the
bus its result, its inverse (as `<id>.<command>`) and its events —
refused if a type isn't declared or a `work_item.recorded` names another
provider's item.

**What D4 adds:** the `extensionInstances` config that enables instances
at boot, health (`InstanceHealth`, auto-disable after repeated failures)
and the Settings → Integrations page. Until then an instance is enabled
only by code (the tests).

**Tests** (`providers/tests.rs`) run the real fake binary (built beside
the test binary by the workspace build) through a script entry in a
temp extension: an unapproved provider is refused and registers
nothing; an edited declarations file is shown unapproved and refused;
the `bad-declarations` hook is refused naming `/commands`, a config
without `team` names `/team`; and the work-items conformance suite
passes through `ExternalWorkItems` over the fake.
