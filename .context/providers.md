# External providers

A provider is a program oxplow talks to — an issue tracker's bridge, a
docs system's — that implements one of oxplow's capabilities (work items
first) outside the app (P5.D). This doc
covers the protocol (D1), the fake provider (D2) and the host with its
consent and spawn rules (D3), instances with health (D4), the
conformance kit with `oxplow plugin test` (D5), which trackers are
backends, and the MCP adapter (P7.A6).

## The protocol (`crates/oxplow-provider-protocol`)

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
| `invoke` | `InvokeParams { handle, command, input, idempotency_key? }` → `InvokeResult { result, events, inverse? }` | run a declared command; the host logs its `EventDraft { type, v, payload, subject }`s ("Idempotency" for the key) |
| `read` | `ReadParams { handle, collector, state? }` → `ReadResult { records }` | run a collector, streaming `$/record` and `$/state` first |
| `shutdown` | → `null` | |

`PROTOCOL_VERSION` is `"2"` (P10: `InvokeParams.idempotency_key`). A
provider's declarations carry the version, so a bump changes what was
approved: every provider is approved again.

**Errors** (`errors.rs`): JSON-RPC's standard codes plus
`NotConfigured` (-32001), `Auth` (-32002, `data.credential` — the
credential the service refused, when the provider knows it; tsk821),
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

A scripted **work-items** provider, lib + bin (the `oxplow-acp-fake`
pattern), that the host's tests and the conformance kit drive over real
stdio. `declarations()` is its `InitializeResult`: the `work_items`
capability (hierarchy, comments, links, delete and `idempotent_writes`),
the work-items verbs `create` / `update` / `transition` (undoable: its
inverse moves the item back) / `link` / `comment` / `delete` over the contract's inputs (`state` /
`native_state`, its `native.points`; `additionalProperties: false`), one
command of its own, `estimate { ref, points }`, the
`work_item.recorded@2` event type with the core schema (its records
state its links and comments; it declares no `ordering` or `lists`), a
`work_items`
collector over the `work_item` entity, and a config schema requiring
`team`. `check` returns handle `fake:<team>` or a `/team` problem. Items
live in memory as `work_item:fake:W-<n>` with native states `Backlog`,
`Doing`, `Stuck`, `Shipped`, `Dropped` (one per canonical state); every
`invoke` returns the `work_item.recorded` events the host logs. Each
write bumps the item's revision (a deleted item stays, marked
`deleted`); `read` streams each item changed after the cursor, in
revision order, as `$/record` followed by a `$/state { cursor, seen }`
checkpoint (opaque to the host), then `{ records }`.

**Script hooks** — `OXPLOW_FAKE_HOOKS` at spawn, or a `fake/hooks
{ hooks }` notification mid-session, which **replaces** them (`""`
clears), keeping only what was declared at `initialize` — `plain-writes`,
`bad-declarations` (tsk1001) (comma-separated):
`fail-next:<n>` (the next n check/invoke/read fail `Internal`),
`started-file:<path>` (each invoke writes `<path>` as it begins, before
`slow` — so a test knows a call is under way without timing it),
`slow:<ms>` (invoke and read wait first; `$/cancel` interrupts them with
`Cancelled`), `slow-check:<ms>` (check waits first), `crash` (the next request drops the connection; the binary
exits 3) and `bad-declarations` (`initialize` answers something other
than `declarations()`, for the host's handshake check), `progress` (a
read sends `$/progress` before each record, then takes 100 ms over it), `read-fail-after:<n>` (a
read fails after `n` checkpointed records) and `bad-record` (a read
streams another provider's item), `rate-limit:<ms>` (its next invoke or
read is refused `RateLimited`), `needs:<NAME>` (`check` reports
`/credentials/<NAME>` unless that credential reached the process) and
`accepts:<NAME>=<value>` (check, invoke and read answer `Auth` naming
`<NAME>` unless the credential the process holds is `<value>` — a
service that takes one token and no other, for the sign-in tests) and
`refuse-auth` (invoke and read answer an `Auth` that names no
credential), `lax-check` (check doesn't look at credentials, so `accepts`
shows only on invoke and read — a service whose check doesn't validate the
token), `lose-reply` (the next invoke lands and is never answered),
`forget-keys` (it declares `idempotent_writes` and does a re-sent write
again — what the kit must catch) and, at start, `plain-writes` (it
declares no `idempotent_writes` — `plain_declarations()` — and ignores
keys). **It keeps `idempotent_writes`:** an `invoke` sent again with its
`idempotency_key` is done once and answered as the first was; the key
sent with another write is `InvalidInput` at `/idempotency_key`.
**Its service outlives its process** when `OXPLOW_FAKE_STATE` names a
file: items, revisions and answered keys are restored at start and kept
after every invoke, as a real service's are — what the kit's restart
re-sends against (tsk916). Fixtures that launch it through an
extension (`providers/tests.rs` `write_extension`, the SDK's
`just_works.rs`, the desktop `plugin_cli.rs`) set it to
`<project>/.oxplow/fake-state-$OXPLOW_PROVIDER_ID.json` — outside the
extension folder, whose contents its consent covers — and the kit tests
clear it before each run, since the golden transcript records the refs
handed out. Without it the fake forgets on exit, and a declared
`idempotent_writes` fails the kit's restart check.
`tests/stdio.rs` pins all of it through a `Peer`, validating the streamed
notifications against the goldens.

## Which trackers are backends (decided 2026-10-05)

