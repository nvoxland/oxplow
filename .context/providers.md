# External providers

An external provider is a program oxplow talks to — an issue tracker's bridge, a
docs system's — that implements one of oxplow's capabilities outside the
app (P5.D): one core gives a provider contract (the agent harness,
snapshots, knowledge, a filesystem; "What a provider may implement"). This doc
covers the protocol (D1), the fake provider (D2), what a provider may
implement, the host with its permission and spawn rules (D3), instances with health (D4), the
conformance kit with `oxplow extension test` (D5). There is no task
tracker contract: tasks are a shared model that stores fill
([tasks.md](./tasks.md)).

## The protocol (`crates/oxplow-provider-protocol`)

**Host secret references.** A provider credential may be declared as
`{name, secretRef:{schemaVersion:1,id}}`, a reference into the host's secret
store. The reference is scoped to the primary project and owning extension,
including for global instances; it is inert unless the extension declares
`secrets` and the deployment is live. No provider receives a resolved secret today; package programs, OAuth
declarations and arbitrary environment variable names cannot consume a
reference. See [extensions-permissions.md](./extensions-permissions.md)
"Host-owned secret references" for persistence and lifecycle details. Runtime
validation and the human settings form remain unfinished.

**JSON-RPC 2.0 over NDJSON on stdio**: one message per line
(`codec::Message` — request, response, error, notification). A small,
symmetric `Peer` (`peer.rs`) is both sides' connection: `start` /
`request` / `call` (typed) send a request and await its reply through an
outstanding-id table, `notify` and `cancel` send notifications, `respond`
answers the other side, and whatever isn't a reply arrives on the
`Incoming` channel — except what a request started with
`start_streaming` is told about: `$/progress`, `$/record` and `$/state`
naming its id go, in order, to a channel of its own, registered before
the request is sent and closed when its reply arrives (P7.A3). Ids are the sender's own, from 1. When the stream
ends, replies still awaited fail (`calls_cut_off` counts them, and
`closed()` resolves once they have). **Everything is bounded**
(`PeerLimits`): a line over `max_line_bytes` (16 MiB) ends the
connection — what was awaited fails saying so — and each channel holds
`channel_capacity` (1,024) messages, so a reader that falls behind slows
the other side (its writes block on the pipe) instead of queueing without
end.

**Notifications about an in-flight request:** `$/cancel { id }` (host →
provider: stop; it answers `Cancelled`), `$/progress { id, message?,
fraction? }`, `$/record { id, entity, row }` (a `read`'s rows, streamed
before its result) and `$/state { id, state }` (a read's checkpoint, the
cursor the next read resumes from).

**The meta-model** (`model.rs`) — Rust types defined once (serde +
schemars), host → provider:

| method | params → result | |
|---|---|---|
| `initialize` | `InitializeParams { protocol_version, host }` → `InitializeResult { protocol_version, provider, capabilities, commands, event_types, collectors, config_schema }` | what the provider is and declares: `CapabilityDecl { capability, features }`, `CommandDecl { name, summary, input_schema, confirm, access, undoable }`, `EventTypeDecl { type, v, schema }`, `CollectorDecl { name, entity, description }` |
| `check` | `CheckParams { config, credentials }` → `CheckResult { problems, handle? }` | validate an instance's config (credentials by name — the values stay in the keychain and reach the provider through its environment); `Problem { path, message }`; a clean check returns the opaque `Handle` the other calls carry |
| `discover` | `{ handle }` → `DiscoverResult { entities }` | the entities the instance can read (`EntityDecl { name, description, schema }`) |
| `invoke` | `InvokeParams { handle, command, input, idempotency_key?, context? }` → `InvokeResult { result, events, inverse? }` | run a declared command; the host logs its `EventDraft { type, v, payload, subject }`s ("Idempotency" for the key). `context` is the run it's part of — `InvokeContext { run, actor, cause?, anchors }` — for the provider to log or forward; it changes nothing about the call |
| `read` | `ReadParams { handle, collector, state? }` → `ReadResult { records }` | run a collector, streaming `$/record` and `$/state` first |
| `shutdown` | → `null` | finish what's in flight, persist, answer, exit; the host kills it after its grace |

Provider → host (protocol 3): `host/call` — `HostCallParams { key?,
scope, op?, args }` → the scope's answer: a scope the provider's
manifest `needs` (`sql.read`: `args` `{ sql, params? }` →
rows), `key` naming the idempotency key of the `invoke` it serves so the
call is recorded with that run. `Instance::invoke(run, …)` registers the
run under the key for the call's length (`HostCalls::begin`), whichever
verb it is — a command, knowledge, snapshot or harness
verb. Refused (`InvalidInput` at `/scope`)
for one it doesn't need or that isn't there, and for one naming no
`invoke` under way: nothing a provider does of oxplow is outside a run.

Provider → host (protocol 4), a notification: `host/changed` —
`HostChangedParams { collectors? }`: something it reads changed at its
service (a webhook, a watch), so the host reads those collectors (all
of them when none) now rather than at the next poll.

`PROTOCOL_VERSION` is `"4"` (P10: `InvokeParams.idempotency_key`; 3:
`host/call`; 4: `host/changed`). A
provider's declarations carry the version, so a bump changes the declarations:
a provider is deployed again to speak it.

**Errors** (`errors.rs`): JSON-RPC's standard codes plus
`NotConfigured` (-32001), `Auth` (-32002, `data.credential` — the
credential the service refused, when the provider knows it),
`RateLimited` (-32003, `data.retry_after_ms`), `InvalidInput` (-32004, `data.field`) and
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

A scripted provider, lib + bin, that the host's tests and the conformance
kit drive over real stdio. `OXPLOW_FAKE_CAPABILITY` picks the capability it
implements; unset, it is a **snapshots** provider. The modes: `snapshots`
(the default), `filesystem`, `agent_harness` (`harness_declarations`) and
`knowledge` (going away with that capability: add no new uses). Every mode
has the same config, requiring `team`: `check` returns handle `fake:<team>`
or a `/team` problem. It takes its instance id from `OXPLOW_PROVIDER_ID`
(`fake` by default). The doc comment at the top of
`crates/oxplow-provider-fake/src/lib.rs` is the full list of modes and
hooks.

