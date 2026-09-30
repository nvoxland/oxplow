# External providers

A provider is a program oxplow talks to — an issue tracker's bridge, a
docs system's — that implements one of oxplow's capabilities (work items
first) outside the app (P5.D, `target-architecture.md` §10). This doc
covers the protocol (D1), the fake provider (D2) and the host with its
consent and spawn rules (D3), instances with health (D4) and the
conformance kit with `oxplow plugin test` (D5).

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
`transition` / `link` / `comment` (effect `record`, no confirmation —
a verb declared `confirm: always` is refused, since the `work_item.*`
command calling it is what a person confirms; tsk569), the
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
`Cancelled`), `slow-check:<ms>` (check waits first), `crash` (the next request drops the connection; the binary
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
extension folder but the manifest and `lenses/` — dot-files, the entry
and the declarations file among them; a symlink anywhere makes it
unapprovable — plus the entry path, `args` (hashed relative to the
folder, where it runs; an arg path leaving the folder is refused at
load), `env` names, `credentials` and `network`. So a changed declaration is a new version,
shown unapproved in Settings → Data → Programs (with its grants listed)
until a person approves it again. Every start re-checks it, restarts
included.

**What runs is a verified copy** (`host::approved_copy`, tsk547). Each
start copies the extension folder into `<oxplow home>/provider-copies/
<project>/<ext>/<id>/<hash>/` (in-memory services: under the state
dir), hashes the copy, and runs only if that hash is approved — then
spawns the entry from the copy and reads the declarations from it. So
what runs is exactly what was approved: a checkout or an edit to the
live tree between the check and the exec, or under a provider that
loads its modules lazily, can't slip in. An existing copy is re-hashed
before each run (a process running as the person can still write
there) and replaced if it changed; older copies go. The provider's
working directory and `OXPLOW_EXTENSION_DIR` are the copy.

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

**Instances** (§10.3, minimal: project scope only). An instance is
`<extension>/<provider id>`, configured in `.oxplow/project.yaml`:

```yaml
extensionInstances:
  tracker/linear: { enabled: true, config: { team: ENG } }
```

The key is human-only (`HUMAN_ONLY_KEYS`: enabling runs a program) and
shared with the team; whether it *runs* is per machine (approval,
credentials, health). The config object is the provider's
`config_schema`'s; `check` validates it.

**The registry** (`registry.rs`, `Services.providers`) keeps the running
instances matching the config: `reconcile()` runs at boot and on every
`ConfigChanged` (`spawn_reconciler`), starting enabled instances and
stopping the rest (a config or spec change restarts one). `enable(ext,
spec, config)` starts an instance and only then registers its declared
commands on the bus as `<id>.<name>` (`External`, `Experimental`, all
invokers; confirm / effect / undoable as declared) and its capability
provider (`ExternalWorkItems`) in `Services.work_items`. A refusal —
unapproved, unconfigured, a handshake that doesn't match — registers
nothing; a start that merely failed (it may come up) registers and
counts as a failure, and its next call restarts it after a backoff that
doubles from `MachineEnv.provider_backoff` (1 s in the app, 0 in
`Services::in_memory`) up to 60 s. `stop(instance)` removes both and
kills the process. An id already used as a command namespace or
provider is refused. **A provider emits only its capability's event
types** (`spec::allowed_event_types`: `work_items` → `work_item.recorded@1`;
tsk548): declaring any other type — another core one such as
`provider.enabled`, which would clear another instance's disable — is
refused when the manifest loads, and the declared schema must equal
core's (checked at enable). Its own types are P7. A command's run
invokes the process and hands the bus its result, its inverse (as
`<id>.<command>`) and its events — refused if a type isn't declared, a
`work_item.recorded` names another provider's item, or a subject isn't
one of its own refs (`check_subject`: `work_item:<id>:…` or
`plugin:<ext>`).

**Calls are bounded** (tsk549): `check` and `invoke` time out after
`HostDeps.call_timeout` (`MachineEnv.provider_call_timeout`: 60 s in the
app, 2 s in `Services::in_memory`); a timeout sends `$/cancel` and
counts as a failure. A restart runs under its own `starting` lock,
never holding `live`, so a start that hangs can't block `stop` (and
through it reconcile, `provider.enable` or `set_instance`). `Peer::start`
refuses once the other side's stream has closed, instead of leaving a
waiter that nothing resolves.