A work-items provider is a **backend for the project's own work**: when
it's the active tracker, every new item goes to it ([work-items.md](./work-items.md)
"One write surface"). So it should have the visibility the person wants
for that work — usually still just theirs, like oxplow's own list or a
local tracker such as beads (filed as the example to build). A team's
tracker (Linear, GitHub Issues) isn't one: an agent's every small task
would land in the team's tracker. At most it would be a separate view of
work, read in through a collector. The reference Linear provider (P7.A5)
was an experiment to prove the protocol and was removed; the fake
provider and `tests-e2e/fixtures/extension` are the complete example.

## The MCP adapter (`crates/oxplow-provider-mcp`)

A provider can be an **MCP server** instead of a program speaking the
protocol (P7.A6). Its manifest entry names the server, a mapping and the
pinned tools instead of an `entry` (exactly one of the two):

```yaml
providers:
  - id: notes
    capability: work_items
    adapter:
      mcp: { command: [bin/notes-server, --stdio] }  # a program in the folder
      mapping: mcp/notes.star                        # Starlark transform(x)
      tools: mcp/tools.json                          # the server's tools/list, pinned
    declarations: provider.json
```

**What runs** is oxplow's own adapter, `oxplow-provider-mcp`, shipped
beside oxplow (a Tauri sidecar staged by `stage-sidecars.sh`; dev and
test builds find it in `target/<profile>`: `host::adapter_bin`), started
like any provider — the scrubbed env, the sandbox and egress proxy, the
verified copy as its cwd — with `--declarations … --mapping … --tools …
-- <command>` (`ProviderSpec::launch`). It runs the server's command as
the folder's file (never a name looked up on `PATH`), as an MCP client
over the server's stdio; the server inherits the sandbox and the
credentials. The loader refuses an adapter whose server, mapping or tools
isn't inside the folder (a path check: a server that doesn't exist fails
when the adapter spawns it, at `check`), an `args:` beside it, and tools
that aren't a JSON list of named tools. **The approval covers it all**: the program hashed is the server's
file, the mapping and tools are named in its args, and the tree hash
covers the folder, so changing the mapping, a pin or the server needs
approving again; the approval row lists the server and each pinned tool
added, removed or changed (`ProviderEffect.tools`).

**A server by `url`** (P9.B4): `mcp` names exactly one of `command` and
`url` (`spec::McpServer`, an enum — the rule is the type):

```yaml
    adapter:
      mcp: { url: https://mcp.example.com/mcp, auth: NOTES_TOKEN }
      mapping: mcp/notes.star
      tools: mcp/tools.json
    credentials: [NOTES_TOKEN]       # or `{ name, oauth }`: signed in for
    network: [mcp.example.com]
```

- The adapter is started with `--url <url> [--auth-env <NAME>]` instead of
  `-- <command>` and speaks **streamable HTTP** to the server (rmcp's
  reqwest client; no redirects followed), through the egress proxy where
  the OS enforces `network`. `auth` names the credential whose value is
  sent as the bearer token; a credential that didn't reach the adapter is
  a `check` problem at `/credentials/<NAME>`.
- The loader refuses a `url` that isn't https (plain http only on
  loopback — the bearer never crosses the network in the clear), a host
  the provider's `network` doesn't list, an `auth` that isn't one of its
  `credentials` or is one of them named as a `client_secret` (the host's
  alone, never in the process's environment, so it could never reach the
  server — tsk835), and `auth` beside a `command`.