**Script hooks** — `OXPLOW_FAKE_HOOKS` at spawn, or a `fake/hooks
{ hooks }` notification mid-session, which **replaces** them (`""`
clears), keeping only what was declared at `initialize`
(`bad-declarations`) (comma-separated). They act in every mode:
`fail-next[:<n>]` (the next n check/invoke/read fail `Internal`),
`started-file:<path>` (each invoke writes `<path>` as it begins, before
`slow` — so a test knows a call is under way without timing it),
`slow:<ms>` (invoke and read wait first; `$/cancel` interrupts them with
`Cancelled`), `slow-check:<ms>` (check waits first), `crash` (the next request drops the connection; the binary
exits 3), `shutdown-file:<path>` (a `shutdown` writes `<path>` first —
proof of an orderly stop), `ignore-shutdown` (it answers `shutdown` and
keeps running), `bad-declarations` (`initialize` declares an extra command
named `undeclared`, for the host's handshake check), `rate-limit:<ms>` (its
next invoke or read is refused `RateLimited`), `needs:<NAME>` (`check` reports
`/credentials/<NAME>` unless that credential reached the process),
`accepts:<NAME>=<value>` (check, invoke and read answer `Auth` naming
`<NAME>` unless the credential the process holds is `<value>` — a
service that takes one token and no other, for the sign-in tests),
`refuse-auth` (invoke and read answer an `Auth` that names no
credential), `lax-check` (check doesn't look at credentials, so `accepts`
shows only on invoke and read — a service whose check doesn't validate the
token), `lose-reply` (the next invoke lands and is never answered),
`host-read` (an invoke first reads the host, `host/call` `sql.read`, and
answers what it read beside its result) and `config-read` (the same for the
extension's own configuration, `extension_config.read`). The hooks that act on a `read` need a
collector, which only the knowledge mode has: `progress` (a read sends
`$/progress` before each record, then takes 100 ms over it),
`read-fail-after:<n>` (a read fails after `n` checkpointed records),
`stuck-cursor` (every `$/state` is `{ cursor: 0 }`), `checkpoint-at-end`
(a read sends one `$/state`, after its last record) and
`hold-read-file:<path>` (a read waits until `<path>` exists, sending
`$/progress` meanwhile).

**Idempotency keys:** an `invoke` sent again with its `idempotency_key`
is done once and answered as the first was; the key sent with another
write is `InvalidInput` at `/idempotency_key`. It declares no
`idempotent_writes`.
**Its service outlives its process** when `OXPLOW_FAKE_STATE` names a
file: its state and answered keys are restored at start and kept
after every invoke, as a real service's are. Fixtures that launch it through an
extension (`providers/tests.rs` `write_extension`, the SDK's
`just_works.rs`, the desktop `extension_cli.rs`) set it to
`<project>/.oxplow/fake-state-$OXPLOW_PROVIDER_ID.json` — outside the
extension folder, whose contents are the deployment — and the kit tests
clear it before each run, since the golden transcript records what it
handed out. Without it the fake forgets on exit.
Two notifications drive it from a test: `fake/changed { collectors? }`
(it sends `host/changed` back, as a webhook would make it) and
`fake/exit` (it exits 3 on its own).
`tests/stdio.rs` pins all of it through a `Peer`, validating the streamed
notifications against the goldens.

**As a snapshots provider** (`OXPLOW_FAKE_CAPABILITY=snapshots`, with
`OXPLOW_FAKE_FEATURES=contents` for the feature of that name, which is
the only one; any other feature or a feature on another mode exits 2) it
declares `snapshots_declarations(contents)` — the `snapshots` capability
with `features: { contents }` and `data: null`, the verbs `mark` and
`changed` and, with `contents`, `read_at` (each `confirm: never`,
`access: record`, inputs `additionalProperties: false`), no event types or
collectors, the same `team` config. `mark` walks `worktree` (skipping
`.git` and `.oxplow`), hashes each file xxh3-128 (lower-case hex, as
core's `content_hash`), names the state `m<N>` and answers `{ handle,
unchanged, file_count }` (no `branch`/`revision`); `unchanged` is whether
the tree equals `parent`'s. `changed` diffs two marked states (`from:
null` is the empty tree, an unknown handle `InvalidInput`) and `read_at`
answers a file's bytes as base64. What it marked, bytes included, lives
in memory only; the `mark`s it answered are counted in its state
(`marks`, also what the next handle is numbered from), so a test can see a
provider that wasn't chosen was asked nothing.

**As a filesystem provider** (`OXPLOW_FAKE_CAPABILITY=filesystem`, with
`OXPLOW_FAKE_FEATURES=writable` for the feature of that name, the only
one) it declares `filesystem_declarations(writable)` — the `filesystem`
capability with `features: { writable }`, the verbs `stat`, `read`,
`list` and, with `writable`, `write` (each `confirm: never`, `access:
record`, inputs `additionalProperties: false`), no event types or
collectors, the same `team` config. It serves an in-memory tree, seeded at
start from `OXPLOW_FAKE_FILES` (a JSON object, path → base64 content) and
changed by `write`. It has one revision, `v1`: no `rev` or `v1` reads it,
any other is `InvalidInput`. Every file's `modified_at` is a fixed
`1700000000000`; folders are implied by file paths.

**As a knowledge provider** (`OXPLOW_FAKE_CAPABILITY=knowledge`) it
declares `knowledge_declarations()` — the `knowledge` capability with
`features: { edit: true }` and `data: { kinds: [note] }`, the verbs
`write_page { id?, title?, body, verified_files, removed_files }` →
`{ id }` and `delete_page { id }` (each `confirm: never`, `access:
record`, inputs `additionalProperties: false`), the event type
`knowledge.page.recorded@3`, a `knowledge_pages` collector over entity
`knowledge_page` (`discover` lists it), and the same `team` config; the
event's schema is core's. Pages live in its state
like items do. Every verb answers the page's record as that event:
`{ page: { uri: "oxplow:oxplow/knowledge?id=<id>&store=<store>", kind?, url?, author?, title, body, uris, updated_at, deleted } }`.
`uris` is the body's `[[…]]` interiors in oxplow's grammar (one with a `/`
or a `.` is `oxplow:oxplow/file?path=<path>`, `<store>:<id>` that store's
page, a bare one an oxwiki page); with no `id` it takes one from the title
(lowercased, other characters `-`); the title defaults to
the id; `verified_files` and `removed_files` are accepted and ignored (it
keeps no pins). There is no `link` verb: core adds the link and writes
the page through `write_page`. `delete_page` answers the record with `deleted: true`, and a page that is
absent or already deleted is `InvalidInput`. `read` of `knowledge_pages`
streams the pages changed after the cursor in revision order, each
followed by a `$/state` checkpoint `{ cursor, seen }`.

## The host (`crates/oxplow-app/src/providers/`)

**The manifest kind** (`spec.rs`): `providers:` is a stable kind (since the command model's
providers phase), loaded onto `Extension.providers` (a disabled extension
has none):

```yaml
providers:
  - id: fake                     # the URI segment and command namespace
    capability: snapshots        # one with a provider contract (below)
    entry: bin/provider          # a program in the extension folder
    args: [--stdio]
    env: [SERVICE_URL]           # host variables passed through by name
    credentials: [token]         # keychain values, as env; or
                                 # `{ name, oauth }` — "Credentials and sign-in"
    network: [api.example.com]   # hosts it may reach
    declarations: provider.json  # its InitializeResult, checked in
    needs: [sql.read]            # scopes it calls (`host/call`); read ones only
```

The loader refuses (into `errors`) an id that isn't lowercase
snake_case, is `oxplow` or a core namespace, or repeats
(`capability::instance_id_problem`); a capability with no provider
contract; `stages:`, `fields` or `id_pattern` on a capability that keeps
no items (none does today); an entry or declarations path
outside the folder (or the manifest, or under a retired UI folder); a bad host
pattern; and declarations that don't parse, speak another protocol
version, lack the named capability, declare a feature that isn't the
capability's (each `true` or `false`), declare a capability `data` that
isn't its contract's shape (`ProviderContract::data`, a JSON Schema: an
agent harness's environment markers and settings) or
any `data` when the contract asks for none, lack a verb its contract requires
for the features it declares, declare a verb that isn't `confirm: never`
and `access: record` (core's call of the verb
is what a person confirms and what is gated), or
declare an event type its contract doesn't let it emit.

## What a provider may implement

A capability a process may implement carries a **provider contract**
(`oxplow_domain::capability::ProviderContract`, on its `CapabilitySpec`).
The host enforces the contract, never a capability's name: the verbs it
requires (each always, or when a feature is declared), the command family
a verb runs as (so an inverse naming a verb undoes through it), the event
types it may emit, what its collectors stream, the URIs an instance owns,
and whether it keeps items.

| | `agent_harness` | `snapshots` | `knowledge` | `filesystem` |
|---|---|---|---|---|
| verbs | `launch`, `tool_use`, `render`; `turns` ← transcript, `token_readings` ← telemetry, `refresh_text` ← runtime_text, `prompt` and `subagent` ← subagents | `mark`, `changed`; `read_at` ← contents | `write_page`, `delete_page` | `stat`, `read`, `list`; `write` ← writable |
| verbs run as | core's call: a session's launch, the hook route, the OTLP route, a transcript read, a text refresh | core's call: every take core makes (a turn's end, a measured run, an archive) | `oxplow.knowledge.<verb>`, after core checks the page against its record | core's call: a reader's `stat`, `read` and `list` for a file URI naming the filesystem |
| declares as `data` | `env_markers`, `settings` | nothing | `kinds` (`{ id, title, bucket }`; none means `note`); feature `edit` | nothing |
| may emit | nothing | nothing | `knowledge.page.recorded@3` | nothing |
| collectors stream | nothing | nothing | `knowledge_page` rows, logged as `knowledge.page.recorded@3` `{ page: row }` | nothing |
| owns URIs | none | none | `oxplow/knowledge` (its `store` param names the store) | none (its id is the `filesystem` param of the file URIs it serves) |

There is no task tracker capability: a task is a row of the shared model
`oxplow/task_record`, filled by a store ([tasks.md](./tasks.md)). A
program that brings an outside tracker's data in is an ordinary extension
whose collector fills the model through `fills:`, not a provider with a
contract. No capability keeps items now, so a provider's `fields`,
`id_pattern` and `stages` are refused.

Each such capability has a **host** (`registry::CapabilityHost`): what
turns a started instance into the capability's implementation and takes
it out again. `Services` builds one per capability and hands them to the
`ProviderRegistry`: the agent
harness's (`agent_harness::HarnessHost`) registers an `ExternalHarness` in
`Services.harnesses` under the instance id, beside the built-ins (a
catalog reload keeps it); the snapshots' (`snapshots::SnapshotsHost`)
registers an `ExternalSnapshots` in `Services.snapshots`; the
knowledge's (`knowledge::KnowledgeHost`) registers an `ExternalKnowledge`
in `Services.knowledge`; the filesystem's
(`filesystems::FileSystemsHost`) registers an `ExternalFileSystem` in
`Services.filesystems` ([filesystems.md](./filesystems.md)). A record row carries its own `uri`; the host
logs it as the contract's event with the row under the contract's key,
that URI the subject and the instance the source (`provider:<id>`: whose
record it is). A URI of a resource with no `provider` param
(`oxplow/knowledge`) is no instance's, so any well-formed one is an
instance's own (`registry::owns`).

Each host's `prepare` validates declarations and constructs its adapter
without changing a live registry or starting a process. It returns a
`PreparedProviderCapability` with fixed features and an infallible,
synchronous registry action that the generation applies at publication. All
the capability hosts use this split; nothing publishes an adapter outside a
generation.

**A host adapts an endpoint, not a process** (`endpoint::Endpoint`):
what its adapter calls — the id and name, the declarations
(`InitializeResult`), `invoke`,
`envelope` (`endpoint::envelope_of`, one rule: a declared type within
the contract, its own records and subjects) and `restart`. An `Instance`
is one (its process); a **script implementation** an extension declares
under `implementations:` is another (`script::ScriptEndpoint`: its
functions, run in the command scripts' sandbox with its `needs`;
extensions-commands.md "Implementations"). So every capability with a
provider contract takes a script with no host change, and the script's
answers meet the same checks as a process's. Script adapters are
prepared and published with the generation's runtime adapters
(`ProviderRegistry::prepare_scripts`, `publish_scripts`, which retires
the previous generation's), not with the instances.

A candidate that is rejected before publication is marked stopped and shut down
directly; cleanup does not withdraw by name or touch an incumbent's
maps/database rows. Process supervision starts after publication.

**A knowledge provider** is one knowledge store (an Obsidian vault, a
Confluence or MediaWiki space); several stores run at once, with no
priority, and oxplow's own store is oxwiki. Core keeps the record (`.context/knowledge.md` "The
capability"). The page commands check a write against core's record
first — the id, every link it adds, the URIs it verifies or removes —
then invoke the verb: `write_page { id?, title?, body, verified_files,
removed_files }` (the body in oxplow's markdown), `delete_page { id }`.
A store with `edit` off is never written. Core does the link command
itself (it adds `- [[target]]` under `## Related` and writes the page
through `write_page`). The store declares its kinds as `data: { kinds }`.
Each answers the page's `knowledge.page.recorded@3 { page: { uri, kind?,
url?, author?, title, body, uris, updated_at, deleted } }`: `body` what it keeps (in its own
grammar if it likes), `uris` what the page links **in oxplow's grammar**
(`oxplow:oxplow/file?path=src/lib.rs`, `oxplow:oxplow/knowledge?id=other&store=oxwiki`) — core links and pins
from those, never parsing a foreign body. A write answering no record of
the page fails. Core records the row, the edges and the pins from it and
logs `knowledge.page.written` after the store's own record. Its collector
streams its pages edited outside oxplow as the same records;
`knowledge::KnowledgeRecords` restates them while the store is enabled
(a body the record already holds restates nothing; `deleted` takes the
page out).

**A snapshots provider** marks the worktree; core keeps the record
(`snapshot`, `file_snapshot`, `snapshot_op`, `snapshot.taken`), so
change entries, tree hashes, retention and every reader treat its
snapshots as core's own. A mark is `mark { stream, worktree, trigger,
thread?, turn?, budget_ms?, parent }` → `{ handle, unchanged,
file_count, branch?, revision? }`; then, unless it's unchanged from a
parent it named, `changed { stream, from, to }` → `{ changes: [{ path,
kind: added | modified | deleted, identity?, size? }] }`; then one
`record_take` with the handle on its op (V46). `parent` is the handle
on the stream's newest op: `null` when core's capture took it (the
first mark after a switch) or nothing has, and then the listing is its
whole tree, which the host reconciles with the record (unchanged files
left out, unlisted ones tombstoned). An identity is the xxh3-128 of the
bytes, 32 lower-case hex digits, oxplow's own; a file over
`snapshot_max_file_bytes` is `oversize`. With **`contents`** the host
pulls each new identity the blob store lacks (`read_at { handle, path }`
→ `{ bytes }`, base64), checks the bytes hash to it, and keeps them, so
every read of its snapshots is local; without it its rows are
identities with no bytes, as the hashes-only built-in's are, and a read
answers "no contents". Provider output is data: a path outside the
worktree, a file listed twice, a bad identity or bytes that don't hash
to theirs fail the mark, and nothing is recorded. The trait's `changed`
and `read_at` answer from the record. Marks are one at a time per
instance. While it's the active implementation, core's capture pipeline
idles (`.context/data-model.md` "An idle pipeline"), and the take on a
turn's Stop path is its `mark` under `snapshotTurnBudgetMs` (an overrun
is recorded, never cut short; the call itself is bounded by the
instance's call timeout).

**A filesystem provider** serves files (an S3 bucket, a remote
checkout, a document store) that readers name by the `filesystem` param of
a file URI (`oxplow:oxplow/file?filesystem=<id>&path=…`). Its id is
`<extension>:<instance id>` (`acme:s3`), registered in
`Services.filesystems` beside core's three, and the namespace `oxplow:` is
core's: an extension named `oxplow` is refused at `prepare`. The
capability is `many`, so every running instance is active at once, with no
choice to make. A filesystem is read-only unless the capability's
`features` say `writable`. Each operation is one invoke, with the
`rev` the reader gave (opaque to core; absent is the provider's default
revision):

- `stat { path, rev? }` → `{ entry }`, `entry` null when nothing is
  there, else `{ kind: file | dir, size?, modified_at?, is_text? }` (ms
  since the epoch). The editor's `language` is not the provider's to
  say: core derives it from the path.
- `read { path, rev?, offset?, len? }` → `{ bytes }`, base64, null when
  there is no such file. A range sends `offset` and `len`; the host
  truncates what comes back to `len`.
- `list { rev?, parent? }` → `{ entries: [{ path, kind }] }`; `parent`
  absent is the whole tree, `""` the root. The host sorts by path.
- `write { path, content }` → `{}`, `content` base64; declared only with
  `writable` (without it `write` is refused as `ReadOnly`, uncalled).

Paths are relative to the filesystem's root, `/`-separated. A path that
leaves it (`..`, `.`, an empty segment, absolute, `\`) is refused by the
host as `FsError::Invalid` before the call, so a provider never sees one.
Failure: a provider's `InvalidInput` (a revision it does not have) is
`FsError::Invalid`; every other failure — its process gone or
dropping the connection, a call timeout, a verb it did not declare, an
answer of the wrong shape, an event — is `FsError::Unavailable`, which a
reader shows and may try again (the registry restarts the process with
backoff, as for any instance). A filesystem provider emits no events.
Turning the instance off, or an update that drops the capability,
unregisters its filesystem: readers and `v_file` then refuse the id with
the registry's "no filesystem `acme:s3`" message. The fake serves it as
described in "The fake provider"; the conformance checks of
`filesystem_conformance` run through the host against it
(`providers/tests.rs`). `oxplow extension test` has no filesystem suite
yet: the checks need the fixture tree in the provider, and the kit has no
way to put it in a read-only one.

**A harness provider** answers what a built-in harness answers in code
(`.context/agent-model.md` "What a harness implements"). Its title is its
`provider.name`; its environment markers and settings are its
capability's `data`. Each operation is one invoke:
`launch` takes the launch's input as it is (`LaunchInput`: the session,
its worktree and project, the endpoints and the session's bearer, the
identity env, the system prompt, the resume id, the agent text, its
`agentConfig`, oxplow's executable, home, and the program `search_path`)
and answers a `Launch` (`{ spec: { kind: pty, command, env }, transcript_missing? }`);
`tool_use { body }` → `{ tool }`; `render { answer }` (`{ kind: ack |
deny | context, … }`) → `{ body }`; `refresh_text { roots, text }` → `{}`;
`turns { transcript }` → `{ turns }`; `prompt { body }` → `{ prompt: { kind:
person, text } | { kind: handback, subagent, text? } | { kind: peer_message,
from, socket?, text? } | { kind: background_job, call?, job?, status?, summary?, text } | null }` and `subagent {
body }` → `{ subagent }` (a `SubagentStart`/`SubagentStop`'s), both its
`subagents` feature (without it a prompt is a person's); `token_readings { records }` (one
export's) → `{ readings }`. An optional verb it doesn't declare answers
none, uncalled. `tool_use` and `render` are on the hook route's path:
each is bounded at 1 s (`HOOK_VERB_TIMEOUT`, inside the route's 5 s), and
a late or failed answer reads as no call and `{}`. Deploying one is
admitting what its launch answers: that command runs in the person's
terminal with their rights, outside the provider's own permissions, so the
deploy's `process` line says so. The live proof is
`crates/oxplow-control-plane/tests/observe_conformance.rs`
`a_provider_launched_session_is_recorded_canonically`: the fake provider
in harness mode (`relay`, deployed and enabled in the project, the
in-process fake not registered) launches the fake harness's scripted
session, whose hooks the route maps and answers through the provider's
`tool_use` / `render`, and the observe suite finds it recorded
canonically.

**The harness conformance suite** (`harness_conformance::suite(svc,
harness, config, search_path)`) checks any registered harness through its
own operations, built-ins and providers alike: its answers render as
objects, a refusal and added context read unlike an acknowledgement, an
empty hook body maps to no tool, and a launch in a scratch worktree (placeholder endpoints, a
marked bearer) keeps the bearer off the command line or program and args, and in a terminal launch's env. In-tree it runs over the
three built-ins and the fake provider;
it found Codex passing its telemetry bearer in `--config`. The kit's arm
runs it with config `{}` and the kit environment's search path (its PATH,
then the install dirs under its HOME — `kit_search_path`), so a harness
provider's program must be findable where a person's would be; `ran`
shows `agent_harness suite`.

**A provider runs under its extension's permissions** (`.context/extensions-permissions.md`
"Permissions"): its manifest declares `process` (an `entry`), the `network` hosts it reaches and the `secrets` its credentials
name — the loader refuses it otherwise, naming what to declare — and a person
approved them when they deployed the extension (a deploy that adds one asks
first; the deploy's consent check is a person's to approve). There is no per-program approval.

**What runs is its deployment** (`host::deployment`). A start reads the
provider's declarations from its extension's deployment
(`.oxplow/runtime/extensions/<name>/<hash>/`), which nothing changes after it
is made, and spawns the entry there; its working directory and
`OXPLOW_EXTENSION_DIR` are that folder. A changed source changes nothing that
runs until it is deployed, and a deployment changes what runs by
activation, which restarts the instances it touches. The host's env
passthrough is `PATH`, `HOME` and the spec's `env`, plus `LLVM_PROFILE_FILE`
(set only in a coverage run, to an absolute path) so an instrumented provider
doesn't write its `.profraw` into the deployment (`host::PASSED_ENV`).

**The spawn** (`host.rs`, `connect`) mirrors an `exec` source: a
scrubbed environment (PATH, HOME, the declared `env` names, the
credentials from the instance's keychain accounts
(`instance:<project>:<namespace>/<instance id>:<name>`),
`OXPLOW_EXTENSION_DIR`, `OXPLOW_PROVIDER_ID` = the **instance id**), the egress proxy and
`sandbox-exec` where the OS enforces `network`, stderr to the log, and
`kill_on_drop` as the last resort. **A process is ended the orderly
way** (`Connection::shutdown`): every stop — reconcile, a config or
deployment change, a disable, a sign-in renewal (`Instance::end`, outside
the `live` lock; the renewal is recorded first, so a call it cuts off
knows why), `check_as`, and the daemon's exit (`Services::shutdown` → `shutdown_all`, every
instance at once, alongside the terminals and LSP servers, within 2 s of
the supervisor's grace; `process::exit` runs no destructors, so without it
the processes were orphaned) — sends
`shutdown` and waits `HostDeps.shutdown_grace`
(`MachineEnv.provider_shutdown_grace`: 5 s in the app, 1 s in
`Services::in_memory`) for the process to exit, then kills it. Then **the handshake**: the live `initialize` must equal
the deployment's declarations (`HostError::DeclarationsChanged` names the
first difference), and `check` of the instance's config must return a
handle (`HostError::Unconfigured { problems }` otherwise). A request
from the provider is answered `MethodNotFound` but `host/call`, served by
the instance's `host::HostCalls` through the gate a command's handler
meets (`scope_calls::Calls`: its `needs`; `sql.read` through the SQL
gateway — `HostDeps::sql`, metric functions included — or `state.read`
over its own private namespace), each call counted
into the trace of the run whose key it
names (`ProviderRegistry::run_op` registers the run's
`Invocation::trace` under its key) — so the run's audit
row records it.The conformance kit has no project: it answers `host/call` like any
other request.

**Instances** (§10.3; project scope). An instance is
`<namespace>/<instance id>`, configured in `.oxplow/project.yaml`:

```yaml
extensionInstances:
  tracker/issues: { enabled: true, config: { team: ENG } }
  # A second account: its own id, saying which provider it is (P9.B1).
  tracker/issues_acme: { enabled: true, provider: issues, config: { team: ACME } }