**Health** (`InstanceHealth { state, consecutive_failures, last_ok_at,
mean_invoke_ms }`, per machine, in memory): `state` is `off`,
`missing` (configured, but no enabled extension declares it),
`unapproved`, `unconfigured { problems }`, `checking`, `ready`,
`failing { errors }` (the last five) or `disabled { reason }`. A failed
start or call counts (a refused input or a cancel doesn't); a success
resets the count and updates `last_ok_at` and the moving-average
`mean_invoke_ms`. **Three failures in a row disable the instance**: it
stops and `provider.disabled@1 { instance, reason }` is logged (source
`system:providers`, subject `plugin:<extension>`). So is a handshake that
doesn't match the approved declarations. The log is what keeps it off,
across reconciles and restarts: an instance whose latest
`provider.disabled` has no later `provider.enabled` stays `disabled`,
and one whose record can't be read stays off too (`failing`, naming
the error — not knowing isn't a yes). **A disable wins over a start in
flight** (tsk569): each disable bumps the instance's epoch under the
`running` lock, and a start registers (`admit`) only if the epoch it
began with still holds, so a concurrent reconcile can't bring back
what was just disabled.
Only a person turns it back on — **`provider.enable { instance }`**
(human-only, `External`, not undoable), which logs `provider.enabled@1`,
resets the count and reconciles.

**Settings → Integrations** (`IntegrationsSection.tsx`, IPC UI-only in
the parity table): `list_provider_instances` (every declared provider and
configured instance, with health, approval, credential status and the
config schema), `check_provider_instance { instance, config }` (start
it with that config and `check`, enabling and saving nothing — the
outcome is the view's state) and `set_provider_instance { instance,
enabled, config }` (`ProviderRegistry::set_instance`: enabling checks
first and refuses an unapproved or unconfigured instance, writing
nothing, with the problem's field as `/config/<path>`; then `config.set`
of `extensionInstances` — to enable, `provider.enable` runs **first**, so
a failed enable writes nothing and the config never says enabled for
an instance that wasn't — then a reconcile). Each row
shows its state, its credentials (set into the keychain through
`set_source_credential`, which accepts a provider's credentials too),
the config as a form from the provider's `config_schema`
(`SchemaForm`, P6.B2: Escape resets an edit; a field's problem disables
the actions), Check and Enable / Disable / Enable again. Approving the
program stays in Data → Programs; approving a provider restarts its
running instance on what was approved (`ProviderRegistry::approved`,
called by `approve_project_program`), so updated declarations take
effect instead of disabling it at its next start as changed.

**Tests** (`providers/tests.rs`) run the real fake binary (built beside
the test binary by the workspace build) through a script entry in a
temp extension: an unapproved provider is refused and registers
nothing; an edited declarations file is shown unapproved and refused;
the `bad-declarations` hook is refused naming `/commands` and disables
the instance; an unconfigured instance can't be enabled (nothing
written) and a configured one enables, writes `extensionInstances` and
disables again; `fail-next:3` disables it after three failures with the
reason logged, keeps it off across a reconcile, refuses an agent's
`provider.enable` and comes back on a person's; and the work-items
conformance suite passes through `ExternalWorkItems` over the fake.

## The conformance kit (`crates/oxplow-sdk/src/conformance.rs`, `plugin_test.rs`)

What `oxplow plugin test <name> [--bless] [--json]` runs for each
provider an extension declares ([extensions.md](./extensions.md) "The
SDK"). The person running it runs their own program, so there is no
approval check; credentials come from the environment (the declared
names).

- **`ReferenceClient`** spawns the provider exactly as the host does
  (`host::spawn`: scrubbed env, sandbox, kill on drop) and taps both
  pipes: every line is recorded and validated against the schema goldens
  (`schemas::for_message` by method; a reply is checked against the
  method its request named; an error reply's `error` against the
  `error` golden, which like every wire type allows no unknown field).
  A provider-sent notification or request outside the protocol, a
  host-only notification (`$/cancel`) from the provider, a reply to an
  id the other side isn't waiting on, or a line that isn't JSON-RPC, is
  a violation (tsk570).
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
- **The capability suite** runs through a throwaway host: a temp
  project (`GitProvider::init_repository`: an empty root commit) with a
  copy of the extension, `Services::in_memory` over it, the provider
  approved there and enabled with the fixture config, then
  `work_items_conformance::suite` against its `ExternalWorkItems`.

The red test (`plugin_cli.rs`,
`plugin_test_blesses_a_provider_then_a_changed_transcript_fails`)
scaffolds `plugin new provider fake`, sees the stub fail the handshake,
puts the fake behind the entry, sees the unconfigured fixture and the
missing golden fail, blesses, passes, then edits the golden and gets
one error at the transcript's line and pointer. The fake binary exits
as soon as serving ends (its runtime would otherwise wait on the
blocked stdin reader).