- **A `401` is `Auth`** naming the bearer's credential (at the server's
  initialize, `tools/list` or a tool call), so a signed-in bearer is renewed and the call tried once
  more ("Credentials and sign-in"); a pasted one is a failure that says
  the bearer was refused. That holds with or without a
  `WWW-Authenticate` challenge (tsk831): rmcp types only a challenged
  `401`, so the adapter reaches the server through its own client
  (`Http`, rmcp's reqwest client delegated to) that reads a bare one —
  rmcp's `HTTP 401 …` response error or reqwest's status error — as
  the same. The one shape it can't see is a `401` whose body is a
  JSON-RPC error: rmcp hands that to the session as the server's error,
  without its status. **A `403` isn't `Auth`**: the server takes the
  token but refuses what it may do, and a renewed token carries the same
  grant — it is an ordinary failure naming the scope the server wants
  (from its `insufficient_scope` challenge). The notes server's
  `Refusal` (`Challenge | Bare | Forbidden`) serves each.
- **What the approval covers** is the url, the mapping, the pins, the
  declarations and the grants — **not the server's code**, which runs
  elsewhere (`ProjectProgram.remote`: the hash covers the url's bytes
  where a local program's file would be; `args` carry the mapping, the
  tools file and `--auth-env <NAME>`). The approval's credentials line
  marks the one sent as the bearer (`NAME (sent to the server as its
  bearer token)`, `ProviderSpec::credential_grants`), so moving `auth` to
  another declared credential shows in the diff as its own `now reads …`
  line beside any other change (tsk834). What stands between a changed
  server and the project is the pin: `check` refuses a server whose
  tools aren't exactly `tools.json`. Data → Programs says so on its row
  ("The server runs elsewhere: its code isn't part of this approval…"),
  and the approval's diff words it as "runs oxplow's MCP adapter against
  <url>".
- Everything after the transport is the same: the pin, the mapping, the
  checks on what the mapping returns.

- **`initialize`** answers the checked-in declarations (the handshake
  checks them like any provider's).
- **`check`** starts the server and runs `tools/list`: a server whose
  tools differ from `tools.json` in anything — each tool is pinned whole
  as the server lists it: name, title, description, input and output
  schemas, annotations (`destructiveHint`), so no hint changes under the
  pin (§6.8; tsk719) — or that lists a tool twice, is a problem at `""`
  naming the first difference,
  and no handle (a changed server needs its pins updated and a person's
  approval). A clean check keeps the config under a handle. Starting the
  server holds nothing else, so a server that hangs starting never stops
  the adapter answering `$/cancel` or `shutdown`; `shutdown` stops the
  calls in flight first, then the server (tsk720).
- **`invoke` and `read`** run the mapping's `transform(x)` twice, each
  under the sandbox with a 5 s budget: `x = { phase, command, input,
  config, provider }` with `phase: invoke` → `{ tool, arguments }` (or
  `{ refuse: { field, message } }`, an `InvalidInput`); the tool's output
  (its structured content, else its text, as JSON when it parses) comes
  back as `x.output` with `phase: invoked` → `{ result, events, inverse? }`.
  A read: `phase: read` (`x.state` the checkpoint) → a tool call; `phase:
  records` → `{ records, state }`, streamed as `$/record`s then one
  `$/state`. **Tool output is data**: the mapping reads it and nothing
  runs it. **What the mapping returns is checked**: an event type the
  declarations don't list, or a subject, item or record that isn't this
  provider's (`work_item:<id>:…`, the id from `OXPLOW_PROVIDER_ID`) fails
  the call; so does a `records` answer without a `state` (a missing
  checkpoint would restart every read from nothing). **A tool error**
  (`isError`) is the server refusing the request, not a failure: the
  mapping runs with `x.error = true` and the error's output and may
  answer `{ refuse: { field, message } }`; an error it passes over (or
  trips on) is an `InvalidInput` naming the tool. Either way it never
  counts toward disabling the provider (tsk719). The notes server reports
  a missing note, parent or bad state as a tool error naming its
  argument.

**Tests**: `tests/adapter.rs` drives the adapter over stdio in a copy of
the fixture extension `tests/fixtures/notes/` in front of the test server
`oxplow-provider-mcp-notes` (`src/notes.rs`, an rmcp server: `list_items`,
`create_item`, `update_item` over notes `open | doing | stuck | closed |
dropped`; over stdio, or with `--http <addr>` over streamable HTTP at
`/mcp`, behind the bearer in `$NOTES_BEARER` when set — the library's
`notes::serve_http` / `notes::http_router` is what the tests run
in-process, behind a token check (`notes::Tokens`; `notes::only(t)`
takes one token, the OAuth stand-in its live access tokens);
`a_server_by_url_is_pinned_and_called` covers the pin, the calls, a
bearer refused at check and mid-session, and a missing one, and
`tests/kit.rs` runs `plugin test` on the fixture with its server by url) — the pin (an edited `tools.json` is refused), the mapping's
calls and outcomes, the read and its cursor, an undeclared event and a
foreign ref refused; `the_pinned_tools_are_the_servers` checks the pin
(`OXPLOW_BLESS=1` re-pins). `tests/kit.rs` runs the fixture through
`oxplow plugin test`, the work-items suite through a throwaway host over
the adapter included.

## The host (`crates/oxplow-app/src/providers/`)

**The manifest kind** (`spec.rs`; an MCP server's form is in "The MCP
adapter" above): `providers:` is an experimental kind,
so only a private extension's are loaded (onto `Extension.providers`; a
disabled extension has none):

```yaml
providers:
  - id: fake                     # the ref segment and command namespace
    capability: work_items       # the only one a provider implements today
    entry: bin/provider          # a program in the extension folder
    args: [--stdio]
    env: [TRACKER_URL]           # host variables passed through by name
    credentials: [token]         # keychain values, as env; or
                                 # `{ name, oauth }` — "Credentials and sign-in"
    network: [api.example.com]   # hosts it may reach
    declarations: provider.json  # its InitializeResult, checked in
    id_pattern: "[A-Z]+-\\d+"     # a work list's own ids (optional)
```

`fields` (a work list's own, kept in `native`: `[{ name, title, kind:
enum|text|number, values? }]`, work-items.md "Declared fields") are
published with the instance's capability row. `id_pattern` (a regex
matched whole) is what a work list's ids look
like: while it's the active one, a loose id (`ENG-12`) in a work-item
command resolves to its item (`work_item::with_loose_refs`,
work-items.md).

The loader refuses (into `errors`) an id that isn't lowercase
snake_case, is `oxplow` or a core namespace, or repeats; an unknown
capability; an `id_pattern` that isn't a regex, or on another capability; an entry or declarations path outside the folder (or the
manifest, or under `lenses/`); a bad host pattern; and declarations that
don't parse, speak another protocol version, lack the named capability,
or — for `work_items` — lack `create` / `update` / `transition` (and
`link` / `comment` / `delete` when its features say so), declare a verb
that isn't `confirm: never` and `effect: record` (the `work_item.<verb>`
command running it is what a person confirms and what is gated).

**Consent precedes execution** (`exec_consent`, `ProgramKind::Provider`,
key `provider:<ext>/<id>`): the approval hash covers every file in the
extension folder but the manifest and `lenses/` — dot-files, the entry
and the declarations file among them; a symlink anywhere makes it
unapprovable — plus the entry path, `args` (hashed relative to the
folder, where it runs; an arg path leaving the folder is refused at
load), `env` names, `credentials` (each as `CredentialDecl::grant` renders
it — a signed-in one with its endpoints, client id, scopes, client secret's
name and redirect port, so changing where it signs in asks again) and
`network`. So a changed declaration is a new version,
shown unapproved in Settings → Data → Programs (with its grants listed)
until a person approves it again. Every start re-checks it, restarts
included.

**The approval shows what it changes** (P6b.E3): an unapproved
provider's row loads `provider_declaration_effects { instance }`
(`ProviderRegistry::declaration_effects`: the spec and declarations on
disk against the last approved copy's — `host::last_approved`, the
intact `copies/<ext>/<id>/<hash>` a start last ran, so a changed spec
that stopped the instance, or a restart, still shows what changed — or
everything it declares when it never ran; read, never run) and lists
its hosts, credentials, commands added, removed or changed (destructive
ones marked), features, or — when none of those shows the change — where
the declarations first differ (`ProviderEffect.lines`, worded in Rust
by `extension_effects::approval_lines`, P8.C5). Its Approve stays disabled
until that diff has loaded (`canApprove`), on top of the reviewed
`version` round trip; a diff that fails to load is shown on the row
("Couldn't compare its declarations: …") and Approve stays disabled with
that reason. The diffs reload only when the set of unapproved providers
or their on-disk `version` changes.

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
working directory and `OXPLOW_EXTENSION_DIR` are the copy. Instances of
one program share its copies and start together routinely, so one start
of a program copies at a time (tsk839): the copy, keep and cleanup run
under an advisory lock on `<ext>/<id>.lock` (`fs2`, beside the folder
the cleanup empties) that threads and processes alike wait on — no
start removes another's temp copy or the copy another just kept, and a
temp copy found under the lock was abandoned.

**The spawn** (`host.rs`, `connect`) mirrors an `exec` source: a
scrubbed environment (PATH, HOME, the declared `env` names, the
credentials from the instance's keychain accounts
(`instance:<project>:<ext>/<instance id>:<name>`),
`OXPLOW_EXTENSION_DIR`, `OXPLOW_PROVIDER_ID` = the **instance id**), the egress proxy and
`sandbox-exec` where the OS enforces `network`, stderr to the log, and
`kill_on_drop`. Then **the handshake**: the live `initialize` must equal
the approved declarations (`HostError::DeclarationsChanged` names the
first difference), and `check` of the instance's config must return a
handle (`HostError::Unconfigured { problems }` otherwise). Requests from
the provider are answered `MethodNotFound`.

**Instances** (§10.3; project scope). An instance is
`<extension>/<instance id>`, configured in `.oxplow/project.yaml`:

```yaml
extensionInstances:
  tracker/issues: { enabled: true, config: { team: ENG } }
  # A second account: its own id, saying which provider it is (P9.B1).
  tracker/issues_acme: { enabled: true, provider: issues, config: { team: ACME } }
```

**An instance id** follows the provider id's rule — one rule,
`oxplow_domain::work_items::provider_id_problem` (tsk840): lowercase
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
| the ref segment (`work_item:issues_acme:ENG-1`) and `check_subject` | the consent key `provider:<ext>/<provider id>` and its hash |
| the bus namespace (`issues_acme.estimate`) | the approved copy (`copies/<ext>/<provider id>/<hash>`) |
| the work-items registry id, `v_capability_provider.provider`, the `activeProviders` value | Data → Programs' row, `declaration_effects` |
| `OXPLOW_PROVIDER_ID` (the provider is told which instance it is) | the kit's fixtures and transcripts (it tests the program, as its default instance) |
| `plugin_health.contribution`, `provider_collector_state.instance` (the name) | |
| the credential accounts (`instance:<project>:<ext>/<id>:<name>`) | |

Consent stays per program: its hash covers the folder, the entry, args
and the *names* of its grants; two instances differ only in config and
credential *values*, which it never covered, so a second approval would
review nothing. Approving a program restarts every running instance of
it, and starts an enabled one that was down for want of approval
(`ProviderRegistry::approved` reconciles, tsk1062).

`ProviderRegistry::resolve(instance)` is the one mapping from a name to
`{ ext, spec, id }` (`enable` / `check` are the default instance's;
`enable_instance` any). A person adds another with `add_instance`
(refused when the extension doesn't declare the provider, the id is
taken, or the id is another provider's own), removes one with
`remove_instance` (it stops, and everything of it goes — tsk841: its
config entry; its credentials, named by the extension or, when the
extension no longer declares its provider, by the copy it last ran
(`host::last_ran_credentials`); its read checkpoints
(`provider_collector_state`, so one added again under the id reads from
the start); its `plugin_health` row; and any `activeProviders` choice
naming its id — except, for a project's replacement of a global
instance, what the global one now showing through has),
and sets a credential with `set_credential` (a declared name; the
instance restarts on it) — RPCs `add_provider_instance`,
`remove_provider_instance`, `set_instance_credential`, UI only. A
provider's credential is its instance's: collectors keep the
extension's `source:<project>:<ext>:<name>` accounts. An extension's
`ui.commands` name the default instance's commands (`issues.estimate`);
another instance's are on the bus under its own namespace.

**Scope** (P9.B2). An instance is the **project's** (above: shared with
the team in `project.yaml`) or the **person's** — global, in this
machine's `instances.yaml` in the global config dir
(`oxplow_config::GlobalInstances`: `instances: { "<ext>/<id>": { enabled,
config, provider? } }`, the same entries and the same validation):

- a global instance applies in **every project whose catalog has its
  extension enabled**; where the extension isn't, it isn't listed at all;
- a project's entry of the same name **replaces it there, whole**
  (`instances_config()` is the one merge: global, then project) — to turn
  it off or configure it differently in one project. A person makes one
  from the global row's **Off in this project** (`off_here`, RPC
  `turn_off_provider_instance_here`, tsk843): the project's own entry,
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
  global one went (tsk838). Now a global one appearing or going never
  moves a project entry onto other credentials, and removing a project's
  replacement removes its own credentials, never the global one's;
- its credentials are the person's, set once:
  `instance:global:<ext>/<id>:<name>` (a project instance's are
  `instance:<project>:…`). Changing one — a pasted value, a sign-in, a
  sign-out — restarts it in **every** project, not only where it was
  changed (tsk842): the keychain can't be watched, so
  `credential_changed` bumps the instance's count in
  `instance-credentials.yaml` beside `instances.yaml`
  (`oxplow_config::CredentialGenerations`: counts only, never a value,
  under the same kind of machine-wide lock), and every other oxplow, on
  its per-minute tick (`reconcile_if_global_changed`), restarts a global
  instance whose count moved since it last started it — so a rotated and
  revoked token never keeps running elsewhere on the old value;
- **consent stays per project**: the hash is of *this* project's
  extension folder and approvals are kept per project, so a global
  instance in a project that hasn't approved the program is `unapproved`
  there, and its row says so;
- it is written where it lives (`set_instance`: the project's entry if
  there is one, else the file; `add_instance(…, scope)`;
  `remove_instance` takes a project's replacement first, and the global
  one then shows through). The file is written only by the registry for a
  person (`Actor::Human`; anyone else is `Denied`) — no command reaches
  it, so no agent or lens can;
- **every change is one read-modify-write of the instances as they are
  then** (tsk837): `write_instances` takes an `edit` of them, under the
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
  it — tsk904), and answers anything but `GET /callback` itself — with a
  plain-text page marked `nosniff` and `no-store` — reads each connection on its own
  task with a deadline (10 s) and a 16 KiB head cap, and never reads a
  body, so an idle or oversized connection never holds up the real
  redirect (tsk825).
- **Token requests** (`token_request`): the client secret goes in an
  HTTP Basic header (`client_secret_basic`, each half form-encoded — RFC
  6749 §2.3.1; the default every server must accept) or, with
  `client_auth: post`, in the form; with Basic neither id nor secret is
  in the form. A token endpoint that answers with a redirect is an error
  — reqwest follows none, so the secret and the code go to the declared
  endpoint only. An answer whose `token_type` isn't `Bearer` is refused
  (a missing one reads as bearer), and `expires_in` is read as a number
  or a numeric string (tsk829).
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
  so a read that started the process itself still renews — tsk907). **Only the refused credential is renewed** (tsk821,
  `Instance::renewable`): the one the `Auth` names, when the instance
  signs in for it; with none named, its credential only when the process
  is handed exactly one and it is signed in — with two or more (a pasted
  key beside a sign-in counts, tsk908) it can't know which and renews
  none, so a good sign-in is never lapsed for another's refusal and the
  refusal is the
  call's failure. A named credential that is pasted, not signed in for,
  has nothing to renew. Another credential is never renewed, so a
  renewal its service refuses for good never lapses one it didn't name.
  None of that counts as a failure; an `Auth` after the renewal does,
  like any other. Renewals of one account are **serialized** in-process
  (`renewal_lock`), and **a refusal renews only the token it refused**
  (tsk928): `oauth::access_token` takes the refused access token, and
  forces a renewal only while that token is still the one stored — a
  caller that waited, another project's instance on the same global
  account, or a Settings Check refused for a token already replaced gets
  the stored one (renewed only if it is lapsing), so a rotating refresh
  token is spent once (tsk827). Each instance keeps the credentials it
  gave its process (`Live.given`), which is how it names the refused
  token. The write compares first: a sign-out during the renewal
  stays signed out, a token replaced meanwhile (a new sign-in, another
  process) is used rather than overwritten, and a refused renewal marks
  the token lapsed only if it is still the one that was refused. Calls
  refused together renew once (tsk828): `Instance.renewed_at` records
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
- **Signing in** is a person's: IPC `begin_oauth_sign_in { instance,
  name, redirect_port }` (`ProviderRegistry::begin_sign_in`, UI-only)
  returns the page to open — **only for a provider approved as it is now** (tsk824): the
  endpoints, client id and client-secret name are part of the approval,
  and a sign-in sends the code, the PKCE verifier and the client secret
  to them, so an edited endpoint is refused until a person approves it
  again (the row's Sign in is off meanwhile); the renderer opens it in **the person's own browser**
  (`tauri-bridge/systemBrowser.ts`, not the sandboxed external-URL
  window — their sessions live there and services refuse embedded
  webviews). The row runs it (`SignInRow`): the shell listens
  (`listen_for_oauth_redirect { port? }` → `{ id, port }`, shell-only — on
  the credential's `redirectPort`, else any free port), the core begins on
  that port, the browser opens the page, and each redirect the shell
  catches (`await_oauth_redirect { listener }`) goes to the core by IPC
  `complete_oauth_sign_in { instance, name, redirect }` (UI-only), the
  shell answering the browser with the core's verdict
  (`answer_oauth_redirect { listener, outcome }`). A listener is known by
  its **id**, never its port (tsk905): a newer sign-in may listen on the
  same port, and a late answer or stop for an old one must not touch it. A
  replaced or left sign-in is stopped with `stop_oauth_redirect
  { listener }`, which tells a browser still waiting and resolves once the
  socket is closed — the row awaits it before listening again. `SignInCompletion` is
  `signed_in`, `failed { error }`, or `not_this_sign_in { reason }` —
  refused, nothing done, the wait goes on. A match ends the sign-in: the
  provider is re-checked as approved with the declaration it began with
  (an endpoint edited meanwhile gets nothing) and with its credentials
  where they were (a global instance turned off here, or a project entry of
  its name arriving, moves them: `failed`, sign in again — tsk909), the
  code is exchanged and the token stored — and then it answers, so the
  browser hears at once; `credential_changed` restarts the instance on it
  afterwards and the renderer hears `CredentialChanged { instance, name,
  error }` when it has — the keychain is no model, so this is one of the
  bus's UI-only signals (tsk906). The finish runs on a task of its own: a
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
  again; tsk904). A stop returns once the socket is closed. A second sign-in for the same credential
  abandons the first (its redirect is no longer the sign-in's; the row
  stops its listener), and the first's news says it was replaced. Every
  sign-in has a **number** (`begin_oauth_sign_in` → `{ url, signIn }`,
  `SignInId`); `CredentialChanged { …, signIn }` names the sign-in it is
  about, so a row ignores news of another sign-in of the credential — its
  own replaced one, another window's (tsk929). A row that is left, or
  whose browser never opened, cancels its sign-in (`cancel_oauth_sign_in
  { instance, name, signIn }`, UI-only): nothing of it — verifier, client
  secret — is kept the five minutes, and its news says it was cancelled. Each `(instance, credential)` has its own gate
  (`sign_in_gate`), held while its sign-in begins, finishes — across the
  code exchange — expires or is abandoned, so two clicks at once leave one
  under way, and removing an instance — a project entry that uncovers a
  global one too — takes its credentials' gates, waits for a finish in
  flight and abandons its sign-ins first, so nothing is kept for what's
  gone (tsk826). A slow token endpoint holds up only its own sign-in
  (tsk910). The row's Sign in is off while one starts, and off
  without the desktop app (a plain browser can't catch the redirect),
  with the reason shown.
  `set_instance_credential` refuses a value for a signed-in credential;
  with no value it signs out.
- **The view**: `ProviderInstanceView.credentials` is
  `InstanceCredential { name, set, sign_in?, redirect_port? }`
  (`redirect_port`: the declared one, where the shell must listen), `sign_in` being
  `not_signed_in | signed_in { until? } | sign_in_again` (`until` only
  for a token that can't renew itself). The Integrations row shows a
  **Sign in** / **Sign in again** button and **Sign out** instead of a
  value box (`signInLine`).
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
  string `expires_in` (tsk830). At `/mcp` it serves the notes MCP server
  behind exactly the access tokens it issued and still holds live, so a
  signed-in bearer runs end to end (oxplow-provider-mcp's
  `tests/signed_in.rs`: sign in, a by-url instance on the token, the
  server expires it, the next create renews once and lands, still
  Ready); `POST /sim/expire` lapses every
  access token and `POST /sim/revoke` revokes the grant (its access
  tokens stop working, its refresh is `invalid_grant`). Run it by hand
  with `cargo run -p oxplow-oauth-sim -- --http 127.0.0.1:8124` (it
  prints its urls). **Development only:** it is a dev-dependency of the
  crates that test sign-in and a binary, never a dependency of anything
  that ships. Guard `no_test_double_is_a_production_dependency`
  (tsk930) holds that for every test double — a workspace crate named
  `*-fake` or `*-sim` (`oxplow-oauth-sim`, `oxplow-ai-fake`,
  `oxplow-provider-fake`, `oxplow-acp-fake`): no workspace crate reaches
  one through a normal or build edge of the resolved graph (`cargo
  metadata`), however its manifest spells the dependency.

The key is human-only (`HUMAN_ONLY_KEYS`: enabling runs a program) and
shared with the team; whether it *runs* is per machine (approval,
credentials, health). The config object is the provider's
`config_schema`'s; `check` validates it.

**The registry** (`registry.rs`, `Services.providers`) keeps the running
instances matching the config: `reconcile()` runs at boot, on the
extension catalog's change signal (`spawn_reconciler`) and on an
`extensionInstances` / `activeProviders` change (the `config.providers`
reactor on `config.changed`, P7.B6), starting enabled instances and
stopping the rest (a config or spec change restarts one). `enable(ext,
spec, config)` starts an instance and only then registers its capability
provider (`ExternalWorkItems::provider`: id, declared features, and the
`WorkItemVerbs` the `work_item.*` commands call — the trait every list
implements, oxplow's own tasks included (`oxplow_tasks::OxplowTasks`) — each verb's
input checked against its declared schema first) in
`Services.work_items`, and its **other** declared commands on the bus as
`<id>.<name>` (`External`, `Experimental`, all invokers; confirm /
effect / undoable as declared). Its capability's verbs are never
commands of their own: `work_item.<verb>` is the one write surface
(P7.A1). A refusal —
unapproved, unconfigured, a handshake that doesn't match — registers
nothing; a start that merely failed (it may come up) registers and
counts as a failure, and its next call restarts it after a backoff that
doubles from `MachineEnv.provider_backoff` (1 s in the app, 0 in
`Services::in_memory`) up to 60 s. `stop(instance)` removes both and
kills the process. Its commands register whole as the namespace's
owner (`register_namespace(id, "provider:<instance>", …)`, all or
none), each with the interim id `<instance>.<capability>.<name>`
(`providers::command_id`: `fake.work_items.estimate`); an id whose
namespace is already held (`namespace_owner`) or that is already a
provider is refused. A stopped instance's go with `unregister_source`. **A provider emits only its capability's event
types** (`spec::allowed_event_types`: `work_items` → `work_item.recorded@1`
or `@2` — a published version's schema never changes, since declarations
are compared to it exactly;
tsk548): declaring any other type — another core one such as
`plugin.enabled`, which would clear another contribution's disable — is
refused when the manifest loads, and the declared schema must equal
core's (checked at enable). Its own types are P7. A command's run
invokes the process and hands the bus its result, its inverse (as
`<id>.<command>`, or for a verb `work_item.<verb>`) and its events —
refused if a type isn't declared, a
`work_item.recorded` names another provider's item, or a subject isn't
one of its own refs (`check_subject`: `work_item:<id>:…` or
`plugin:<ext>`).

**Calls are bounded** (tsk549): `check` and `invoke` time out after
`HostDeps.call_timeout` (`MachineEnv.provider_call_timeout`: 60 s in the
app, 2 s in `Services::in_memory`); a timeout sends `$/cancel` and
counts as a failure. A restart runs under its own `starting` lock,
never holding `live`, so a start that hangs can't block `stop` (and
through it reconcile, `oxplow.plugin.enable` or `set_instance`). `Peer::start`
refuses once the other side's stream has closed, instead of leaving a
waiter that nothing resolves.

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

**A call's outcome is its instance's while that instance runs**
(tsk820, tsk836). Every health write a running instance causes takes the
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
turned it off and on, or when an approval restarted it, can't bring a
process back — nor report its restart's `unapproved`, `unconfigured`,
changed declarations or failure against the successor. An enable's first
start, before the instance runs, reports by name (`made_by: None`).

**Health** (`InstanceHealth { state, consecutive_failures, last_ok_at,
mean_invoke_ms, rate_limited_until, activity }`, per machine, in memory): `state` is `off`,
`missing { reason }` (configured, but it names no provider an enabled
extension declares; the reason says what to add), `refused { reason }`
(enabled, but it can't run as configured — above), `unapproved`, `unconfigured { problems }`, `checking`, `ready`,
`failing { errors }` (the last five) or `disabled { reason }`. A failed
start or call counts (a refused input or a cancel doesn't); a success
resets the count and updates `last_ok_at` and the moving-average
`mean_invoke_ms`. The count and the disable are the policy every plugin
contribution shares (P7.C1, `plugin_health.rs`; [extensions.md](./extensions.md)):
the `plugin_health` row keyed `<extension>` / `<instance id>`, kind
`provider` (`v_plugin_health`). **Three failures in a row disable the
instance** — a write counts once however often it is sent again under
its idempotency key (an effect's automatic retries, a person's retry of
the same composition: `Instance::failed_keys`, tsk913), so one outage
that a reaction retries through doesn't halt it: it stops, and the row (`disabled`, its reason) and
`plugin.disabled@1 { plugin, contribution, kind, reason }` commit
together (source `system:plugins`, subject `plugin:<extension>`). So is
a handshake that doesn't match the approved declarations. The row is
what keeps it off, across reconciles and restarts, and one whose row
can't be read stays off too (`failing`, naming the error — not knowing
isn't a yes). `InstanceHealth` is the process's state, showing the
row's count. **A disable wins over a start in
flight** (tsk569): each disable bumps the instance's epoch under the
`running` lock, and a start registers (`admit`) only if the epoch it
began with still holds, so a concurrent reconcile can't bring back
what was just disabled.
Only a person turns it back on — **`oxplow.plugin.enable { plugin, kind,
contribution }`** (human-only, `External`, not undoable; it replaced
`provider.enable`), which marks the row `ok`, logs `plugin.enabled@1`,
resets the backoff and reconciles. Settings → Integrations' Enable runs
it before writing `extensionInstances`.

**Reading: collectors and sync** (`sync.rs`, P7.A3). `Instance::read`
runs one declared collector's `read` — **one at a time per collector**
(a second sync waits, then resumes after the first; tsk715) — from the
checkpoint it last stored
(`provider_collector_state`, [data-model.md](./data-model.md)), through
`start_streaming`. Each `$/record` must be the collector's entity and —
for a work-items provider — a `WorkItemRecord` of its own item; it is
kept as a `work_item.recorded@2` envelope (the actor's source) until the
next `$/state`, which commits the batch **and** the checkpoint in one
transaction, so a read that fails midway keeps exactly what its last
checkpoint covered and the next read resumes there. A record equal to its
item's last `work_item.recorded` (a write's or an earlier read's —
looked up by `json_extract(payload, '$.item.ref')`, index V153) is
**not logged**: it restates nothing, and logging it would echo a write
back to whatever reacted to it — an effect that writes another
provider's item would hear its own write again on every sync (tsk799).
The checkpoint's `records` still counts what was read. This leans on a
read restating a write exactly, which the work-items conformance suite
checks. The result's
`records` must equal what was streamed; records after the last
checkpoint of a read that succeeded land too. A record that breaks a
rule, a count that doesn't match, or a read that sends nothing for
`call_timeout` (`$/cancel` follows) fails the read and counts toward the
instance's health like a failed call (a refused input or a cancel
doesn't). The projection (`work_items.project`) then restates
`v_work_item`.

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
reconcile (tsk716). An external write whose
reply was lost (it landed at the tracker but the call timed out) is
restated by the next read.

**Settings → Integrations** (`IntegrationsSection.tsx`, IPC UI-only in
the parity table): `list_provider_instances` (every declared provider and
configured instance, with health, approval, credential status and the
config schema), `check_provider_instance { instance, config }` (start
it with that config and `check`, enabling and saving nothing — the
outcome is the view's state) and `set_provider_instance { instance,
enabled, config }` (`ProviderRegistry::set_instance`: enabling checks
first and refuses an unapproved or unconfigured instance, writing
nothing, with the problem's field as `/config/<path>`; then `oxplow.config.set`
of `extensionInstances` — to enable, `oxplow.plugin.enable` runs **first**, so
a failed enable writes nothing and the config never says enabled for
an instance that wasn't — then a reconcile). Each row
shows its state, its credentials (each set into the keychain through
`set_instance_credential`, or signed in for — "Credentials and sign-in"),
the config as a form from the provider's `config_schema`
(`SchemaForm`, P6.B2: Escape resets an edit; a field's problem disables
the actions), Check and Enable / Disable / Enable again, and each
collector's line (`collectorLine`: records delivered, last read, a
failed read's error; the view's `collectors`) with **Sync Now** while it
runs. **Instances** (P9.B6): under the rows, "Add another instance of
`<ext>/<provider>`" takes a name (the instance id: snake_case, not
already an instance — `newInstanceProblem` says so before the core
does) and a scope ("This project's" / "Mine, in every project") and calls
`add_provider_instance`; Enter adds, Escape clears. A named instance, a
project's replacement of the person's own, and one whose provider is
gone have **Remove** (`InlineConfirm`, `remove_provider_instance`:
`canRemoveInstance`) — a global one whose provider is gone included: a
global instance is left out of the list only where the project doesn't
have its extension enabled; a provider's own instance is turned off, not
removed. The page is covered by `IntegrationsSection.test.tsx` and
`integrationsModel.test.ts`, the core by `providers/tests.rs`, and the
whole in a browser by the suite's `integrations/fake-provider` spec
(`tests-e2e/`, P11): a person fills the fake's Team, presses Check and
Enable, makes it the project's work list in Settings → Pieces; an item created on it reaches the
fake's service, and after Sync Now it is on the Board. Approving the
program stays in Data → Programs; approving a provider restarts its
running instance on what was approved (`ProviderRegistry::approved`,
called by `approve_project_program`), so updated declarations take
effect instead of disabling it at its next start as changed — and starts
an enabled instance that was waiting for the approval.

**Tests** (`providers/tests.rs`) run the real fake binary (built beside
the test binary by the workspace build) through a script entry in a
temp extension: an unapproved provider is refused and registers
nothing; an edited declarations file is shown unapproved and refused;
the `bad-declarations` hook is refused naming `/commands` and disables
the instance; an unconfigured instance can't be enabled (nothing
written) and a configured one enables, writes `extensionInstances` and
disables again; `fail-next:3` disables it after three failures with the
reason logged, keeps it off across a reconcile, refuses an agent's
`oxplow.plugin.enable` and comes back on a person's; and the work-items
conformance suite passes through the dispatching `work_item.*` over the
fake; `work_item.*` writes the fake's items through its process with one
audit row (and undo dispatches again), its verbs aren't on the bus but
`fake.estimate` is; a verb's input is checked against its declared schema
and a parent or link target of another provider is refused.

**Its commands can appear in core menus**: the extension's
`ui.commands` may name the provider's own `<provider>.<name>` commands
(not its capability's verbs — those are `work_item.<verb>`), grouped
under the provider and checked against its declarations
([extensions.md](./extensions.md)).

**Its features are published** while it runs: `admit` writes the
instance in the capability registry (`CapabilityRegistry::set_external`)
and `tear_down` takes it out, each restating `v_capability_provider`, so
the UI offers only what the provider declares
([work-items.md](./work-items.md)). Each reconcile restates the rows
(`CapabilityRegistry::publish`), so a choice takes effect with the config
change.

## Idempotency

A write to a provider may land without oxplow learning it did (a crash, a
timeout, a lost reply). Sending it again is safe only if the provider can
tell it is the same write. The contract (P10, built):

- **`features.idempotent_writes: true`** in a provider's work-items
  declaration promises that two `invoke`s carrying the same key perform
  the write once and answer alike (a key sent with another write is
  `InvalidInput` at `/idempotency_key`). `PROTOCOL_VERSION` is `"2"`.
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
  same ([extensions.md](./extensions.md) "Attempts"); anything else waits for
  a person's `oxplow.effect.retry`, asked first. A failed command a person ran
  is reported, not retried.
- **The kit checks the promise** (the work-items suite,
  [work-items.md](./work-items.md) "Conformance"): a provider that
  declares it and doesn't keep it fails.

The fake declares and keeps it, which proves the host and nothing about a
service: a real provider declares it only once its service is known to do
a keyed write once.

## The conformance kit (`crates/oxplow-sdk/src/conformance.rs`, `plugin_test.rs`)

What `oxplow plugin test <name> [--bless] [--json]` runs for each
provider an extension declares, after its check and its lens and
collector examples ([extensions.md](./extensions.md) "The SDK"). The person running it runs their own program, so there is no
approval check; credentials come from the environment (the declared
names — for a signed-in credential, an access token the author got
themselves: the kit never signs in — and never a client secret).
The report's **`left`** lists what the run made in a provider's own
system and didn't remove — the ref a work-items `create` example returned,
and the items the conformance suite filed and couldn't delete
(`work_items_conformance::SuiteRun.left`: everything, for a provider
without `delete`); the text form prints each as `left in the provider:`.
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