```

**An instance id** follows the provider id's rule — one rule,
`oxplow_domain::tasks::provider_id_problem`: lowercase
letters, digits and underscores, starting with a letter, and none of
oxplow's own (`oxplow`, a core namespace such as `code` or `config`),
checked wherever an id is written (`extensionInstances`, `instances.yaml`,
a manifest's `providers`). An instance oxplow still won't run — its id is
a namespace or provider something else registered first, or it declares
an event type oxplow doesn't know with that schema — is `refused { reason
}` on its row, never "enabled" and silently off. A provider's **default
instance** has the provider's own id; any other instance names its
provider (`provider:`) — without it, an id that isn't a declared
provider's is `Missing { reason }`, and the reason says what to add.
One spelling everywhere, because the id is used as it stands:

| Keyed on the **instance id** | Keyed on the **provider (program)** |
|---|---|
| the URI's `provider` param (`oxplow:oxplow/knowledge?id=a&provider=wiki_acme`) and `check_subject` | the extension's deployment (`.oxplow/runtime/extensions/<ext>/<hash>/`) |
| an extension command's `instance` (`tracker.snapshot.mark { instance: issues_acme }`) | the extension's permissions (`process.run`, `network`, `secrets`) |
| the capability registry id, `v_capability_provider.provider`, the `activeProviders` value | the Extensions page's provider row, `declaration_impact` |
| `OXPLOW_PROVIDER_ID` (the provider is told which instance it is) | the kit's fixtures and transcripts (it tests the program, as its default instance) |
| `contribution_health.contribution`, `provider_collector_state.instance` (the name) | |
| the credential accounts (`instance:<project>:<namespace>/<id>:<name>`) | |

Permissions stay per extension: they cover the folder, the entry, args
and the *names* of its credentials; two instances differ only in config and
credential *values*, which they never covered, so a second instance asks
nothing. A deploy that changes the program restarts every running instance
of it, in the same activation.

`ProviderRegistry::resolve(instance)` is the one mapping from a name to
`{ ext, spec, id }` (`enable` / `check` are the default instance's;
`enable_instance` any). A person adds another with `add_instance`
(refused when the extension doesn't declare the provider, the id is
taken, or the id is another provider's own), removes one with
`remove_instance` (it stops, and everything of it goes: its
config entry; its credentials, named by the extension or, when the
extension no longer declares its provider, by the copy it last ran
(`host::last_ran_credentials`); its read checkpoints
(`provider_collector_state`, so one added again under the id reads from
the start); its `contribution_health` row; and any `activeProviders` choice
naming its id — except, for a project's replacement of a global
instance, what the global one now showing through has),
and sets a credential with `set_credential` (a declared name; the
instance restarts on it) — the commands `oxplow.provider.add_instance`,
`remove_instance` and `set_credential` (`credentials.write`, a `writeOnly` value: the
collector credential's unrecorded path). A
provider's credential is its instance's: collectors keep the
extension's `source:<project>:<ext>:<name>` accounts. An extension's
command declared over its provider's operation runs on the instance its
input names (`instance`, else its `uri`'s, else the default).

**Scope** (P9.B2). An instance is the **project's** (above: shared with
the team in `project.yaml`) or the **person's** — global, in this
machine's `instances.yaml` in the global config dir
(`oxplow_config::GlobalInstances`: `instances: { "<namespace>/<id>": { enabled,
config, provider? } }`, the same entries and the same validation):

- a global instance applies in **every project whose catalog has its
  extension enabled**; where the extension isn't, it isn't listed at all;
- a project's entry of the same name **replaces it there, whole**
  (`instances_config()` is the one merge: global, then project) — to turn
  it off or configure it differently in one project. A person makes one
  from the global row's **Off in this project** (`off_here`, command
  `oxplow.provider.turn_off_here`): the project's own entry,
  off, with the global one's config and provider; Disable or a config
  edit on a global row with no project entry changes it everywhere, and
  Remove on the project's entry brings the global one back here. **A project's entry
  is the project's, credentials included** (`scope_of`: `project`, with
  `overridden` saying it replaces a global one; the row reads "this
  project's, replacing yours"). The P9 plan had an override keep the
  global scope and credentials; that broke a team's committed entry the
  moment the person added a global instance of the same name in another
  project — it moved onto the global credentials (unconfigured, or the
  other project's token against this one's config), and back when the
  global one went. Now a global one appearing or going never
  moves a project entry onto other credentials, and removing a project's
  replacement removes its own credentials, never the global one's;
- its credentials are the person's, set once:
  `instance:global:<namespace>/<id>:<name>` (a project instance's are
  `instance:<project>:…`). Changing one — a pasted value, a sign-in, a
  sign-out — restarts it in **every** project, not only where it was
  changed: the keychain can't be watched, so
  `credential_changed` bumps the instance's count in
  `instance-credentials.yaml` beside `instances.yaml`
  (`oxplow_config::CredentialGenerations`: counts only, never a value,
  under the same kind of machine-wide lock), and every other oxplow, on
  its per-minute tick (`reconcile_if_global_changed`), restarts a global
  instance whose count moved since it last started it — so a rotated and
  revoked token never keeps running elsewhere on the old value;
- **the program is the project's**: a global instance runs *this* project's
  deployment of the extension, under the permissions deployed here, so in a
  project that hasn't deployed the extension it is not listed at all (above);
- it is written where it lives (`set_instance`: the project's entry if
  there is one, else the file; `add_instance(…, scope)`;
  `remove_instance` takes a project's replacement first, and the global
  one then shows through). The file is written only by the registry for a
  person (`Actor::Human`; anyone else is `Denied`) — no command reaches
  it, so no agent or page can;
- **every change is one read-modify-write of the instances as they are
  then**: `write_instances` takes an `edit` of them, under the
  registry's `instances_gate` (one change at a time in a process, never
  held across a check or a reconcile — `set_instance` checks first, then
  takes the gate, re-resolves and edits) and, for the file,
  `GlobalInstances::update` — under `instances.yaml.lock`, which every
  oxplow on the machine takes, it reads the file as it is, edits,
  validates and writes it atomically (temp file and rename). So Enables
  on two rows at once both stand, two oxplows never drop each other's
  entry, and a reader never sees half a file;
- another project's oxplow re-reads it by its modification time, on the
  per-minute sync tick (`reconcile_if_global_changed`); the one that
  wrote it reconciles at once. A file that doesn't load keeps what was
  last read, with a warning.

**Credentials and sign-in** (P9.B3; `spec.rs` `CredentialDecl`,
`oauth.rs`). A credential is a name (a value the person pastes) or one
they **sign in** for:

```yaml
credentials:
  - CLIENT_SECRET                  # pasted
  - name: TRACKER_TOKEN            # signed in for; the env var holds the access token
    oauth:
      authorize_url: https://tracker.example/oauth/authorize
      token_url: https://tracker.example/oauth/token
      client_id: oxplow
      scopes: [read, write]
      client_secret: CLIENT_SECRET # optional: the NAME of a pasted credential
      client_auth: basic           # how the secret is sent: basic (default) | post
      redirect_port: 8123          # optional: for a service that wants a fixed redirect
```

The loader refuses a name declared twice, an endpoint that isn't https
(plain http only on loopback), and a `client_secret` that doesn't name a
pasted credential of the same provider. Collectors keep the bare-name
form; sign-in is a provider's.

- **oxplow runs the flow, not the provider.** Authorization code with
  PKCE (S256, always) and a loopback redirect (RFC 8252). The core
  starts it and finishes it; **the desktop shell catches the redirect**
  (P10), so it lands where the person's browser is whether the core runs
  here or on a remote daemon, and the core never binds a socket for a
  sign-in (guard `the_core_never_binds_a_socket_for_a_sign_in`).
  `oauth::begin(decl, client_secret, redirect_port)` builds the page
  (`redirect_uri` `http://127.0.0.1:<redirect_port>/callback`) and a
  `PendingSignIn` holding the `state` and the PKCE verifier only the core
  holds; a provider that declares `redirect_port` is signed in for on
  that port only. `PendingSignIn::redirected` takes the path and query
  the shell caught: only `/callback` with this sign-in's `state` is its —
  anything else does nothing and the sign-in waits on — and
  `PendingSignIn::exchange` trades the code. It is hand-rolled (two form
  POSTs over reqwest), one mechanism for every provider. The listener is
  its own crate, `oxplow-oauth-redirect` (the shell's; no core crate uses
  it), and answers anything but `GET /callback` itself — with a
  plain-text page marked `nosniff` and `no-store` — reads each connection on its own
  task with a deadline (10 s) and a 16 KiB head cap, and never reads a
  body, so an idle or oversized connection never holds up the real
  redirect.
- **Token requests** (`token_request`): the client secret goes in an
  HTTP Basic header (`client_secret_basic`, each half form-encoded — RFC
  6749 §2.3.1; the default every server must accept) or, with
  `client_auth: post`, in the form; with Basic neither id nor secret is
  in the form. A token endpoint that answers with a redirect is an error
  — reqwest follows none, so the secret and the code go to the declared
  endpoint only. An answer whose `token_type` isn't `Bearer` is refused
  (a missing one reads as bearer), and `expires_in` is read as a number
  or a numeric string.
- **The token is one keychain secret** — JSON `{ access_token,
  refresh_token?, expires_at?, scope? }` under the credential's instance
  account. The provider's process is handed **the access token alone**,
  as the env var of the credential's name. A credential named as a
  `client_secret` is the host's to send with token requests and is
  **never** in the process's environment (nor the kit's).
- **Renewal** (`oauth::access_token`): before every start a token
  within 60 s of lapsing is renewed and kept; a `check` that answers
  `Auth` is tried once more on a renewed token (`Instance::start`); an
  `invoke` or `read` that answers `Auth` renews, ends the process and
  tries the call once more on a fresh one (`Instance::reauthorize` — a
  process started since the refused call is left alone, so two callers
  renew once; "since the call" counts from when the call had its process,
  so a read that started the process itself still renews). **Only the refused credential is renewed** (`Instance::renewable`): the one the `Auth` names, when the instance
  signs in for it; with none named, its credential only when the process
  is handed exactly one and it is signed in — with two or more (a pasted
  key beside a sign-in counts) it can't know which and renews
  none, so a good sign-in is never lapsed for another's refusal and the
  refusal is the
  call's failure. A named credential that is pasted, not signed in for,
  has nothing to renew. Another credential is never renewed, so a
  renewal its service refuses for good never lapses one it didn't name.
  None of that counts as a failure; an `Auth` after the renewal does,
  like any other. Renewals of one account are **serialized** in-process
  (`renewal_lock`), and **a refusal renews only the token it refused**: `oauth::access_token` takes the refused access token, and
  forces a renewal only while that token is still the one stored — a
  caller that waited, another project's instance on the same global
  account, or a Settings Check refused for a token already replaced gets
  the stored one (renewed only if it is lapsing), so a rotating refresh
  token is spent once. Each instance keeps the credentials it
  gave its process (`Live.given`), which is how it names the refused
  token. The write compares first: a sign-out during the renewal
  stays signed out, a token replaced meanwhile (a new sign-in, another
  process) is used rather than overwritten, and a refused renewal marks
  the token lapsed only if it is still the one that was refused. Calls
  refused together renew once: `Instance.renewed_at` records
  each credential's last renewal, `reauthorize` returns early when that
  credential was renewed since the refused call, and a call cut off because a renewal ended its
  process (peer closed) is retried like an `Auth` — in `invoke` and in
  the sync's `read`.
- **A renewal the service refuses for good** (`invalid_grant`, or no
  refresh token to renew with) rewrites the kept token as lapsed with no
  refresh token — never deleted — so the credential reads
  `sign_in_again`, and the instance is **unconfigured** at
  `/credentials/<NAME>` ("sign in again"): it stops and registers
  nothing, and that is not a failure of the program. Not signed in at
  all is the same state with "isn't signed in". A token endpoint that
  can't be reached is an ordinary failed start (backoff), and a token
  that's still good is used as it is.
- **A component page signs in through the bridge** (`signIn`, protocol 11): the host runs the flow below (`viz/hostSignIn.ts`, each step a command the component declares) and answers how it ended; the frame never touches the shell or the browser.
- **Signing in** is a person's: command `oxplow.provider.begin_sign_in { instance,
  name, redirect_port }` (`ProviderRegistry::begin_sign_in`, `credentials.write`)
  returns the page to open — **only for a provider as its deployment declares it**: the
  endpoints, client id and client-secret name are part of the deployment,
  and a sign-in sends the code, the PKCE verifier and the client secret
  to them, so an edited endpoint is refused until it is deployed
  (the row's Sign in is off meanwhile); the renderer opens it in **the person's own browser**
  (`tauri-bridge/systemBrowser.ts`, not the sandboxed external-URL
  window — their sessions live there and services refuse embedded
  webviews). The row runs it (`SignInRow`): the shell listens
  (`listen_for_oauth_redirect { port? }` → `{ id, port }`, shell-only — on
  the credential's `redirectPort`, else any free port), the core begins on
  that port, the browser opens the page, and each redirect the shell
  catches (`await_oauth_redirect { listener }`) goes to the core by the command
  `oxplow.provider.complete_sign_in { instance, name, redirect }`, the
  shell answering the browser with the core's verdict
  (`answer_oauth_redirect { listener, outcome }`). A listener is known by
  its **id**, never its port: a newer sign-in may listen on the
  same port, and a late answer or stop for an old one must not touch it. A
  replaced or left sign-in is stopped with `stop_oauth_redirect
  { listener }`, which tells a browser still waiting and resolves once the
  socket is closed — the row awaits it before listening again. `SignInCompletion` is
  `signed_in`, `failed { error }`, or `not_this_sign_in { reason }` —
  refused, nothing done, the wait goes on. A match ends the sign-in: the
  provider is re-checked against the declaration it began with
  (an endpoint edited meanwhile gets nothing) and with its credentials
  where they were (a global instance turned off here, or a project entry of
  its name arriving, moves them: `failed`, sign in again), the
  code is exchanged and the token stored — and then it answers, so the
  browser hears at once; `credential_changed` restarts the instance on it
  afterwards and the renderer hears `CredentialChanged { instance, name,
  error }` when it has — the keychain is no model, so this is one of the
  bus's UI-only signals. The finish runs on a task of its own: a
  caller that goes away mid-exchange (a dropped connection to a remote
  daemon) never leaves it half done. The row acts on the outcome it holds
  too — it stops waiting, and shows a failure the core didn't announce.
  A completed sign-in can't be completed again. Sign-ins are tracked per
  instance and credential (`sign_ins`, each with a sequence number and
  an expiry timer): one not finished within five minutes
  (`oauth::SIGN_IN_WAIT`) ends, and the renderer hears why; the shell's
  listener stops by then too, by itself (`RedirectListeners`, a deadline
  task per listener — whether or not anyone still waits on it: a redirect
  it holds is answered and the socket closed, so a declared port is free
  again). A stop returns once the socket is closed. A second sign-in for the same credential
  abandons the first (its redirect is no longer the sign-in's; the row
  stops its listener), and the first's news says it was replaced. Every
  sign-in has a **number** (`oxplow.provider.begin_sign_in` → `{ url, signIn }`,
  `SignInId`); `CredentialChanged { …, signIn }` names the sign-in it is
  about, so a row ignores news of another sign-in of the credential — its
  own replaced one, another window's. A row that is left, or
  whose browser never opened, cancels its sign-in (`oxplow.provider.cancel_sign_in
  { instance, name, sign_in }`): nothing of it — verifier, client
  secret — is kept the five minutes, and its news says it was cancelled. Each `(instance, credential)` has its own gate
  (`sign_in_gate`), held while its sign-in begins, finishes — across the
  code exchange — expires or is abandoned, so two clicks at once leave one
  under way, and removing an instance — a project entry that uncovers a
  global one too — takes its credentials' gates, waits for a finish in
  flight and abandons its sign-ins first, so nothing is kept for what's
  gone. A slow token endpoint holds up only its own sign-in. The row's Sign in is off while one starts, and off
  without the desktop app (a plain browser can't catch the redirect),
  with the reason shown.
  `oxplow.provider.set_credential` refuses a value for a signed-in credential;
  with no value it signs out.
- **The view**: `ProviderInstanceView.credentials` is
  `InstanceCredential { name, set, sign_in?, redirect_port? }`
  (`redirect_port`: the declared one, where the shell must listen), `sign_in` being
  `not_signed_in | signed_in { until? } | sign_in_again` (`until` only
  for a token that can't renew itself). The Integrations row shows a
  **Sign in** / **Sign in again** button and **Sign out** instead of a
  value box (`signInLine` in the component's `app.js`).
- **Limits.** Only the desktop app can sign in (a plain browser on a
  daemon can't catch the redirect). A declared `redirect_port` already in
  use on the person's machine fails the listen, with the reason on the
  row. No real OAuth service
  has been exercised: the tests run against **`oxplow-oauth-sim`**
  (`crates/oxplow-oauth-sim`, axum), a stand-in authorization server
  that holds each code to the client, redirect and PKCE challenge it was
  issued for (RFC 6749 §4.1.3), reads the client from a Basic header or
  the form, issues refresh tokens, and can expire, rotate, revoke, delay,
  redirect token requests and answer with another `token_type` or a
  string `expires_in`. It serves OAuth only (no `/mcp`); a stand-in service
  asks it which access tokens it issued and still holds live, so a
  signed-in bearer runs end to end (`OAuthSim::accepts(token)` is the check
  a stand-in service makes); `POST /sim/expire` lapses every
  access token and `POST /sim/revoke` revokes the grant (its access
  tokens stop working, its refresh is `invalid_grant`). Run it by hand
  with `cargo run -p oxplow-oauth-sim -- --http 127.0.0.1:8124` (it
  prints its urls). **Development only:** it is a dev-dependency of the
  crates that test sign-in and a binary, never a dependency of anything
  that ships. Guard `no_test_double_is_a_production_dependency` holds that for every test double — a workspace crate named
  `*-fake` or `*-sim` (`oxplow-oauth-sim`, `oxplow-ai-fake`,
  `oxplow-provider-fake`): no workspace crate reaches
  one through a normal or build edge of the resolved graph (`cargo
  metadata`), however its manifest spells the dependency.

The key is shared with the team; whether it *runs* is per machine (the
deployment, credentials, health). The config object is the provider's
`config_schema`'s; `check` validates it.

**The registry** (`registry.rs`, `Services.providers`) keeps the running
instances matching the config. Extension activation is the only thing that
starts or replaces them: `initialize` at boot, then a generation on the
extension catalog's change signal and on an `extensionInstances` /
`activeProviders` change (the `config.providers` reactor on `config.changed`).
`ProviderRegistry::reconcile()` is only the request for a generation. A
generation starts every enabled instance that isn't already running unchanged
and stops the rest. A running instance whose identity is the same in the new
generation — its program (the extension's deployment), its
config, scope, declarations and spec, a live connection, and a live owner
(`authority_current`: its extension is still enabled at that version; a
redeployed edit replaces it in the same
activation rather than rejecting the generation at `validate_live`) — is carried over
with its process; only a changed instance is replaced, and a removed or turned-off
one stopped (the extension's place among the others doesn't count). An instance that can't run never rejects the
generation: one that is unconfigured, refused (its id is another
owner's), missing, or disabled is left out with that state on its row, and one
whose start failed or hung is published failing, restarting with backoff and
counted by `contribution_health` — every other extension and instance
activates. A started instance is handed to its capability's host at
publication — for a knowledge store, `ExternalKnowledge` in
`Services.knowledge` (each host's adapter, "What a provider may
implement") — and its **other** declared commands on the bus as
`<id>.<name>` (`External`, `Experimental`; confirm / access / undoable as declared). Its capability's verbs are never
commands of their own: core's call of the verb is the one write surface. A refusal —
unconfigured, a handshake that doesn't match — registers
nothing; a start that merely failed (it may come up) registers and
counts as a failure, and its next call restarts it after a backoff that
doubles from `MachineEnv.provider_backoff` (1 s in the app, 0 in
`Services::in_memory`) up to 60 s. `stop(instance)` removes both and
kills the process. **Its commands are its extension's**: the manifest
declares each once over an operation its declarations list
(`commands:` → `{ name: snapshot.mark, provider: fake, op: mark,
ui, … }`, [extensions-commands.md](./extensions-commands.md) "Commands"), so it has one id
for every instance (`tracker.snapshot.mark`), registered by the
extension's generation whether or not an instance runs. Its spec takes the
operation's input schema (plus an optional `instance`), effect and
undo; its `confirm` is the manifest's when that asks more, else the
declaration's; its `op` is `{ scope: provider:<namespace>/<provider>, op }`.
A run goes through the bus's **provider router**
(`CommandBus::set_provider_router`, `ProviderRouter for
ProviderRegistry::run_op`) to the instance its input names — an
`instance` field, else the instance its `uri` is of
(a URI's `provider` param), else the provider's default instance —
`Unavailable` when that one isn't running; its inverse names an
operation, made a call of the command declared over it (with the
instance), or `<family>.<verb>` for a capability verb (its contract's
`dispatch`, when it has one). A capability verb can't be
declared this way: core calls it. An instance registers only its
capability's implementation (no bus namespace). **A provider emits only
its capability's event types** (its contract's `events`: `knowledge` →
`knowledge.page.recorded@3` — a published version's schema never changes,
since declarations are compared to it exactly): declaring any other type — another core one such as
`contribution.enabled`, which would clear another contribution's disable — is
refused when the manifest loads, and the declared schema must equal
core's (checked at enable). Its own types are P7. A command's run
invokes the process and hands the bus its result, its inverse (as
`<id>.<command>`) and its events —
refused if a type isn't declared, a record names another provider's row,
or a subject isn't one of its
own URIs (`check_subject`: its contract's resource with `provider=<id>`, or
the extension's URI `oxplow:oxplow/extension?name=<ext>`).

**Calls are bounded**: `check` and `invoke` time out after
`HostDeps.call_timeout` (`MachineEnv.provider_call_timeout`: 60 s in the
app, 2 s in `Services::in_memory`); a timeout sends `$/cancel` and
counts as a failure. A restart runs under its own `starting` lock,
never holding `live`, so a start that hangs can't block `stop` (and
through it reconcile, `oxplow.contribution.enable` or `set_instance`). `Peer::start`
refuses once the other side's stream has closed, instead of leaving a
waiter that nothing resolves.

**A process is supervised** (`Instance::supervise`, for each started
process): one that exits while it is its instance's process — not one
the host ended, which leaves `live` first — is noticed at once, not at
the next call. The exit counts toward health (`its process exited
(status N)`) unless a call it cut off already counted it
(`Peer::calls_cut_off`), and the instance starts again once its backoff
is up — so a process that keeps dying backs off and is disabled like one
whose calls keep failing.

**A provider can say something changed** (`host/changed`):
`Instance::announce` reads the collectors it names after
`CHANGED_DEBOUNCE` (1 s), through `oxplow.provider.sync` as the system
like the schedule (`sync_announced`, audited); what is announced in the
meantime joins that read.

**Rate limits** (P7.A4). A `RateLimited` reply (`data.retry_after_ms`)
never counts toward disable, and never resets the count either. A wait of
at most `RATE_LIMIT_WAIT_MAX` (10 s) is slept through and the call
retried once — a read retries from its last checkpoint; a longer one, a
second, or one with no `retry_after_ms` fails the call honestly ("is
rate limited (…); try again in Ns"). Either way the instance's
`rate_limited_until` is set (cleared by the next success), and the
schedule skips the instance until then. **Progress**: a read's
`$/progress` is the instance's `activity` (`issues: page 2 (40%)`) while
it runs, cleared when it ends. Settings → Integrations appends both to
the instance's status (neither is a problem colour). The fake's
`rate-limit:<ms>` hook refuses its next `invoke` or `read` that way.

**A call's outcome is its instance's while that instance runs**. Every health write a running instance causes takes the
`Instance` that caused it and does nothing unless it is still the one
running under its name (`is_running`, by identity) — checked and written
under the `running` lock a stop takes: `call_succeeded`, `call_failed`
and `failed`, a (re)start's `start_failed` (its stop is
`take_if_current`, never by name), `disable` (the persistent disable
too), `note_rate_limit` and `set_activity`. A call that finishes after
its instance was stopped — the first read an enable starts, cut short or
just late — neither marks it `ready` again nor counts a failure against
it, and a restarted instance never inherits the old one's results.
**A stopped instance never starts again**: `tear_down` marks it
`stopped` before it ends its process, and `connection()` refuses to
start one that is (and ends a process whose start finished after the
stop). So a `oxplow.provider.sync` still holding the old instance when a person
turned it off and on, or when a deploy restarted it, can't bring a
process back — nor report its restart's `unconfigured`,
changed declarations or failure against the successor. An enable's first
start, before the instance runs, reports by name (`made_by: None`).

**Health** (`InstanceHealth { state, consecutive_failures, last_ok_at,
mean_invoke_ms, rate_limited_until, activity }`, per machine, in memory): `state` is `off`,
`missing { reason }` (configured, but it names no provider an enabled
extension declares; the reason says what to add), `refused { reason }`
(enabled, but it can't run as configured — above), `unconfigured { problems }`, `checking`, `ready`,
`failing { errors }` (the last five) or `disabled { reason }`. A failed
start or call counts (a refused input or a cancel doesn't); a success
resets the count and updates `last_ok_at` and the moving-average
`mean_invoke_ms`. The count and the disable are the policy every extension
contribution shares (P7.C1, `contribution_health.rs`; [extensions-activation.md](./extensions-activation.md)):
the `contribution_health` row keyed `<extension>` / `<instance id>`, kind
`provider` (`v_contribution_health`). **Three failures in a row disable the
instance** — a write counts once however often it is sent again under
its idempotency key (an effect's automatic retries, a person's retry of
the same composition: `Instance::failed_keys`), so one outage
that a reaction retries through doesn't halt it: it stops, and the row (`disabled`, its reason) and
`contribution.disabled@1 { contribution, reason }` (`contribution` the provider's URI) commit
together (source `system:extensions`, subject `extension:<extension>`). So is
a handshake that doesn't match the deployed declarations. The row is
what keeps it off, across generations and restarts, and one whose row
can't be read stays off too (`failing`, naming the error — not knowing
isn't a yes). `InstanceHealth` is the process's state, showing the
row's count. **A disable wins over a start in
flight**: each disable bumps the instance's epoch under the
`running` lock, and a generation publishes a started instance only if the
epoch it began with still holds, so a concurrent generation can't bring back
what was just disabled.
It is turned back on by **`oxplow.contribution.enable { extension, kind,
contribution }`** (`External`, not undoable; it replaced
`provider.enable`), which marks the row `ok`, logs `contribution.enabled@1`,
resets the backoff and asks for a generation. Settings → Integrations' Enable runs
it before writing `extensionInstances`.

**Reading: collectors and sync** (`sync.rs`, P7.A3). `Instance::read`
runs one declared collector's `read` — **one at a time per collector**
(a second sync waits, then resumes after the first) — from the
checkpoint it last stored
(`provider_collector_state`, [data-model.md](./data-model.md)), through
`start_streaming`. Each `$/record` must be the collector's entity — the
one its contract streams — and a row of its own (its `uri`), valid as
the contract's event; it is kept as that envelope (`knowledge.page.recorded@3`
`{ page }`, the actor's source) until the
next `$/state`, which commits the batch **and** the checkpoint in one
transaction, so a read that fails midway keeps exactly what its last
checkpoint covered and the next read resumes there. A record equal to its
row's last recorded event (a write's or an earlier read's, looked up by
the contract key's literal path) is **not logged**: it restates nothing,
and logging it would echo a write back to whatever reacted to it — an
effect that writes another provider's row would hear its own write again
on every sync. The checkpoint's `records` still counts what was read. The
result's
`records` must equal what was streamed; records after the last
checkpoint of a read that succeeded land too. A read holds at most
`HostDeps.max_uncheckpointed` records between checkpoints
(`MachineEnv.provider_max_uncheckpointed`: 10,000; 100 in-memory): one that
streams more without a `$/state` fails. A record that breaks a
rule, a count that doesn't match, or a read that sends nothing for
`call_timeout` (`$/cancel` follows) fails the read and counts toward the
instance's health like a failed call (a refused input or a cancel
doesn't).

**`oxplow.provider.sync { instance, collector? }`** is the one way to read
([commands.md](./commands.md)): Settings → Integrations' **Sync Now**, an
agent, the schedule and the start. **The schedule** (`sync_due`, every
minute from `spawn_sync_scheduler`) reads each running instance's
collectors whose last read is older than its `syncMinutes`
(`extensionInstances.<instance>.syncMinutes`, default 5; `0` means only
on request), as `Actor::System` through the bus, so each read is
audited. **A start** reads every collector once (`sync_started`) so the
items are there before the first scheduled read — spawned, not awaited:
a large first read must not hold up Settings' Enable or every other
reconcile. An external write whose
reply was lost (it landed at the tracker but the call timed out) is
restated by the next read.

**Settings → Integrations** is oxplow-foundation's own component
(`extensions/oxplow-foundation/components/integrations/`: `index.html`,
`app.js`, `app.css`, hand-written, no build), mounted by the page
`integrations` in `oxplow/settings-integrations.sections` after Capabilities and
Replaced components. Its operations are commands
(`commands/provider_instances.rs`, scopes `providers.write` and
`credentials.write`, all `External`) and the instances are read as the live
models `v_provider_instance` (every declared provider and configured
instance, with its state, `detail`, config and config schema),
`v_provider_credential` (each credential's presence and sign-in state,
never a value) and `v_provider_collector` (each collector's reads), pushed
down on `instance` and following `ConfigChanged`, `ExtensionsChanged`,
`CredentialChanged` and the models `v_contribution_health` /
`v_collector_state`. The page's query names all three (credentials and
collectors as aggregate columns), so a change to any of them changes the
rows and the frame re-reads (`onUpdate`; a page re-run posts `update` only
when its rows differ). The component declares the `oxplow.provider.*`
commands it runs and `sql: read`; the frame is as tall as its content
(`fit`: a document section has no height to fill). Each row shows the
instance's state, its collectors, its credentials, and: **Configure…**
opens the config as oxplow's form (`openForm({ schema: configSchema,
values })`, answering the values; the edit is the component's until it is
saved), **Check** (`oxplow.provider.check_instance { instance, config }`:
start it with that config and `check`, enabling and saving nothing — the
outcome is the view's state), **Enable / Disable / Enable again**
(`oxplow.provider.set_instance { instance, enabled, config }`:
`ProviderRegistry::set_instance`: enabling checks first and refuses an
unconfigured instance, writing nothing, with the problem's field as
`/config/<path>`; then `oxplow.config.set` of `extensionInstances` — to
enable, `oxplow.contribution.enable` runs **first**, so a failed enable
writes nothing and the config never says enabled for an instance that
wasn't — then a reconcile), **Save changes** for an enabled instance's
edited config, and **Sync Now** (`oxplow.provider.sync`) on a collector
while the instance runs. A pasted credential is **Set… / Replace…**
(a masked `writeOnly` field of an `openForm`; `oxplow.provider.set_credential`)
and **Forget**; a signed-in one is **Sign in** (`signIn`, above) and **Sign
out**. **Instances** (P9.B6): under the rows, "Add another instance of
`<namespace>/<provider>`" opens a form for a name (the instance id: lowercase
snake_case, not already an instance) and a scope ("project" / "global")
and runs `oxplow.provider.add_instance`. A named instance, a project's
replacement of the person's own, and one whose provider is gone have
**Remove** (`oxplow.provider.remove_instance`, `confirm: destructive`, so
oxplow asks the person) — a global one whose provider is gone included: a
global instance is left out of the list only where the project doesn't have
its extension enabled; a provider's own instance is turned off, not
removed. The core is covered by `providers/tests.rs`
(`provider_instances_are_managed_by_commands_and_read_as_a_model`), and the
whole in a browser by the suite's `integrations/fake-provider` spec
(`tests-e2e/`, P11): a person configures the fake's Team in the form, presses Check and
Enable, and presses Sync Now. Admitting the
program is the extension's deploy (the Extensions page); a deploy restarts
its running instances on the new declarations in the same activation, so
updated declarations take effect instead of disabling an instance at its
next start as changed.

**Tests** (`providers/tests.rs`) run the real fake binary (built beside
the test binary by the workspace build) through a script entry in a
temp extension: a provider whose extension doesn't declare `process.run` is refused
at load and registers nothing; an edited declarations file is refused;
the `bad-declarations` hook is refused naming `/commands` and disables
the instance; an unconfigured instance can't be enabled (nothing
written) and a configured one enables, writes `extensionInstances` and
disables again; `fail-next:3` disables it after three failures with the
reason logged, keeps it off across a reconcile, refuses an agent's
`oxplow.contribution.enable` and comes back on a person's; and a command
an extension declares over the fake's operation (`tracker.snapshot.mark`) is
on the bus, run on the instance its `uri` or `instance` names, with one
audit row, and its input is checked against its declared schema.
**Its commands appear in core menus** by their own `ui`: one about a
URI's kind (`ui.about: <kind>`) is on that URI's page menu and its
rows' right-click menus (`refOffers`).

**Its features are published** while it runs: publication writes the
instance in the capability registry and the generation's withdrawal takes it
out, each restating `v_capability_provider`, so the UI offers only what the
provider declares. Each generation restates
the rows, so a choice takes effect with the config change.

## Idempotency

A write to a provider may land without oxplow learning it did (a crash, a
timeout, a lost reply). Sending it again is safe only if the provider can
tell it is the same write. The contract (P10, built):

- **`features.idempotent_writes: true`** in a provider's declaration
  promises that two `invoke`s carrying the same key perform the write once
  and answer alike (a key sent with another write is `InvalidInput` at
  `/idempotency_key`; protocol 2). No capability's provider contract
  offers the feature today (the task tracker's did, and is gone), so no
  provider can declare it.
- **`InvokeParams.idempotency_key`** goes with every write: the caller's
  (a step of an effect's reaction: `effect_step_key`,
  `effect:<effect>:<event id>:<index>:<hash of the call>`, the same on
  every attempt — [commands.md](./commands.md)) or one
  `Instance::invoke` mints, the same key on each of its re-sends.
- **The host re-sends** a call refused (`Auth`, `RateLimited`: it never
  landed) as before; one cut off under way because a renewal ended its
  process may have landed, so it is sent again **only to a provider
  declaring `idempotent_writes`** — to any other the cut-off is the
  call's failure.
- **An effect's failed attempt is sent again by itself** only when every
  step it composed was a write to such a provider, at most twice, and it
  sends exactly what the failed attempt composed, so its keys are the
  same ([extensions-events.md](./extensions-events.md) "Attempts"); anything else waits for
  a person's `oxplow.effect.retry`, asked first. A failed command a person ran
  is reported, not retried.

The fake answers a re-sent key as it answered the first (its
`lose-reply` hook makes the case); a real provider would declare the
feature only once its service is known to do a keyed write once.

## The conformance kit (`crates/oxplow-sdk/src/conformance.rs`, `extension_test/`)

(The agent harnesses have their own suite over what core records of a
session: [agent-model.md](./agent-model.md) "Observe conformance".)

What `oxplow extension test <name> [--bless] [--json]` runs for each
provider an extension declares, after its check and its page and
collector examples ([extensions-sdk.md](./extensions-sdk.md) "The SDK"). The person running it runs their own program, so no deploy
approval is asked; credentials come from the environment (the declared
names — for a signed-in credential, an access token the author got
themselves: the kit never signs in — and never a client secret).
The report's **`left`** lists what the run made in a provider's own
system and didn't remove (`conformance::SuiteRun.left`: everything, for a
provider without a delete); the text form prints each as `left in the
provider:`.
Against a real service that is what to clean up.
`test_extension` reads this process's; `test_extension_in` takes a
`HostEnv` instead — for the provider's credentials, its declared `env`
and the throwaway host the conformance suite runs in
(`Services::in_memory_on_machine` takes the machine's environment, as
`MachineEnv.host_env`) — so tests running at once never share one.

- **`ReferenceClient`** spawns the provider exactly as the host does
  (`host::spawn`: scrubbed env, sandbox, kill on drop) and taps both
  pipes: every line is recorded and validated against the schema goldens
  (`schemas::for_message` by method; a reply is checked against the
  method its request named; an error reply's `error` against the
  `error` golden, which like every wire type allows no unknown field).
  A provider-sent notification or request outside the protocol, a
  host-only notification (`$/cancel`) from the provider, a reply to an
  id the other side isn't waiting on, or a line that isn't JSON-RPC, is
  a violation.
  `finish()` sends `shutdown`, waits for the process and returns the
  session.
- **The session**: `initialize` (must equal the declarations file,
  naming the first difference), `check` with `fixtures/provider-<id>.yaml`'s
  `config` (each problem is a finding at that file), then each
  `intent.examples[*]` whose `fixtures/<name>.yaml` is `{ input: {
  command, input }, expect }`: `invoke`, and the result must match
  `expect` (`first_mismatch`; `"$any"` matches anything).
- **The golden transcript** `fixtures/transcripts/<id>.jsonl`: one
  `{ from, message }` per line, `normalize`d — ids renumbered from 1 per
  sender in order of its requests (replies and `$/…` notifications
  follow their request), the host's version `$any`. Compared line by
  line (a mismatch names the file, the line and the JSON pointer;
  `$any` in the golden matches anything, so an author can loosen a
  volatile value by hand); `--bless` writes it. A missing golden is a
  finding.
- **What it reads** (P7.A7, `read_back`): when the provider declares
  collectors, `discover` must list each one's entity, and each
  collector's `read` — after the examples wrote something — must stream
  records of its entity, exactly as many as its `ReadResult` says,
  ending with a `$/state` checkpoint; a second `read` from that
  checkpoint must not stream them all again (**a cursor that doesn't
  advance** fails; the fake's `stuck-cursor` hook is the red). It shows
  in `ran` as `discover` and `read <collector>`; the messages join the
  golden transcript.
- **The capability suite** runs through a throwaway host: a temp
  project (`GitProvider::init_repository`: an empty root commit) with a
  copy of the extension, `Services::in_memory` over it, the provider
  enabled there with the fixture config, then its
  capability's suite: when the instance made the project's snapshots, the captures following it,
  `snapshot_conformance::suite` on the primary stream (round-trip, an
  unchanged mark keeps its snapshot, diffs with deletions, turn-end
  anchoring, the budget recorded), which both built-ins pass too; or —
  the instance enabled as a knowledge store —
  `knowledge_conformance::suite` through the page commands (a write lands
  the page, its body and its event; pins go stale and re-pin; a dangling
  link is refused; a link lands; a delete takes the page), which oxwiki
  passes too.
- **A script implementation's suite** (`suites::script_suite`): for each
  `implementations:` entry whose entry is a script, the same capability
  suite (`capability_suite`, shared with the providers') on a fork of the
  throwaway with the implementation chosen (a choosable capability's
  `activeProviders`); its case is `implementation <id>`.

The red test (`extension_cli.rs`,
`extension_test_blesses_a_provider_then_a_changed_transcript_fails`)
scaffolds `extension new provider fake`, sees the stub fail the handshake,
puts the fake behind the entry, sees the unconfigured fixture and the
missing golden fail, blesses, passes, then edits the golden and gets
one error at the transcript's line and pointer. The fake binary exits
as soon as serving ends (its runtime would otherwise wait on the
blocked stdin reader).

## External execution authority

Providers run with the person's file, network
and process authority; the deploy's `process` line says so and oxplow doesn't confine them
(there is no execution tier, and confinement is shelved until real OS
confinement exists). Network declarations plus the macOS egress proxy limit
direct network paths but do not promise confinement against local brokers or
other code.

The host launches the entry from the immutable deployment. `host/call`
rechecks the live-owner gate and applies model/path service needs to
the registered run. A connection monitor checks the live owner every 100 ms and
kills its owned process group when the extension is disabled or changed. Shutdown, natural exit
and dropped connections clean up ordinary children and disarm group tracking;
processes deliberately escaping the group keep the person's authority. Changing a working-tree script does not change the frozen
provider that is already running.

Copy preparation uses the same bounded regular-file copier as collectors:
500 files and 16 MiB, including metadata. It rejects symlinks and special
files, preserves empty helper directories and normal permission bits, and
strips set-id bits. The copied extension root is owner-only on Unix. A file
replaced with a FIFO cannot block copy preparation before launch.

## Private state host calls

`host/call` with `scope: state.read` and `{scope: project|personal,
area: state|cache}` returns null or a private `{stamp, values}` snapshot. The
provider must declare that need, name an in-flight invoke key, stay a live
owner and match the current generation/version/enabled descriptor. The owner
is captured from an exact active catalog declaration when the instance is
built, and requires the same generation before and after the deployment is resolved; a command in another extension cannot redirect it to that extension's
namespace, and a candidate check cannot borrow the installed owner. No request
selects an extension or DB path. Each call owns a short read transaction;
provider execution and waiting hold none. Late catalog changes conservatively
refuse the old instance's state access until lifetime reconciliation. See
[extensions-manifest.md](./extensions-manifest.md) "Private state host boundary".

## Provider resolution snapshots

Resolution captures effective instance config and credential scope together:
global entries first, replaced whole by project entries of the same name.
Provider choice and scope use that captured mapping, so an override removed
mid-resolution cannot pair project config with global credentials. A reconcile
pass uses one such mapping and one extension declaration snapshot for every
instance; async stops/starts do not make later instance resolution reread a
newer config/catalog. Existing live-owner checks
still apply before execution.

Bundled declaration ownership wins before enabled filtering. A disabled bundled
owner cannot be replaced by an enabled project namesake. This is a resolution
consistency prerequisite for generation staging.

## Staged provider generations

`ProviderRegistry::prepare_generation` stages every configured enabled,
non-disabled provider against one immutable catalog snapshot. It resolves the
effective project/global configuration and credential scope, rejects duplicate
instance IDs, prepares capability adapters, starts each process, and completes
its private `check` before returning a candidate. Preparation performs no
registry admission, health publication or supervision. Candidate diagnostics
are fixed and value-free; failed preparation shuts down already-started
candidates, with owned process-group cleanup as the cancellation/drop fallback.

The candidate owns the provider reconcile gate. Before publication,
`reserve_running` takes the running-map writer before the database write
permit. This ordering is required because a stop or failure can hold that map
while awaiting a database write. The reservation verifies that the incumbent
map still matches the captured base and rechecks effective configuration,
scope, global-file mtime, catalog generation, disable epoch, candidate peer
liveness and the current deployment. Candidate event declarations must
also match the vocabulary prepared for the same generation. Candidate
`check`-time `host/changed` notifications are ignored until the instance is
admitted.

`ProviderGenerationReservation::apply` runs only after the candidate database
transaction commits. It withdraws incumbent adapters, stops and removes old
instances, publishes candidate adapters and running instances, marks them
admitted, then starts supervision. Old processes are shut down orderly outside
the transaction; dropping the retired connection is the process-group kill
fallback. Thus a preparation or transaction rejection retains the incumbents.
`ExtensionActivation::activate` can coordinate these provider candidates with
native capability, command, model, vocabulary and adapter candidates. This
coordinator is the boot activation path. It first publishes a required-bundled
baseline, then attempts strict full activation; failure of the required baseline
prevents serving, while full-candidate failure retains the baseline or last good
generation. Provider stop/revocation takes effect immediately and retains the
running-map state needed for orderly cleanup; replacement is queued for a later
generation attempt, and process shutdown stays outside the SQL transaction.
Runtime catalog reads use the committed snapshot plus immediate explicit-disable
and missing-folder revocation, without loading changed declarations. Some
native/background and synchronous metadata entrypoints still need reader-pin
review, and metric catalog reseeding remains separate.

## The read adapter into the record models

A provider instance's `knowledge.page.recorded` records are also written
into bundled's `oxplow/knowledge_record` model under the instance's id
(`providers/model_records.rs`, consumer `records.models`; it is
knowledge-only), beside the `knowledge.records` projection, which goes in a
later phase. A task is not read this way: a task store fills
`oxplow/task_record` through `fills:` ([tasks.md](./tasks.md)). See
`.context/models.md` "Providers". A provider program changes nothing.
