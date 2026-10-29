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
| effect | an extension's reaction to events ("Effects" below) |

"Plugin" is only a Claude Code or opencode plugin. Two more terms keep
authority honest: a **need** is what a contribution says it requires (a
capability, a feature, a scope); a **grant** is what a person approved
(a program's hash and its declared access). A need is never a grant.

## Contributions (descriptors)

Every contribution has one `oxplow_domain::contribution::Descriptor`,
built at load from the kind's typed declaration
(`extensions::descriptors::describe`, `Extension.contributions`): its
identity (`ContributionId { extension, kind, local }`; one grammar for
`local`, dots only for the kinds whose ids are dotted), its owner
(extension, origin and the extension's `version:`),
how it runs (`RunsOn`), its needs and `after`, and the **global name** it
takes in a space extensions share (a metric key, a fact collector's id).
The kinds' table (`contribution::KINDS`: manifest key, title, whether two
extensions may share a name) is the one place those facts live.

**A contribution's only public name is its URI** (`ContributionId::reference`,
its `Display`; [resources.md](./resources.md) "Using URIs"): the resource
`oxplow/<kind>` with the kind's own name as the key param,
`oxplow:oxplow/<kind>?<kind>=<extension>/<id>`
— `oxplow:oxplow/job?job=effort-review/review`,
`oxplow:oxplow/collector?collector=triage/todos`,
`oxplow:oxplow/page?page=github/pr`,
`oxplow:oxplow/dataset?dataset=effort-review/reviews` — or,
for a kind whose names are already global (`ContributionKind::global_name`:
command, measure, metric, dimension, event type, resource),
`oxplow:oxplow/<kind>?<kind>=<name>` —
`oxplow:oxplow/command?command=oxplow.review.accept`,
`oxplow:oxplow/metric?metric=acme.todos`.
Storage (`v_contribution.uri`, `v_extension_run.contribution`,
`v_job_run.job`), events, manifests (imports, exports, `where:`), grant
keys and the desktop carry that string; every contribution kind in
`KINDS` is a resource of oxplow (`oxplow/page`, `oxplow/command`, …) whose
key param is its own name (`build::key_param`), so a new kind is
linkable with no other change. Text names a contribution as a
`ContributionRef` (`FromStr`); a local URI says its owner, a global one is
resolved by the catalog (who declares that name). Starlark reads one with
`uri.contribution(r)` → `{ kind, extension, id }`.

At load, `check_ids` turns an id that breaks the grammar into a load
error, and the folder name must be a valid extension name
(`valid_extension_name`; bundled names are core's). Across the catalog,
`check_catalog` (end of `load_extensions_in`, after disables) adds every
enabled extension's descriptors to a `Registry` in catalog order: a global
name a kind doesn't share that another already holds is refused — a load
error of the later extension naming both — and the contribution is taken
out of that extension, so nothing downstream sees two (this is what keeps
the first extension's metric when two declare one key). `version:`, when
given, is checked by `manifest_v2::valid_version`; a bundled extension's
version is oxplow's own. Every kind is described: collectors
(global: a fact collector's id), effects, commands (global: the bus id),
advisories, skills (global: the name), implementations (global:
`<capability>/<id>`), providers (global: the id, which is also the default
instance's id, its URIs' segment and its commands' namespace), measures,
metrics and dimensions (global: the key), datasets (global: the
`v_<ext>_<name>` view), event types (global: the type string; one
descriptor per type, its highest `v` the contract version), resources
(global: `<extension>/<name>`), pages,
panels, slot mounts (`<host>.<slot>-<n>`), decorators (`decorator-<n>`),
replacements and custom components (all mounted, no global name).

The id rules each kind used to restate are the domain's: `valid_local`
(effects, advisories, pages, panels, skills, custom components),
`valid_extension_name`, `extension_namespace` (the `-`→`_` rule, used by
command namespaces, dataset and entity views, entity tables, event types
and the SDK). Rules that stay their own on purpose:
`refuse_shared_namespaces` refuses *both* extensions whose commands map
to one namespace (a namespace is one extension's);
`capability::instance_id_problem` keeps provider ids to snake_case (the
id is also a command namespace); a dataset can't take a core dataset's name
or be declared twice in one extension; the event-type "two extensions
make one namespace" check (unreachable with valid names, kept for an
invalid-named folder, which still loads).

**Discovery.** Each generation (boot and every catalog change)
restates the `contribution` table (V47, `oxplow_db::contribution_store`)
from every enabled extension's descriptors, read as `v_contribution`.
The same transaction restates `extension_listing` (V74,
`oxplow_db::catalog_listing_store`), read as `v_extension`, and
(V110) `page`, `page_answer` and `page_param` from every enabled extension's
page files (`extension_generation::page_rows`), read as `v_page`,
`v_page_answer` and `v_page_param`. (V113 dropped the old UI table,
`thread_answer` and dashboard tables.) A page's URI is `oxplow:oxplow/page?page=<extension>/<slug>`
(`page=<slug>` for an extension in the `oxplow` namespace), stored in the row.
Two kinds of listing row follow something other than a generation: those
for an extension with no deployment running (`undeployed`, `not_here`) follow
its *source on disk*. The config watcher (`config_watch.rs`) watches
`.oxplow/extensions/` and, on a change there, calls
`ExtensionActivation::restate_sources`, which rewrites only those rows
(`catalog_listing_store::restate_source_backed_tx`) from the current
sources, under the activation lock and with no activation: nothing that runs
changes. So `rm -rf` of an undeployed source drops its row at once, and a new
source is listed at once. We restate rather than read live (as
`v_extension_source` is) because a live dataset is materialized by name in the
query, and `v_extension` is a stored table other views and pages join.
Sources that `extensionSources` points outside the folder are not watched;
they show at the next activation.
The reference's "What it can declare" table is generated from
`contribution::KINDS` (each kind's `summary`) between markers in
`docs/reference/extensions.md`;
`descriptors::catalog_tests::the_reference_lists_every_kind` fails when
they drift (`OXPLOW_BLESS=1` rewrites it). The inventory and plan behind
all this are the foundation roadmap's "Normalize contribution
descriptors".

**Public imports and exports.** The pure validator is `extensions/imports.rs`;
`extensions/public_contracts.rs` normalizes manifest declarations, binds public
dataset references and withholds contributions before catalog publication.
Exports are explicit opt-in. A stable contribution kind or a legacy alias
does not make a contribution public.

- Targets are contribution URIs (`oxplow:oxplow/dataset?dataset=effort-review/reviews`). An
  import of a globally named kind (`oxplow:oxplow/command?command=acme.pr.merge`) is of
  whichever extension declares that name.
  An import has a consumer-local binding name, a positive integer contract
  version and required/optional status. Binding names are lowercase letters,
  digits, dashes and underscores, starting with a letter and ending with a
  letter or digit. Aliases and internal names never resolve a target.
- Package `version` is release SemVer; `engine` is host compatibility;
  an export's positive integer version is its individual contract. Imports
  require exact equality. A dataset/event export cannot relabel its native
  descriptor version. An export must name its owner's declared contribution.
- Export visibility is `private` by default (not importable), `experimental`
  (private consumers only), or `stable` (private/shared consumers).
  A shared producer cannot expose experimental exports; an explicitly
  private contract can stay internal to a shared package.
- Missing, disabled, inaccessible or incompatible required targets block
  the consumer. Optional equivalents remain explicitly unavailable with
  warnings. Malformed declarations, zero versions, duplicate local bindings,
  self-imports and ambiguous exports are errors even when optional. There is
  no catalog-order winner for duplicate identities.
  An optional import of a global name (a `command`, an `event_type`) that no
  extension declares is just a missing target: it degrades with the
  rest, and a report names the global name as its producer.
- A blocked producer cannot supply a binding. Required failure propagates
  to consumers to a fixed point; optional bindings become unavailable.
  Disabled consumers do not impose activation dependencies. Among otherwise
  eligible enabled candidates, reject every strongly connected cycle,
  including available optional edges, with a deterministic path and errors
  at each participating import; propagate these failures too. Remaining
  candidates activate dependencies first, with lexical tie-breaking.
- Imports express compatibility, never permissions, subscriptions or event
  conversion. Existing foreign-event subscriptions and upcasts stay intact.
  The host exposes public references and gates contributions that use an
  unavailable optional binding before catalog publication. The validator neither
  rewrites SQL nor invents a dynamic extension service/RPC surface.

The normalized validator takes all candidates (name, enabled/shared flags,
declared contribution versions, exports and imports with `DeclaredAt` sources)
and returns resolved binding states, blocked extension names, activation order
and source-located diagnostics. It is deterministic and does no I/O.

Manifest declarations are `exports: [{contribution, version, visibility}]`,
`imports: [{id, contribution, version?, optional}]` and
`import_guards: [{contribution, imports: [binding]}]`. An import's
`version` is optional: absent, whatever version the producer exports
binds; given, it must equal the export's. The binding records the version
it bound (`publicContracts.bindings[].version`). Guards name this
extension's datasets, pages, collectors, commands, effects, advisories,
skills or providers. A guard's bindings must be declared and nonempty; unknown or
duplicate names are errors. Datasets and a page's `sql` loaders can read
a public dataset with `import_ref('binding')`; tokenized calls infer a guard automatically. Strings,
comments and qualified function names are untouched. A dataset's binding becomes
an ordinary `ref('<producer>/<dataset>')`, retaining lineage and native contract
checks; a page loader's becomes its quoted published view. An
unavailable import withholds the page (and its slot mounts), and
`extension test` skips the examples that run it. SQL grants still apply.
`examples/extensions/effort-review` (producer: `exports` its `reviews`
dataset) and `examples/extensions/review-badges` (consumer: an optional
import read by its pages and a slot mount) are the working pair; the
review-badges test in `crates/oxplow-sdk/tests/examples.rs` covers available,
disabled, missing and another version. `extension check` and `extension test` print one line per binding from
`public_contracts.bindings` (`oxplow_sdk::import_lines`: available, or the
`UnavailableReason::describe`, the one wording the load warning uses
too). Withholding is reported once per cause: each unavailable optional
import's warning lists what it withheld (guarded contributions, and the
mounts, pages, panels and slots pruned with them), and
`publicContracts.withheld` records each withheld identity with the bindings
that withheld it. A mount withheld only because another extension's page
went gets its own warning. `extension test` skips an example whose target
was withheld, with that cause, instead of failing it.

The loader validates declarations and optional guards before legacy global-name
collision pruning can hide duplicate identities or let a withheld contribution
reserve a global name. Host manifest checks (including engine compatibility)
also block candidates with explicit public contracts and their required
consumers; an invalid candidate never reserves global names. Resolve imports,
withhold guarded contributions, remove mounts that
refer to absent pages/datasets/providers, withdraw their exports, and
resolve again until no contribution is removed. Repeat resolution after global
collision checks, then bind SQL and publish. Removal is monotonic within a
load; a later reload starts from the source declarations. This also withdraws
gated exports from downstream consumers without blocking independent
contributions. Stable slot identities retain their original manifest indexes.
Disabled and blocked candidates are cleared after dependency resolution.

`Extension.publicContracts` exposes declarations, withheld identities, resolved
binding states, published references and an activation index. The index orders
dependencies before consumers. Boot activation publishes runtime registries for
one catalog snapshot; the descriptor index itself remains source metadata. A
blocked extension's lookup reports its contract errors,
distinct from configuration disabling. Other contribution kinds expose their
published reference metadata; imports add no dynamic service-call surface.
Authoring checks and impact review now build a read-only candidate catalog
from fresh source declarations, replace only the candidate's matching origin,
and resolve imports and guards before compiling the overlay. This snapshot
supplies all enabled datasets and entity stubs; it does not reuse active views or
publish registries. Previously active consumers blocked by the candidate report
errors on that candidate, while unrelated existing failures are not attributed
to it. Dataset compilation also reports errors from declared transitive consumers.
Disabled packages can be inspected without changing enablement or authority.

Both impact sides resolve against current project dependencies, rather than a
complete historical dependency catalog. The `before` side reads its own raw
source even if its active contributions were cleared. Rejected project folders using
reserved bundled names remain diagnostics only and cannot block the bundled
package's public identity. Runtime generation activation is integrated; import
withdrawal and activation behavior are covered end to end by
`extension_imports_tests.rs` (compatible, mismatched, missing, optional,
cyclic, private/experimental, producer disabled, and an update that would break
a consumer; collector, event-type and command exports and imports; diamonds, three-way
and self cycles, an available optional edge closing a cycle, shared and
experimental combinations, a repaired producer bringing its consumer back, and a
guarded page pruning its slot mounts). A contract-blocked extension is cleared to `enabled: false` by the
load, so `CatalogSnapshot::admitted` holds it by `public_contracts.blocked`,
not by `enabled`.

## Namespaced configuration declarations

`configuration:` in manifest v2 declares non-secret settings as
`{version, schema, defaults}`. The version is a positive integer distinct from
package, engine and public contract versions. The JSON Schema must declare an
object root; defaults are an object and default to `{}`. Unknown declaration
keys and explicit `configuration: null` are invalid. The loader retains only
validated declarations in `Extension.configuration`; errors remain source
located. The existing top-level `config:` key is still invalid. Provider
instance configuration remains its own contract.

`extensions/config_schema.rs` compiles with an explicitly offline retriever:
no HTTP or filesystem schema retrieval, including when another dependency
enables retrieval features. Schema/default inputs are bounded to 1 MiB serialized
JSON and 64 nesting levels. Serialization counts bytes without allocating an
unbounded duplicate. Diagnostics contain a JSON pointer and a built-in keyword
or fixed error code, never library error text, enum choices, unexpected-property
lists or offending values. At most 32 violations plus a truncation marker are
returned, sorted and deduplicated.

Defaults are fragments of objects that higher layers can complete. Direct
`required` and `minProperties` errors on mergeable objects defer to final resolved
validation. Arrays replace whole values, so required fields inside their objects
are checked even in defaults. All other schema constraints apply to defaults as
written; branch constraints such as `anyOf`/`oneOf` are not solved by inventing
hypothetical user values. Authors must provide defaults satisfying those branch
constraints. Final resolved validation defers no constraints. JSON Schema
`default` annotations do not insert values; use the declaration's `defaults`.

`extensions/config.rs` supplies pure resolution: recursively merge objects in
`defaults < project < personal` order; arrays, scalars and explicit null replace
subtrees. Missing layers inherit; a present null layer fails the object-root
requirement. Empty overriding objects merge without clearing lower fields.
Reset/unset is removal of a stored override, never a magic JSON sentinel. Origins
are per effective leaf JSON pointer, with arrays atomic, empty objects as leaves
and correct `~`/`/` escaping. Replaced subtrees lose stale origins. Merge/schema
validation share a bounded byte counter and nesting check; merging avoids
redundant object clones and origin removal uses the BTreeMap's prefix range.

`ConfigurationFile::resolve` connects that resolver to the offline schema adapter:
validate partial defaults and the final resolved object using the policy above,
map only pointer/code diagnostics into layer-local issues, and return no resolved
values on failure. Invalid namespace input is omitted from its diagnostic. The
resolver caps, sorts and deduplicates issues; the schema adapter's cap remains
smaller. There is no permissive fallback, external retrieval or secret lookup.

### Private configuration services

`extension_configuration::ConfigurationHost` loads the owning extension's
manifest defaults and private Project/Personal override rows, resolves them in
that order, validates the effective schema and reports per-leaf origins.
`extension_config.read` is distinct from core app-settings `config.read`.
Native operations are `snapshot` (effective values, origins and layer stamps)
and `inspect` (stamps only, including old schema versions). Scripts declare
`needs: [extension_config.read]` and call
`scope("extension_config.read", {})`. Composers read their existing connection;
effects, collectors and provider host calls use an owned DB read per call.
Provider identity is captured at construction, as for private state. No
extension/owner selector, SQL, credentials or authority comes from settings.

A native `scope: extension_config.write`, `op: compare_exchange` takes
`schema_version` and optional `project`/`personal` objects, each containing
`expected` (exact prior stamp or null for creation) and `values` (object
fragment). The host validates the complete candidate before either CAS mutation.
Both layers can be replaced atomically for an explicit schema migration. An
untouched layer must already use the current manifest version; reads of old
versions fail closed. `inspect` supplies the old stamps needed for migration.
Version decrease is refused. Failure rolls back both layers; storage errors
abort the outer audited command transaction. The write runs no script and waits
on nothing external. Writes return stamps and omit both layer inputs from audit;
private compositions/proposals follow the state rules. `inspect` and `snapshot`
(and the stamps a write returns) carry `needs_migration: { project, personal }`:
whether a stored layer is at a schema version other than the declared one.
Reading such a layer fails closed with a `/schema_version` error, distinct
from a stale-stamp conflict (`/expected`).

**Schema migrations.** `configuration.migrations` lists steps
`{ from, to, entry }`, each a pure Starlark `transform(values) -> values` in
the extension folder (the sandbox of every other script: no I/O, no services).
At load the steps must each go forward, start at distinct versions, chain end to
start with no gap or overlap, and the last must arrive at `version`; each entry
must exist and parse (a violation is a load error, `migration_not_forward` /
`migration_duplicate` / `migration_chain` / `migration_target`). The runner
(`extension_config_migration.rs`) runs at activation inside the generation's
database transaction, for each enabled extension and each stored layer
(project, personal) behind the declared version: it chains the steps from the
stored version, checks that each returned an object, resolves the carried
layers (with the neighbouring layer) against the declared schema, and writes
each with a compare-and-swap at the new version. The schema describes only the
declared version, so it is checked once on the result, not between steps. Any
failure (a step that errors or returns a non-object, settings the schema refuses,
a stored version no step starts from or newer than declared) names the
extension and the step and rolls the whole candidate back: every layer stays at
its old version and the extension is held back exactly like one whose dataset does
not compile (`Failed::hold`: its last working version keeps serving, on the
settings that version reads; a first-time extension stays out), the problem
shown in Settings → Extensions. Fixing the step and activating again migrates.

Reset uses an empty override object at a newer revision, retaining stale-token
rejection. Disable/uninstall retain local rows for explicit recovery/reinstall;
access is refused while the owner is disabled, missing or stale.
Both scopes live in the local project's database, with no global identity lookup.
Raw secrets do not belong in defaults, schema or overrides. Secrets are host-owned
references ("Host-owned secret references"); this service resolves non-secret
settings only.

### Configuration in SQL

An extension's own SQL — its datasets, its pages' `sql` loaders and its
advisories' queries — reads its own configuration with
`config('/pointer')`, a JSON pointer into the extension's effective values
(`defaults < project < personal`, the `ConfigurationHost` resolution above):
`HAVING count(*) >= config('/struggle/repeatedEdits')`.

- **Resolved, not a function.** Like `ref()` and `source()`, `config()` is
  not a SQL function: where the SQL is compiled or run, each call is
  replaced by the literal of its value (a number as is, a string quoted, a
  boolean 1 or 0). Nothing registers a global function, so there is no `v_*`
  view or `oxplow_query_sql` call that reads configuration, and no way to name
  another extension's: the owner is fixed by where the SQL is declared
  (`ExtensionDatasets.config` for a dataset, the page's or advisory's own
  extension for those). Core SQL reads none. Settings are non-secret by
  construction; a literal in a view or a query is no more exposed than they
  are.
- **Datasets** are compiled with the values read in the activation's own
  transaction (`PreparedExtensionDatabase::replace_tx`), so the view carries
  the literal; a stored configuration that can't be resolved holds that
  extension back like a dataset that doesn't compile. **A change republishes:**
  the generation records the configuration the datasets were compiled against
  (`ConfigSignature`, beside the entity signature), the entities consumer
  checks it after each `command.executed` (`request_config_refresh`; a write
  through `extension_config.write` is one) and requests an activation when it
  differs; a changed definition refills a materialized dataset.
- **Pages and advisories** read the values stored now each time they run
  (`SqlGateway::with_extension_config`), so a change applies to the next
  run with no republish. A check (`oxplow extension check`, the effect report)
  reads no stored settings and compiles against the declared `defaults`.
- **Errors at load.** A read must name a key of the extension's own
  `configuration.schema` (under `properties`) whose `type` is a number,
  string or boolean: an unknown key, an object or array, a read in an
  extension that declares no configuration, or a pointer that isn't a quoted
  string starting with `/` is a load error at the SQL, naming the key.
- **A computed type.** A literal has no declared type to SQLite: a dataset
  column over a `config()` read declares `type: ""` (untyped), and integer
  division applies to an integer setting (write `1.0 * config(...)` to divide
  as a real).

oxplow-bundled uses it for its review-policy constants (README "Settings");
`oxplow.review_policy.read` / `.write` are its commands over the scope above.
Pinned by `extension_sql_config_tests.rs` and
`datasets::tests::config_reads_become_the_owners_literals`.

## Private state host boundary

`extension_state.rs` supplies `StateHost`, bound by the host to the primary
project, over the caller's already-owned connection. Requests select only
`scope: project|personal` and `area: state|cache`; they cannot select an extension,
project path, SQL, run identity or grants. The host requires a valid contribution,
an explicit `state.read` or `state.write` declaration, allowed live authority,
the current catalog generation/package version and an enabled descriptor. Core's
unrestricted context without a declaration is refused. Imports do not select
another namespace. Disabled, removed, stale or undeclared owners fail
with fixed diagnostics before payload access.

Native commands may declare `scope: state.read`, `op: snapshot` to read their own
namespace. This read returns null when absent or `{stamp, values}` when present;
it has read access and its result is not copied into command audit. State remains absent from
SQL datasets. Scripts may declare `needs: [state.read]` and call
`scope("state.read", {"scope": "project", "area": "state"})`; collectors, fact
collectors, effects and policies use a separate owned read for each call, with
no transaction held while their worker waits. Command composers use the routing
connection. The shared `scope` builtin accepts declared callable services
and checks the actual service on every call; declaring state access does not
imply SQL access. Service listings report declared callable services as potential
uses when a script names or aliases `scope`, rather than inferring every dynamic
call argument.

**A view of state.** Since state isn't in SQL, a view shows it through a
command that reads it and returns it as its `result` (a component
`invoke`s it; the triage example's `triage.todos.decisions`), and changes
it through a script command that reads the stamp and composes the
extension's own `state.write/compare_exchange` command with it
(`triage.todos.decide`): the read and the swap land in one transaction,
and a concurrent change refuses the second rather than losing it.

Provider `host/call` serves the same state read using the provider contribution
identity captured from an exact enabled catalog declaration at instance creation,
with the catalog generation unchanged across the instance's start,
not the invoking command's extension or a protocol-supplied owner. Calls require
a registered in-flight invocation and recheck its live authority. Candidate
checks without an exact active declaration acquire no state reader. Successful
state reads contribute to the invoking run's scope trace. An unchanged provider
instance with an old catalog generation is conservatively refused; active
generation coordination must reconcile its lifetime. Examples may supply typed
state answers; they never fall back to real state without a valid owner and
configured host. Bare CLI/fixture SQL gateways have no state reader.

The synchronous write helper accepts an existing
audited TxCtx, refuses a pure run or an unaudited/autocommit connection, and
publishes fenced CAS in a savepoint. Live checks run before and after both reads
and writes; rejected publication rolls back its state mutation. Storage errors
must abort the outer transaction. Extensions can declare `scope: state.write`,
`op: compare_exchange`; the native operation takes scope/area, expected stamp,
schema_version and a complete values object, returns only its new stamp, and
records values as an omitted-byte count. Reset uses an empty values object at a
new revision; it does not delete/recreate the namespace. No inverse stores old
private values. Nested compositions inherit private-input recording and child
metadata is redacted. Calls that require confirmation cannot retain these inputs
in a public proposal; a person must invoke them directly with confirmation.
State has no migration runner (configuration's is above), and lifecycle cleanup
beyond retaining rows is unbuilt. Raw secrets do not belong in state payloads.

Current-generation checks use the filesystem catalog, including its stat walk;
they are conservative on any catalog change. This does not pin DB/runtime
registries or keep state access usable under a retained last-good handler after
a rejected candidate. The coherent activation coordinator must adopt the host
boundary with its active-generation identity. Working-path tests cover the
stores, resolution and schema adapters, the secret reference and host, and the
command-bus path end to end (`extension_private_state_tests.rs`: write/read,
owner isolation, disable keeps rows, precedence, value-free refusals, restart
over a database file); adversarial and concurrency coverage is still to do.

> **Status: built.**
> - **Current:**
>   - loading project extensions and their pages from
>     `.oxplow/extensions/` (see "What works today" below);
>   - running and validating pages over IPC and MCP;
>   - the `oxplow-extension` agent skill;
>   - the launcher's page sections (a page's `category`);
>   - sharing: `oxplow.extension.share` / `unshare` move a source in or out
>     of git, and a teammate deploys it in their checkout ("Sources and
>     deployments": boot deploys nothing);
>   - Explore Data (oxplow-foundation's custom page, with Save as Page);
>   - **bundled extensions** (compiled in, read-only, reserved names).
>     `oxplow-bundled` holds the effort review packet and the analytics
>     pages; core works with it disabled;
>   - `exec` **sources** that bring external records in as entities. The
>     mechanics and decisions are in
>     [semantic-layer.md](./semantic-layer.md) → "User and extension
>     sources", and a tested example is in `examples/extensions/github/`;
>   - **slots** on pages (`oxplow-foundation/diff-view.sections`,
>     `commit.sections`, `uncommitted-changes.sections`,
>     `work-item.sections`, `tasks.sections`, `new-session.sections`,
>     `diff.file-header`), **advisories**, per-project **disabling**;
>   - page alerts, panels, and extension-declared measures, metrics,
>     dimensions and fact collectors (see "Contributing metrics");
>   - whole-page and row actions;
>   - manifest v2 with `intent` and the stable/experimental split, the
>     catalog cache and the SDK: `oxplow extension new|check|test`
>     ("The SDK").
>
> When a piece ships, move it from "target" to "current" here, in the
> same commit.

## What works today

Code: `crates/oxplow-app/src/extension/` (the extension machinery: the
loader `extensions.rs`, activation, catalog, deploy, jobs, resources),
`crates/oxplow-app/src/pages/` (the page schema and spec types — `Viz`,
`VizConfig`, `ViewContext` in `spec.rs` —, runner, open runs, actions,
checks and text renderings), `crates/oxplow-rpc/src/commands/extensions.rs`
and `pages.rs` (IPC), and the page tools in `crates/oxplow-mcp/src/lib.rs`.
The authoring format, as agents learn it, is in the `oxplow-extension`
skill's `references/` (`page-format.md`, `loaders.md`, `view-nodes.md`);
the user guide is `docs/guide/pages.md`.

- **Files.**
  - `.oxplow/extensions/<name>/extension.yaml` is a **manifest v2**
    (`manifest: 2`, `name` = the folder, `intent`, the
    contribution kinds; see "`extension.yaml`" below). It has no `pages:`
    key: pages are files.
  - `pages/<slug>.yaml` is one page: metadata (`title`, `description`,
    `label`, `icon`, `category`, `keywords`, `params`, `answers`, `actions`,
    `alert`, `badge`, `needs`, `for` / `accepts` / `default` / `pinned`,
    `comments`, `refresh`, `hidden`, `tool`), `loaders:` (named loaders: `sql`,
    `command`, `page`, `rows`) and `view:` (a tree of nodes, with an optional
    `rail:`). Details: [pages-and-tabs.md](./pages-and-tabs.md) "Page files".
  - Unknown keys are errors, so typos surface instead of being ignored.
- **Ids.** A page id is `<extension>/<slug>`.
- **Reading.** What the app shows is the **live set**: each extension's
  active deployment ("Sources and deployments"), whichever stream is open:
  listings, pages, slots and panels, custom components, prompts,
  advisories, `oxplow_show_page` and `oxplow_get_open_page`. A stream's
  `stream_id` only scopes the data a page reads (`:stream_id` /
  `:thread_id`). Reads go through `Services.extension_catalog`
  (`crates/oxplow-app/src/extension/extension_catalog.rs`). `find_page`, the page
  tools and the advisory/metric/source readers take the catalog; the
  authoring checks (`oxplow_validate_extension`, preview) load the source
  instead. `ExtensionCatalog::snapshot` returns immutable declarations with
  their generation identity; `runtime_snapshot` is the last committed
  catalog plus immediate disable/removal revocation. The raw `snapshot` is
  the authoring/candidate view, with a staged change in it.
- **Params.** Values are bound as `:name`. Supplied values override
  defaults; for a param the viewer's context supplies (`stream_id`,
  `thread_id`) the context comes second. An unknown param name is an
  `Invalid` error, so a typo can't silently fall back to a default. A param
  with no default and no value binds NULL. `head_sha`, when a page declares
  it, is the viewed stream's HEAD: the run resolves it, and a change to
  `v_stream` (a commit, a branch switch) runs again the loaders that bind it.
- **Errors never cascade.** A missing or bad `extension.yaml`, or a folder
  name mismatch, marks that extension's `errors` and skips its pages. A
  bad page file is listed in `errors` and skipped; the other pages still
  load. A loader that fails shows its error and the rest of the page
  still shows.
- **Checks.** `oxplow_validate_extension` and `oxplow extension check` run
  the load checks and then dry-run every page (`pages::checks::dry_run`):
  each `sql` loader runs once with its params' defaults, earlier loaders
  as tables. They report SQL errors and viz nodes whose `columns` or role
  blocks (`link.from`, `chart.x`, `tree.parent`, …) name a column the
  loader doesn't return. `command` and `page` loaders do not run (a check
  acts as no one), so a node they feed is not column-checked.
  `pages::checks::layout_warnings` adds warnings, never errors: more than
  six filters, a `choice` with more than twelve options, a description
  over 200 characters, more than eight sections or eight table columns, a
  param nothing reads. A page in the launcher (a `category`, not
  `hidden`) is also warned, at the param's line, for a param nothing
  fills in when it opens there (`launcher_unfilled_params`: no `default`,
  `options_from`, filter or selection role, and not the viewer's
  `stream_id`, `thread_id` or `head_sha`). A collector's `input` query is
  dry-run as a page's loaders and an advisory's query are, so one that
  reads a physical table (or doesn't compile) is an error at the
  collector's line. Load checks also cover an unknown `icon`, a page named by `options_from`,
  `expand` or a form field's `from` that is not in `pages/`, a `component`
  node or `viz: custom` naming a component the extension doesn't declare,
  a `replaceable` target that is not a replaceable component
  (`v_replaceable`), the renderer rules (below), and the params a panel gives
  its pages. A slot mount whose host page is not loaded, or has no such
  slot, is a warning on the mounting extension, and it shows nowhere.
- **Surfaces.**
  - IPC: `open_page` (starts a run a window keeps open), `run_page`,
    `get_page`, `page_text`, `page_form`, `submit_page_form`,
    `run_page_action`. MCP: `oxplow_run_page`,
    `oxplow_find_page`, `oxplow_show_page`, `oxplow_run_page_action`,
    `oxplow_validate_extension`; an agent reads a
    page's definition with `oxplow_read`
    (`oxplow:oxplow/page?page=<ext>/<slug>`).
  - An agent lists pages, extensions and what each contributes with
    datasets, not tools: `v_page`, `v_page_answer`, `v_page_param`,
    `v_extension` and `v_contribution` (restated with each extension
    generation from the active catalog, the live set; an extension that
    fails to load holds its generation back, so it is not a row —
    `oxplow_validate_extension` says why). The UI's `list_extensions` stays
    IPC.
  - **Over MCP these are the authoring tools** and read the caller's
    **source**: an omitted `stream_id` is the caller's own stream
    (its header, else its thread's; the primary only for an anonymous
    caller), so an agent in a worktree can check the extension it just
    wrote before it deploys it — the same for
    `oxplow_run_page` with `draft: true` and `oxplow_ensure_change`.
  - **An agent tests an extension with the CLI, not a tool:**
    `$OXPLOW_BIN extension test <name> [--junit <file>]` from its own
    checkout (a worktree's draft as it is). `OXPLOW_BIN` is set for every
    agent session to the binary that launched it (the daemon's, which has
    the `extension` subcommands too, so it works on a remote host). The
    throwaway runs in that process, never in the app: a hang, a crash or
    its memory are the test's. It is a test run like any other — a
    `runPatterns` entry and a report collector for `--junit` put it in the
    Tests panel — and its programs see the agent's own environment, which
    the agent's shell already has.
    `oxplow_preview_collector` reads that stream's files and data but runs
    the deployed entity collector. `oxplow.extension.deploy` names no
    stream: it deploys the main stream's copy.
    `site_search` is the exception by design: omitted, it searches every
    stream; it indexes pages as well as the wiki.
  - **Over IPC (the UI)** they show the live set: `list_extensions`,
    `get_page`, `load_component` and `invoke_component_command` take no
    stream; `open_page`, `run_page` and `run_page_action` take one only for
    the page's data context. Checking an extension is the agent's
    `oxplow_validate_extension` (and the CLI's `check`), not an IPC read.
  - `oxplow_get_open_page` (an agent's) plus `report_open_page` (UI) are
    current; see "Agents: the MCP surface".
    Agents write page files with their normal Edit tool, under the filing
    guard, taught by the `oxplow-extension` skill.
- **UI.**
  - The desktop draws every page the same way: the page host
    (pages-and-tabs.md "The desktop host") runs the page, `PageTree` draws
    its node tree, and its viz nodes use the viz components
    (`apps/desktop/src/viz/`, entry `VizView.tsx`; the directory
    keeps the old name). A viz node draws a `table`, `list`, `number`
    (through `formatMetricValue`), `markdown` and the rest below. Linked cells go
    through `RouteLink`, so plain-click navigates in the tab.
  - The rail is the page's one place for controls (`PageRail`,
    usability.md "A page's controls are in its rail"): its filter params
    as `ParamRow`s (label beside value, click to edit in place or in a
    popover), its selection params as the picked row, and About this
    page (`origin`). A row shows what it applied at once, before the run
    echoes it. A row click on a column that `select`s a param sets it
    (`onSetParam`). Copy, Add to Context and the whole-page actions are
    handed to the nav bar: in its ☰, except a whole-page action the
    manifest marks `prominent: true`, which stays a button there (several
    may; a row, group, drag or edit action can't be — `parse_actions`). In
    the ☰ the page's actions sort by label; `submenu: <name>` puts a page
    action in that submenu, shared with the commands whose `ui.group` is the
    name (a prominent one can't be in one). Any action may name an `icon`
    from `extension_resources::ICONS` (the desktop's `src/icons.ts`): alone
    on its button in the bar, beside its label in a menu.
    **A page is a function of its tab entry** (`tabs/session.ts`): its
    params are its URI's (`oxplow:oxplow/page?page=<ext>/<slug>&k=v#f=v`): the
    `?` holds its `resource` params, the `#` its filters and picks. A change
    of a filter or pick goes back through `PageNavigation.setViewParams`
    with the values that differ from the defaults (`changedParams`, by
    type) — a filter replaces the entry, a selection pushes one — and a
    change of a `resource` param navigates to another resource
    (`withResourceParams`). So a reopened tab, a restart, a bookmark and
    Back show the same filters and the same pick. Outside a tab it keeps its
    params itself.
  - A run failure shows the error inline, with a nudge to ask an agent. A
    page whose `needs` are not met (a capability that is not active) runs no
    loader: its `PageRun.inactive` says what it needs, and the page shows
    that instead of failing.
  - Refresh is opt-in: a page offers it only when it says `refresh: true`.
    A page re-runs on its own when the datasets it read change, so Refresh is
    for one that reads outside oxplow (an integration's list, a file on
    disk).
  - The page re-runs when a dataset it read changed (`datasetsChanged`), when
    facts landed for a measure its `metric_grid()` read
    (`metricSamplesChanged`), when the live set changes, and after a
    reconnect (`onRemoteReconnect`): events sent while the daemon was
    unreachable never arrive, so anything may have changed. The daemon
    keeps each open run (`pages/open_runs.rs`) and tells the window which
    loaders changed (`PageRunChanged`); `useRerunOnChange` in
    `src/viz/rerun.ts` is the window's side.
  - The Cmd+P launcher re-reads pages (the live set's) every time it
    opens and lists them under their `category` (a page with none, or
    `hidden: true`, is not listed), merged into the static directory by
    category order (`mergeDirectory`).
  - `alert:` marks when a page needs attention: `{ min_rows: N }` (the
    watched loader returned at least N rows) or `{ column: c, above: X }` /
    `below: X` (the first row's value), with an optional `label`. The loader
    watches the `badge:` loader, else the first loader. Loading
    refuses an alert with both or neither condition. Every `PageRun` (IPC
    and MCP) carries `alert: {firing, count, value, message}`; the message
    is `label: count`, `N rows`, or `label: value`. `badge: { loader,
    column? }` is a count on the page's nav entry (the loader's first row's
    value, else its row count; Backlog's ready count).
  - **Pages** (`Extension.page_files`, `pages::schema::Page`; checked at
    load) — every page is a page file, oxplow's own included. Its tab id is
    the URI `oxplow:oxplow/page?page=<extension>/<slug>[&params]`, or
    `page=<slug>` for the extension whose `namespace:` is `oxplow`
    (oxplow-foundation: its pages are core-named as its commands are,
    `CORE_NAMESPACE`); `pageRef` carries it. `category` lists it in the
    launcher (none: a page only opened from elsewhere — a diff, a form);
    `keywords` are more words the launcher matches. `title` is its name (the
    launcher's, Open with's); `label` is what its tab says when that
    isn't the name, `{{param.<name>}}` filled from what it shows
    (`label: "{{param.path}}"`; `pages/renderers.ts` `tabLabel`; filled
    to nothing, the title) and `{{row.<column>}}` from the first row of the
    page's first loader. The tab says the page's title until that loader
    has run (`PageHost`: `tabLabel` over `labelRow` → `usePageTitle`), then
    the filled label; a loader that fails keeps the title. Either may take
    filters: `{{param.sha|short}}` (the first 7 characters), `|basename` (the
    last path segment) and `|url` (a web page's host and path, cut to 40),
    refused if unknown. A renderer's label names params of what it renders
    (else a load error), a route's its `params`. A page's `icon` is one of
    the resource icons (`ICONS`; an unknown one is a load error); the tab
    strip, launcher, Go To lists and backlinks draw it (`PageKindIcon` by
    the page's id). A **custom page** is a page whose `view:` is one
    `component` node: the extension's own `custom_components` entry fills the
    page body under core's nav bar, told the page's view params, history
    state, stream and thread in `init.page`, setting its title and actions as
    data core draws ("Custom components"). How the desktop draws a page:
    pages-and-tabs.md "Declared pages".
    **A page with `for` is a renderer** of a resource; one without is a
    route (`oxplow:oxplow/page?page=…`). A URI opens as itself and the
    desktop picks among its renderers (pages-and-tabs.md "Renderers").
    The rules, checked at load (`renderer_problem`, after `resources:` is
    parsed):
    - `for: <resource> | [<resource>, …]` names a resource of the
      extension's own, one of oxplow-foundation's (`oxplow/*`), another
      extension's, or the scheme `https` / `http` (a web page).
    - `accepts: { <param>: present | absent | <pattern> }` keys are the
      resource's params (any key when the resource takes `additional`
      params); a value is `present`, `absent` or an anchored id pattern
      (the `param_pattern` subset). A renderer shows a URI only when every
      rule holds.
    - `accepts: { content: <loader> }` (the key `content`, not a param)
      names one of the page's own loaders (or the system's `content`); the
      page is offered for a URI only if that loader yields a row. It is a
      load error if the name isn't a loader of the page, if it (or what
      it reads) is a `page` loader, or on a page with no `for`. The daemon
      runs only that loader's dependency chain (`sql`, `rows`, a read-only
      `command`, `content`) with 2 s and 1 MiB; a failure is not offered.
      Which pages draw a URI is `pages/renderers.rs`, asked by
      `oxplow.page.renderers { uri }` (pages-and-tabs.md "Renderers").
    - `default: true` is only for a resource the extension declares (or,
      for oxplow's namespace, a scheme), and one page per resource. An
      extension that renders another's resource adds a renderer to pick
      ("Open with"), never the default.
    - `pinned: true` makes its tabs pinned.
    - `comments: true` makes the page's text take comments: its URI (its
      tab id) is their target, the nav bar gets the comment navigator, and
      a selection in its text starts one — plain text and the markdown
      viewer through the app's DOM layer (the body carries `data-ref`),
      the markdown editor and a `code` viz through their own layers
      (usability.md "Comments are a page's"). Comments name one URI, so
      `true` is the whole declaration.
    - `default`, `pinned` and `accepts` without `for` are a load error.
    - A page with `for` takes no `category`, `hidden`, `keywords` or `badge`,
      declares `params: [{ name: uri }]`, and is handed the `content` loader
      (above, Metadata and Loaders).
    - A renderer is run with the params of the URI it draws: a param it
      declares that the URI carries takes the URI's value (supplied → URI
      → context → default), so a `path` param of an `oxplow/dir` renderer
      is the URI's `path`. Its resource params besides `uri`, `stream_id`
      and `thread_id` must be params of each resource it renders (a load
      error otherwise: it would never get them); its filters ride in the
      `#` as any page's. Its rail doesn't draw `uri` (the page is the
      URI), and another value of a URI param opens that URI
      (`withRenderedParams`, pushed).
  - **Panels**: `panels: [{ id, title, icon?, scope: project | stream |
    thread, body, badge?, open?, collapsed?, count?, dock? }]` put a page in
    the left nav (with `dock: true` it goes in the bottom dock instead: an
    icon with the count as its badge that opens the body above the bar,
    and the rail does not draw it). `body`, `badge`, `collapsed` and
    `count` name **pages of the same extension, by slug**. `open`, a
    `page` URI, is the page its header opens — a core page it summarizes,
    as Comments opens the inbox; absent, the body page. `collapsed` is a
    page shown compact while the panel is collapsed, as its summary
    (absent, a collapsed panel is just its header). `count` is a page whose
    first cell (else row count) is the header's count, raising no alert; with
    a `badge` too, the count page's number shows and the badge still feeds
    Alerts (`panelCount`). (`Extension.panels`, `ExtensionPanel`; checked at
    load in `parse_panels`: a valid, unique id; pages that exist — body,
    badge, collapsed, count — a badge page with a `badge` or an `alert`,
    and a `stream` / `thread` scope's pages declaring `stream_id` /
    `thread_id`). The nav binds those itself (`panelParams`: the stream's and
    thread's row ids for the stream and thread it's shown for, re-run when
    they change) rather than leaving the backend to infer the thread from
    the selection, and runs every panel's pages from one owner
    (`useExtensionPanelRuns`), each distinct page once per refresh whatever
    roles it plays. The body renders compact; the badge's alert count shows
    on the panel, and the **Alerts** panel and page (the dock's bell) list
    every firing badge. oxplow-bundled's Waiting on You is a panel whose
    badge is its own page. A page dropped by `public_contracts` takes the
    panel parts that name it with it.
  - **Actions are commands**: `actions:` on a page declares
    `{ id, label, command, input?, … }` — a button in the nav bar or its ☰
    menu; on a viz node, with `row: true`, an item in each row's menu —
    every row of every row component (table, list, tree, timeline, detail,
    steps, hunks, board) is focusable and opens it from a right-click or the
    Menu key / Shift+F10. A command that asks shows `CommandConfirm` from
    the page itself, above the body, so it asks wherever the row is, not only
    where the toolbar is shown. An embed node's page actions run on the
    **embedded run**: pressing one on an embedded page acts on that page's
    loaders (`<embed id>/<node id>` names the node).
    - `input` is the command's input. A string that is exactly
      `{{param.<name>}}` or `{{row.<column>}}` becomes that value, typed
      (a number stays a number); a string containing them has them spliced
      in as text. `{{param.<name>.id}}` is the key of a URI param, for a
      field that takes a bare id. The values go into the command's input,
      never into SQL. One tokenizer reads them (`oxplow_domain::template` —
      every template's, a command's `ui.input` and `open_after` too), for
      load-time validation and run-time binding alike, so the two can't
      disagree. At load, a `{{param.x}}` must name a declared param and
      `{{row.x}}` needs `row: true` — or a node that holds one row (detail,
      form, markdown), where an action may name `{{row.x}}` and `when:` too
      and means that row; a whole-page action's `{{row.x}}` means the first
      loader's one row. `oxplow_validate_extension` checks `{{row.x}}` and
      `when` against the loader's columns; the command name must be
      well-formed. Any other shape is a field error.
    - **An action over a one-row result** (`PageAction::one_row`) is
      derived at load, not declared: an action without `row: true` that
      names `when:` or `{{row.*}}`, on a page or node that holds one row. The
      server reads the row itself (`pages::actions::page_row`, and the node's
      row for a viz node): it refuses unless the loader has exactly one row
      ("its first loader has N rows") and refuses a `row` the caller passes,
      then binds and checks `when` as for a row action. A person's press, a
      form's prefill (a Rename… starting from the name) and
      `oxplow_run_page_action` are the one path. The window shows such a
      button only while the result has exactly one row and `when` holds
      (`offeredWholePage`, the one predicate the toolbar and the page's
      hand-up share).
    - **Drag, bulk and key actions.** `drag: true` makes a row action
      what a drop runs (`rowInteract.ts`, `RowsBody`): a row the action is
      offered on (`when`) drags (`VIZ_ROW_DRAG_MIME`, taken only by the view
      it came from; a marked row carries every marked row), and where it
      lands binds `{{before.*}}` (the row below the drop), `{{after.*}}`
      (the row above), `{{onto.*}}` (the row dropped on: its middle third,
      offered only when some drag action names it), `{{group.value}}`
      (the group a drop lands in: among its rows, on its heading, on a board
      column) and `{{order.*}}`: that column of **every row of the result**
      (not only the drawn ones) in its order after the drop, the dragged
      rows moved before the row below, else after the row above, else to
      the end (`withOrder`; a drop onto a row moves nothing), so a command
      that takes a whole new order gets one. `{{order.*}}` binds a list, so
      it is only a field's whole value (spliced into text or put in a list
      it is an error), and a drop always has one (`Drop.order`; without it
      the field is an error, never left out). A field that is exactly one of
      the others with nothing there is **left out** of the input (a drop at a
      list's end has no row below), so a command's optional field stays
      absent; one spliced into text, or in a list, with nothing there is an
      error. Every drop scope needs `drag: true`, and its column must be in
      the result. Several drag actions may be declared: a drop runs the first
      the row is offered whose places are all there (one naming the group
      needs a group; one naming `onto` needs a row dropped on; one naming the
      order needs it). Rows dropped together run it once each, in drawing
      order, each after the one before it. The press carries the `drop`
      (`viz_actions::Drop`); a drop on another action, or a drag action
      pressed without one, is `Invalid` at `/drop`. `bulk: true` puts a row
      action in the **marks bar** (which stands in for the page's buttons
      while rows are marked, the page's nav-bar actions too): it runs once
      per marked row it is offered, in order, asking once and toasting once.
      `key: <k>` (one character, or `Delete` / `Backspace`; never one the
      row list keeps — Enter, Escape, Space, Tab, the arrows, Home, End,
      Page Up/Down, the menu key — nor one taken twice in a node) runs a row
      action on the cursor row, or on the marked rows when it is `bulk`. The
      marks themselves are the kit's (usability.md "Rows: marks, drags and
      keys"), kept per node in the entry's history state.
    - **Edit actions.** `edit: true` makes an action the Save of a
      `markdown` viz's editor: a form over the viz's first row (`row` and
      `form` implied). Its **edited field** is the one input field bound
      exactly to a `{{row.<column>}}` and not `fixed`
      (`PageAction.edit_field`, `extensions::edit_column`); Save runs it
      with `{ <field>: <the text> }` as the form's input, laid over the
      bound input as any form's is. The viz must name that column as its
      text (`markdown: { text: <column> }`). Refused at load: no such
      field or several ("fix all but one"), `edit` with `drag`, `bulk`,
      `key`, `group` or `primary`, two edit actions on a node, an edit
      action off a `markdown` viz, a `markdown` text other than the one
      it saves.
    - **`when: <column>`** (row actions only): the action is offered only on
      rows where that column is truthy (not null, 0, `""` or false) — the
      row menu leaves it out elsewhere (`actionOffered`), and a run on such
      a row is refused. `oxplow_validate_extension` checks the column is in
      the result. The query computes the condition (`can_enable`).
    - **A page grants no power.** `pages::actions::run_page_action` runs the
      command through the bus as `Actor::Page { page_id, on_behalf_of }`
      (`page_id` is `<extension>/<page>`, with `#<node>` for a node's
      action) — `Human` from the UI (`run_page_action` RPC), the calling
      agent from MCP `oxplow_run_page_action` (which never confirms). The
      bus applies the agent policy through `on_behalf_of`. A command that
      asks (`confirm`) asks only a person; a page action an agent pressed
      runs without asking. The audit records the page's URI as its source
      (V114 added the `page` actor kind).
    - **The command runs after the read.** An action, a form submit and
      a component's invoke are resolved under one pinned catalog
      generation (`viz_actions::Prepared`: the page, its `when`, the
      bound input, the bundle's commands), and the command runs once
      that pin is released. Activation waits for every reader of the
      generation it replaces, so a command that activates (the
      Extensions page's Deploy, Disable, Remove) would wait on its own
      pin and is refused with "activation cannot wait on its own
      reader".
    - A command that asks (`NEEDS_CONFIRMATION`, now kept on the thrown
      `IpcCallError`'s `code`) shows a confirm strip in the toolbar: Run
      (focused, Enter) or Cancel (Escape).
    - **Every page has Copy** (its text rendering, the `page_text` RPC, from
      `viz_text.rs` and `pages::text`) **and Add to Context**
      (`[oxplow page <id> …]` into the agent's input, never sent) —
      kit affordances, not declared. A compact strip (a panel body, an
      embed) shows no toolbar.
    - Running a collector is the command **`oxplow.collector.sync { collector }`**
      (External; `collector_runner::CollectorRunner::sync`,
      which the scheduler uses too). It **never grants**: what a collector
      may reach is its extension's `permissions:`, approved at deploy
      ("Permissions"). An agent's `run_command oxplow.collector.sync` runs it as the
      agent.
    - UI: the row menu and buttons in `VizView.tsx`, pressed
      on the page run (`run_page_action`); logic in `viz/vizActions.ts`
      (`performVizAction`, `rowRecord`). Copy is the page host's
      (`page_text`), Add to Context the ☰'s.
    - The GitHub example's PR page has a Sync PRs action
      (`command: oxplow.collector.sync`).
  - **Viz** (`VizView.tsx`; the `Viz` enum and `VizConfig` in
    `pages/spec.rs`): `table`, `list`, `number`, `markdown`, `bar`
    (`DailyBarChart`), `line` (`components/charts/TrendChart`, one chart per
    `chart.series`), `treemap` (two-level `components/charts/squarify`),
    `board`, and the structure components: `tree` (collapsible nesting by
    `tree.parent`; orphans are roots, a cycle is cut; `lazy` and `branch`
    read one level per param value), `timeline` (oldest first, linked
    through `timeline.uri`), `detail` (the first row as label/value pairs),
    `steps` (a checklist by `steps.status`: done, active, failed, else
    pending) and `hunks` (each row's file diffed between `hunks.from` and
    `hunks.to` in the diff viewer, `DiffPane`, one expanded at a time).
    The full-height ones (`Viz::fills`) are `code`, `diff`, `editor`,
    `terminal` and `embed`; `approvals` is core's consent list; `custom` is
    an extension's sandboxed component. **`form`**: `form: { command,
    defaults? }` renders the command's `input_schema` as a `SchemaForm`, its
    fields starting from `defaults` (placeholders bound like an action's)
    under the loader's first row — a form needs no `from`
    (RPC `page_form` returns the spec and those values). Submitting (RPC
    `submit_page_form`, `pages::actions::submit_page_form`) runs the command
    as the page acting for the person, the defaults under what they
    entered; a command that asks shows `CommandConfirm`. A form's text
    rendering names its command (an agent runs the command itself). The
    loader drops a page whose viz node lacks the column roles its viz
    needs, naming the block (`chart`, `tree`, `timeline`, `steps`,
    `hunks`), and `oxplow_validate_extension` checks every named column
    exists in the loader's result (`VizConfig::role_columns`). Pure data
    shaping lives in `vizModel.ts` (`barRows`, `lineSeries`, `treemapItems`,
    `treeNodes`, `timelineEntries`, `stepItems`, `hunkRows`).
- **Sharing.** There are three levels:
  - **Yourself.** A source is personal (`.oxplow/.gitignore` ignores it). It
    runs after `oxplow.extension.deploy`.
  - **Your team.** `oxplow.extension.share { name }` un-ignores the source;
    the person commits it. A teammate deploys it in their checkout (Settings
    → Extensions lists it `undeployed` with Deploy): boot deploys nothing.
  - **Someone else's.** The interim way: clone it into
    `.oxplow/extensions/<name>/` (or point `extensionSources` at it), list
    it, deploy it.
  - A teammate's extension runs nothing until a person deploys it here.
    An exec source, a Starlark one that calls a model (`ai_*`) and
    advisories need `permissions:` the manifest declares, and a deploy
    that adds a permission asks a person first ("Permissions"); a
    deployment is immutable, so what runs is what was deployed (see
    [architecture.md](./architecture.md) → "A repo's config never runs a
    program without consent").
- **Explore Data: oxplow-foundation's data app.** Deliberately simple,
  and the proof that an extension ships a data app: a **page with a
  component node** (`pages/explore-data.yaml`: `view: { component: { id:
  explore-data } }`, the component declared under `custom_components:`,
  its source in `extensions/oxplow-foundation/src/explore-data/`, built
  with the Timeline by foundation's `package.json`). Core draws what the
  kit draws; the component holds only the editor and the pickers.
  - **The catalog** is the page's **rail**: a `rail:` node in the same
    page file (a `list` over `v_dataset`, oxplow's own first, its `view`
    column selecting the `dataset` param, the picked row emphasized). A
    pick pushes `#dataset=` (Back un-picks) and the component hears it.
  - **The component** (`main.ts`, its pure half `dataset.ts`, tested from
    the desktop as `viz/exploreDataset.test.ts`): the picked dataset's
    documented columns (`v_field`) and lineage (`v_dataset_lineage`:
    the datasets it reads, links that pick them, the tables it reads, the
    datasets that read it); the SQL box (a pick writes `SELECT * … LIMIT
    50`; any read-only query, metrics included — `SELECT bucket,
    MEASURE('<key>') FROM metric_grid('week')` — Cmd/Ctrl+Enter runs
    it); **Show as** (`table`, `list`, `number`, `markdown`, `bar`,
    `line`, `treemap`), a chart's columns picked from the result
    (`chartDefaults`, then a select per role); **Chart a metric** seeds
    the explorer's metric query (`metricTemplate`: `metricSeriesSql` as a
    line) and, while the SQL is still it, **Slice By** regenerates it with
    a dimension (`v_dimension`, `sliceTemplate`); edited SQL is never
    rewritten. Its view is the tab's `#` (`dataset`, `sql`, `viz`, `chart`,
    `template`), the unrun draft its history state; it ignores the echoes
    of its own writes, so only a real move (Back) redraws. "Ask your agent
    to build one…" invokes `oxplow.page.draft_with_agent`, the one command
    it declares.
  - **The result** is core's: the component hands the page an inline
    page (`showView`, protocol 12: its query as one `sql` loader, its viz
    over it — `specOf` in `src/explore-data/dataset.ts`), the page opens it
    (`open_page { specYaml }`) and draws it under the frame with
    `PageTree` (`pages/PageComponent.tsx`), live, with **Save
    as Page** (`pages/SaveViewForm.tsx`: `oxplow.extension.write_page {
    extension, page, spec, deploy: true }`, then opens the page; a saved
    page pins to a dashboard as an embed). A refused spec answers the
    component `INVALID` with its field, and it shows the reason under the
    SQL box.
  - **No raw tables.** The React page's Raw tables switch read physical
    tables over IPC; a component reads datasets only, as every page does.
    Raw reads stay the agent's `oxplow_query_sql`.
  - **What an agent shows.** An agent answers with a page rather than
    pasting rows: MCP **`oxplow_show_page { page | spec, params? }`** runs
    `oxplow.page.show`. A `spec` is an inline page of the agent's own
    (read-only, no actions or forms). It opens beside the conversation
    as a page file on `oxplow:temp` (`oxplow:oxplow/page?page=temp/<id>`) and the call returns the page's
    text. The person saves it with **Save to wiki** or **Save as Page**
    (`oxplow.extension.write_page`). Nothing else is stored: there are no
    thread answers and no Keep This. See pages-and-tabs.md "Showing a
    page".
  - The launcher's **Data** category holds Explore Data, Metrics
    and Dashboards.
- **Sources (collectors).** Declared under `collectors:` in `extension.yaml` (v1: `sources:`) and parsed
  into `Extension.sources`. A bad source is reported in `errors` without
  hiding the extension's pages. Settings → Data shows each source
  (runtime, schedule, row counts or failure, Sync Now);
  Settings → Extensions keeps each extension's source credentials.
- **Settings shows what `extension check` finds.** Loading can't
  catch a page whose SQL fails (it never runs the SQL), so Settings →
  Extensions runs `oxplow_validate_extension` — the same dry run `oxplow extension
  check` prints — for each enabled project extension, on open and on
  `extensionsChanged`, and lists its errors with the load errors, once
  each. Bundled extensions are checked where they're built.
- **Bundled extensions.** There are two. `oxplow-foundation` is required: oxplow's own
  commands, AI providers and harness implementations, the default work list,
  effort policy, snapshots and wiki, and the
  measures, dimensions and metrics oxplow records itself (tokens, efforts, tests,
  coverage, analysis, work items: its `measures:` / `dimensions:` / `metrics:`,
  scope `extension:oxplow-foundation`; the producers' Rust only records the facts).
  `oxplow-bundled` holds the
  review packet, the analytics pages and the agent's advisories in one
  extension, because the point is building oxplow on its own extension
  platform, not letting people pick between its features. It can still be
  disabled like any other. `oxplow-bundled` also holds the code metrics measured from each snapshot's
  tree (TODO markers, function count, complexity and length, doc coverage,
  duplicated lines and the per-language idiom counts) with their collectors,
  metrics and the measures only they write (`oxplow.todo`, `oxplow.ast_hit`,
  `oxplow.duplicate_lines`, `oxplow.doc_coverage`); foundation has none of
  them. Complexity, function length and parameter count stay foundation's
  measures: core's change analysis records them too. Both share the `oxplow`
  namespace, so a script or metric in one names the other's `oxplow.*`
  measures without a warning.
  - Their sources live in the repo at `extensions/<name>/`, and they
    ship as described under "Shipped deployments" below.
  - `origin` is `bundled` and their path is `bundled:<name>`.
  - Their names are reserved:
    - a project folder with that name is listed with an error and never
      shadows the bundled one;
    - `oxplow.extension.deploy` of the name resolves to the shipped deployment;
    - `extensions::writable_extension_dir` (the check behind
      `oxplow.extension.write_page`) refuses to write into a bundled
      extension.
- **Slots (see "Slots" below).**
  - `extension.yaml` declares `ui: { slots: [{slot, page, at?, title?}] }`
    (v1: top-level `slots`). A slot is a `slot` node in a page, named
    `<extension>/<page>.<name>` (`oxplow-foundation/diff-view.sections`).
    A mount whose page is not in `pages/`, or whose name is not that shape,
    is a load error. A mount whose host page is not loaded, or has no slot
    of that name, is a warning ("`x` shows nowhere"). Loaded as
    `Extension.ui.page_slots`.
  - The mounted page runs as an embed: its params come by name from the
    host's params, else from the first row of the host's loaders. So the
    changes page (`diff-view`) offers `change_id` and `effort_id` from its
    loader row; the work item page offers `uri`; the diff page offers
    `path` in its `file-header` slot (bundled's co-change page mounts
    there). The host draws each mounted page as a section titled by its
    page.
  - A `stack` folds titled sections that have nothing to show into one
    closing line (see "A page of sections leads with what matters").
- **Change links.** `diff-at` (`from` path, optional `line`, `base` and
  `head` columns holding the change's labels; join `v_change`) opens that
  file's diff between the two sides (commit shas / `HEAD` → git refs,
  `working tree` → disk, `snapshot N` → that snapshot, so an effort's
  change links too). `compare` takes
  a `path:start-end|peer:start-end` value (build it in SQL) plus an
  optional `head` version column and opens the side-by-side
  `duplicate-block` page, which is the general compare page (two `code`
  rows with `sync: true`).
  Both URIs carry their whole spec, so App's `handleOpenPage` and
  in-tab navigation register them directly.
- **Disabling.** An extension that is off contributes nothing: it has
  `enabled: false` and no pages, slots or sources, so every consumer
  ignores it; `find_page` and `run_page` say it's disabled. Which are
  on is the team's `extensions:` plus the person's `enableExtensions` /
  `disableExtensions` ("Sources and deployments"). Settings → Extensions
  toggles it through `oxplow.extension.enable` / `disable`.
- **A page of sections leads with what matters.** In a `stack`, a
  titled `viz` section with no rows (no error, not `replaceable`) folds
  into one closing line, "Nothing found: Duplication, Co-change
  Surprises, …" (`foldEmpty` in `viz/vizModel.ts`, used by
  `pages/PageTree.tsx`). The effort page opens with its
  **verdict** above the slots: the `effort.verdict` replacement target
  ("Replacements"), which core leaves empty — what counts as tested,
  covered or backed is a review policy's. oxplow-bundled's
  `effort-verdict` page is the default (the last test run, diff coverage
  against the `oxplow.coverage.abs_pct` metric's `warnAt`, unverified
  claims, decisions to confirm); another extension may bring its own and
  the person chooses. The task page links to the effort page as
  **Review**: on each effort's activity card, and in the details rail for
  the newest effort.
- **The review packet** (in `oxplow-bundled`). Its pages, mounted in
  the `sections` slots of `oxplow-foundation/diff-view` (the Changes
  page), `commit` and `uncommitted-changes`:
  - Decisions Made (`v_decision`, `provenance = 'recorded'`)
  - Decisions Oxplow Noticed (`v_decision`, `provenance = 'inferred'`)
  - Unverified Claims (`v_claim` whose `v_oxplow_bundled_claim_status`
    says `verified = 0`: it cites nothing and no person verified it, and
    for a `tests_pass` claim its effort's latest test run didn't pass
    with at least one test — the bundled policy's rule; core keeps only
    what a claim cites and its `review`; a dataset reads the claims from
    the agent's answers, nothing records them), plus the dismissed ones
    (which count for nothing, shown so they can be reopened), each row
    with its review state and **Mark Verified**
    (`oxplow.effort.verify_claim`), **Dismiss**
    (`oxplow.effort.dismiss_claim`) and **Reopen**
    (`oxplow.effort.reopen_claim`)
  - Decisions Oxplow Noticed's rows carry **Confirm** / **Dismiss**
    (`oxplow.effort.confirm_decision` / `oxplow.effort.dismiss_decision`)
  - What Deviated (its own dataset `v_oxplow_bundled_deviation`: each
    effort's files against its work item's title and body — a file is in
    the area when the text names it or one of its directories at least
    two levels deep; none when the item names no area. Live, not
    materialized: 0.2 s over all 5.5k effort files of this repo's
    database)
  - Tests Weakened (deleted test functions from `v_change_function`,
    fewer assertions and new skip markers from `v_change_test_file`;
    also mounted in the `oxplow-foundation/commit.sections` and `oxplow-foundation/uncommitted.sections` slots)
  - Struggled Here (`v_oxplow_bundled_struggle`, the bundled dataset: a file edited 5+ times or 3+ failed shell commands per effort)
  - Review Prompt (a copyable markdown prompt for reviewing the effort
    with a second harness: the task, the agent's summary, the files it
    changed, its claims and recorded decisions, and what to report; an
    effort's item comes from the active work list (`v_work_item`), and an
    effort on no item names itself instead.
    What Deviated stays silent there: with no task text, no area is
    stated)
  - Context Read (`v_context_read`)
  - Verify a Claim With Evidence (hidden, `viz: form` over
    `oxplow.effort.verify_claim`, `claim` a param)

  **Its verdicts (P7.C5, the first bundled `commands:`)**, on an
  effort's page under **Commands** (their `ui.about: effort`):
  `oxplow.review.accept { effort, force? }` comments the review on the
  effort's work item then transitions it to `done`, refusing (`{ refuse }`)
  while a claim is unverified (a dismissed one doesn't count) or an
  inferred decision unreviewed unless
  `force` (then it lists them in the comment);
  `oxplow.review.request_changes { effort, note? }` comments a checklist —
  each unverified claim, inferred decision, file outside the area, and
  the note — then transitions to `todo`. A claim, decision or path is
  spliced in through `md_text` (one inert line: an agent's claim can't
  add headings, links or items to the reviewer's comment); the note is
  the reviewer's own markdown. Both read one `input` query
  (`v_effort` + `json_group_array`s over `v_claim` and its
  `v_oxplow_bundled_claim_status`, `v_decision`,
  `v_oxplow_bundled_deviation`), compose `oxplow.work_item.comment` and
  `oxplow.work_item.transition` — as steps, every list's verbs running
  outside the transaction (not undoable; [commands.md](./commands.md) →
  "Composition") — refuse an
  effort without a work item by name, are `confirm: always`, and are a person's or
  a page's — never an agent's. `questions.yaml` + `README.md` (its skill)
  say what an agent can read of the packet; a bundled extension's
  questions are checked against a running registry by
  `the_bundled_extensions_answer_their_questions` (oxplow-sdk).

  It also has a Waiting on Me page, reachable from the launcher:
  what your agents are waiting on you for (`v_agent_status` with
  `awaiting_user`: a question or plan they put to you, a permission
  prompt, a final message ending in a question), blocked tasks, and open
  notes. Each empty state says what's *good*, e.g.
  "Every claim for this effort is backed by evidence."

  The slot pages above need `effort_id` / `change_id`, so they're
  `hidden` from the launcher (opened there they'd show NULL-param empty
  states). The launcher gets stream-level starters instead, under Work:
  **Recent Decisions** and **Unbacked Claims**, scoped by the implicit
  `stream_id`. `launcher_pages_need_no_slot_params` enforces the rule
  for every bundled page.
- **One skill set.** Every agent runtime writes its skills and commands
  from one `AgentText` (`oxplow_domain::agent::text`): core's
  (`OXPLOW_SKILLS` and `CORE_COMMANDS` in
  `crates/oxplow-agent-text/src/lib.rs`) plus what
  extensions offer now ("Skills").

- **P6b (2026-10-01).** An extension can also *act* and
  *extend the shell*: `commands:` (Starlark composing core commands, see
  "Commands"), `ui:` — `slots` in one dotted namespace ("Slots"),
  `commands` in core menus ("Commands in core menus"), `decorators`
  (stable since P10) — and `custom_components:` (stable since P11)
  rendered sandboxed by a page's `custom` viz or `component` node
  ("Custom components"). Installs, updates and
  deploys are reviewed by what they change ("Reviewing by
  effect"). An agent's run that needs a person waits as a proposal
  ([commands.md](./commands.md), "Proposals").
- **P8 (2026-10-02).** The experimental kinds run, in a private
  extension: `event_types:` ("Event types"), `ref_kinds:` (now `resources:`, "Resources")
  and `effects:` ("Effects"), through one swappable vocabulary. A change
  is reviewed by its impact — rows, collector outputs, effects'
  reactions — at any revision, on the CLI (`extension check --impact`)
  and in an effort's review ("Reviewing by impact"). Datasets declare keys and may materialize on a
  clock or incrementally ([semantic-layer.md](./semantic-layer.md)).
  Changes go live with `oxplow.extension.deploy` ("Sources and deployments").

## The rules

- **Controls in core, instruments in extensions.** Core supplies the page
  and reference graph, stream/thread shell,
  agent surfaces, diff and inspection primitives, the semantic layer, and
  safe host services. Tasks, knowledge storage, and effort policy have
  selectable implementations, including none where supported. Collected
  evidence and analysis mechanisms are shared infrastructure; particular
  workflow policies and presentations belong in extensions. The
  [foundation roadmap](./foundation-roadmap.md) identifies remaining core
  policies to extract through public APIs.
- **One format for first- and third-party.** The bundled
  `oxplow-bundled` extension uses exactly the format a user or their
  agent writes. If extracting it needs something the format lacks, that
  feature is added to core for everyone and logged under "Added for
  extraction" below. No private backdoors.
- **Host-mediated execution.** Extensions use YAML, SQL, scripts,
  external programs, and optional custom components in sandboxed frames.
  Extension JavaScript does not execute with the app window's authority.
  Code in the window holds the daemon's UI
  token, and its `/events` socket carries terminal input, so agent-written JS
  in the window could drive the agent. Custom components use the declared,
  scoped bridge instead; arbitrary app-window code remains outside that
  contract.
- **Presentations are policy.** Bundled pages provide useful starting
  views, including exceptions at an effort boundary. Trends, reports and
  domain-specific dashboards are equally valid extension presentations;
  the semantic data and metrics are reusable independently of a view.

## Sources and deployments

An extension has a **source**, the files a person or their agent edits,
and **deployments**, the immutable copies oxplow runs. Nothing runs a
source directly: an edit changes nothing until it is deployed.

- **Source:** `.oxplow/extensions/<name>/` in a checkout, unless
  `extensionSources: { <name>: <folder> }` points the name elsewhere. The
  folder is relative to the checkout or absolute (`~/` is the home folder).
  The team's `extensionSources` is in `.oxplow/project.yaml`; a person's own,
  which wins, is in `.oxplow/personal.yaml`. This repo points
  `oxplow-foundation` and `oxplow-bundled` at `extensions/…`, so their
  sources are ordinary repo files. A personal source outside every checkout
  (`~/oxplow-extensions/x`) is how an agent in any stream edits it: the write
  guard only blocks paths inside another stream's checkout.
- **One deployable source.** The settings (which names are on, their
  `extensionSources`) are always the primary checkout's. A source inside the
  project is deployed only from the **primary** stream's checkout (the one
  the project was opened from, whichever branch it is on), uncommitted
  changes included. Another stream's checkout holds a **draft** of it: that
  stream's agent edits, checks and tests it there, and it deploys once the
  change reaches the primary's branch. A source outside the project has one
  copy, so any stream deploys it (`extension_deploy::refuse_draft_deploy`).
  `oxplow.extension.draft_status { name, stream_id? }` (a read,
  `extension_deploy::source_diff`) says how a stream's source differs from
  the running version: pages and commands added,
  removed and changed, and whether the checkout is a draft. A deploy that
  removes a page or command still goes, with a warning naming them.
- **Deployment:** `.oxplow/runtime/extensions/<name>/<hash>/`, where
  `<hash>` is the hash of the source (`extension_store::source_hash`: sha256
  over each file's relative path and bytes, dot entries skipped, cut to 16
  hex characters). The hash is the extension's identity; a deployment
  folder is written whole (copied beside, then renamed) and never changed.
  A source with a `build:` hashes its **inputs** — every file but
  `build.output` and the `build.exclude` folders — plus its `build.run`
  text (`extension_store::Inputs`; a manifest without `build` hashes as it
  always did), and its deployment holds those inputs and the built output,
  never the excluded folders ("Builds").
  Everything oxplow reads for a deployed extension — manifest, pages,
  Starlark, datasets, component bundles, provider entries, skills — comes
  from that folder. Its `Extension.path` is the folder's absolute path, so
  every `root.join(ext.path)` reader finds it. The file watcher never looks
  under `.oxplow/runtime/`; it does watch `.oxplow/extensions/` (an agent's
  edits to a source are its work, snapshotted and given to its effort), and
  watching never activates anything.
- **Shipped deployments:** `oxplow-foundation` and `oxplow-bundled` ship
  inside oxplow (`bundled_extensions::BUNDLED`, each one's files through
  `BundledExtension::files()`) and are written to the store once per hash as
  their shipped deployment (`extension_store::extract_shipped`); they run
  from disk like any other. What ships is the folder: every file but dot
  entries and `build.exclude`, its build's output included.
  - **A release build compiles them in.** `crates/oxplow-app/build.rs`
    walks each folder into `include_bytes!` arrays, so adding a file to the
    folder ships it; one with a `build:` is built there first (its command,
    `/bin/sh -c` in its folder), so its output ships without being
    committed — building oxplow needs the tools those builds use (bun).
    Cargo is told to rerun on the manifest, every input file, and the input
    folders that hold nothing skipped — never a folder holding the output,
    `node_modules` or a dot entry, which cargo would check recursively and
    find rewritten by the build every time. So a new top-level file (or one
    in a folder like that) is noticed only after `extension.yaml` is
    touched.
  - **A debug build (and every Rust test) reads them from their folders**
    the first time they're asked for, once per process
    (`extension_store::shipped_files`), so editing an extension never
    recompiles `oxplow-app` or anything above it. Its `build.rs` watches only
    itself and runs a `build:` only when its `output` folder is missing (a
    fresh checkout); a changed source reaches a running dev oxplow through
    deploy, which builds. A test reads the folder as it is on disk, edits
    included, the moment it runs.
  - The shipped hash uses the source rule
  (`extension_store::shipped_hash`: the inputs and the build command,
  never the output), so deploying an unchanged
  `extensions/oxplow-foundation` resolves to the shipped deployment. A
  bundled extension with no active deployment of its own runs the shipped
  one (`Deployment::Shipped`).
  - **A folder never stands in for one.** Outside development a source
    load (`load_sources`, `source_or_shipped`) takes a shipped name's copy
    built in, never `.oxplow/extensions/<name>/`: a project folder with
    that name is ignored with a warning on the shipped extension, so a
    cloned repo can't load its own `oxplow-bundled` as oxplow's. A load
    therefore holds one entry per name, and `extensions::owner(list,
    name)` is the one lookup by name — none tie-breaks on origin
    (`source_guards::no_lookup_by_name_prefers_an_origin`).
- **Developing oxplow (`enableBundledExtensions: false`):** in a dev
  build (`cfg!(debug_assertions)`; a release build logs and ignores it),
  oxplow's own extensions don't run from the copies built in: they load
  from their `extensionSources` folders and are deployed like any
  extension (`bundled_extensions::shipped(name, &settings)` is the one
  accessor for the shipped files; `find`, `is_required`, `is_reserved`
  and `origin_of` stay name-based, so the `oxplow` namespace and the other
  rules for oxplow's own extensions still apply). Set in `project.yaml`
  (this repo commits it), overridable in `personal.yaml`, read
  once when the catalog is made (`ExtensionCatalog::settings`): changing
  it takes a restart. Boot deploys nothing here either: a checkout's first
  run, and a core change that foundation's recorded deployment doesn't
  activate against, need `cargo run -q -p oxplow-dev -- extension deploy`
  with oxplow quit (`extension_deploy::DEPLOY_FROM_OUTSIDE`; it deploys
  every extension that is on whose source isn't what runs, foundation
  first). Nothing ships to roll back to, and a non-required one can be
  removed.
- **The live set:** each extension's active deployment, in the
  `extension_deployment` table (`v_extension_deployment`: name, hash,
  deployed_at, commit, dirty, source, active). `commit` is the primary
  checkout's HEAD when the source lies inside the project (NULL outside it)
  and `dirty` says the source had uncommitted changes, untracked files
  included: an uncommitted deploy is normal, and rollback names the
  target's commit. The publish transaction of every
  activation marks the active rows from what it published, so the table
  and what runs never disagree. The store keeps the active deployment and
  the five newest (`extension_store::KEEP`); deploy prunes the rest.

**Which extensions are on** is two lists: the team's `extensions: [..]` in
`.oxplow/project.yaml` (absent: `[oxplow-bundled]`; `[]` turns it off) and
the person's `enableExtensions` / `disableExtensions` in
`.oxplow/personal.yaml`. On = (team ∪ enable) − disable
(`oxplow_config::ExtensionSettings::enabled`). `oxplow-foundation` is always
on and never listed. A listed name with neither a source nor a deployment
on this machine is listed as `not_here` (`v_extension.deployment`), not an
error, so a teammate's personal extension in the team list breaks nothing.

**Two ways to load** (`extension_catalog.rs`):

- **Deployed** (the daemon's catalog, `ExtensionCatalog::for_project`): the
  live set, the same whatever root a caller names. An extension that is on
  but has no deployment is listed `undeployed`, not run.
  `extensions::load_deployed`.
- **Source** (standalone `ExtensionCatalog::new()`, the CLI, the SDK, the
  authoring checks `oxplow_validate_extension` / preview): each source
  folder in a checkout, every one present on unless `disableExtensions`
  lists it (`extensions::sources_on`) — what the author's sources would do.
  `extensions::load_extensions`.

**Deploy** (`oxplow.extension.deploy { name }`, the agent's
`run_command`; the CLI has no way to reach the running daemon yet, so it
has no deploy; `extension_deploy::deploy`): a caller in a stream other than
the primary is refused when the source lies inside the project
(`refuse_draft_deploy`, naming the check and test tools and the branch it
deploys from) → the source in the primary checkout → hash → already active is `already: true` →
source load, whose errors refuse it → copy into the store and record the
row → turn it on in the person's layer when nothing turns it on → stage the
live set with it in (`ExtensionCatalog::stage_changes`, which every staging
check's `snapshot()` then reads) and publish. An extension the staging holds
back (it doesn't prepare) keeps its previous deployment and the deploy
fails with its problems; a candidate overtaken by another change is staged
again. The command returns once the deployment runs; restart starts from
the recorded live set. Nothing caps an
extension's size: a deployment over 25 MiB goes live with a warning in the
result's `warnings` (every deployment copies the whole folder).
`oxplow.extension.rollback { name, to? }` makes an earlier deployment
active (`to` a hash or its start; a bundled one with nothing older returns
to the shipped deployment). `oxplow.extension.enable` / `disable` write the
person's lists (enable deploys one with no deployment here).
`oxplow.extension.remove` takes it out of both lists, stops it, forgets its
deployments, private state and settings, and never deletes its source.

**Builds.** `build: { run, output, exclude? }` is for a component or a
program that needs a tool to make (TypeScript, a bundler). Deploy runs
`run` with `/bin/sh -c` in the source folder (`extension_build::run`:
this process's environment with the person's `PATH` and `HOME` from the
host env, a process group, a 10-minute timeout, the end of its output in
a failure), then copies the inputs and `output` into the deployment and
loads the deployment folder: one that doesn't load once built (a bundle
the build didn't write) is refused (``extension `x` doesn't load once
built``) and leaves nothing behind. A build that fails, or doesn't write
`output`, refuses the deploy with ``building `x`: …``. A build runs with
the person's authority and oxplow doesn't confine it, so its command is
approved like a permission: a new or changed `run` adds ``build — runs
`…` in <source> with your authority, on every deploy`` to what the deploy
asks (`extension_deploy::build_line`); the same command never asks again.
The approval covers the command text only — what `npm run build` does is
whatever `package.json` says. The recorded command
(`extension_deployment.build_run`, `v_extension_deployment.build`) is what
rollback compares; rollback never rebuilds, and neither does a deploy whose
hash is already in the store. A component bundle under `output` loads from
the source as declared (`BundleLook::Unknown`) while `output` doesn't
exist yet, so validate and preview pass before any build. A bundled
extension's build runs when oxplow is built ("Shipped deployments"); while
developing oxplow its deploy (from `oxplow-dev` or the app) runs the build
like any extension's. `oxplow extension check`
runs no build (the built source isn't deployed in its throwaway);
`extension test` builds, as the author's own run; its copy links the
`exclude` folders instead of copying them.

**Startup** (`extension_deploy::start`, before the first activation)
**deploys nothing.** The live set starts from the active rows whose folder
is still in the store, and that is all boot runs. An extension that is on
with no deployment here (a fresh clone, a just-pulled teammate's) is listed
`undeployed`, its `pending` naming what its deploy will ask for, until a
person deploys it. So what runs is only ever what someone deployed, never a
source caught half-edited. (Until 2026-10-24 boot deployed those sources
itself; with a held deployment losing its record, that made a core change
take two restarts and put whatever the source held live unasked.)
Boot is one activation (`ExtensionActivation::initialize`: `start`, then
the first generation).

**The live set is what was chosen, not what happened to run.** A
deployment that is held — it doesn't load or doesn't prepare against this
oxplow — keeps its row `active` and its place in the catalog's live map,
whatever runs in its place (its last working version, the copy that
ships, or nothing): every activation and every boot tries it again and
lists why it doesn't run (`v_extension.errors`), until a person deploys
another or rolls back. The publication records the published deployments
plus each held name's chosen hash (`activate_kind`: `chosen`, read before
the change is staged). A change that is held was never chosen, so a failed
deploy still changes nothing. Because the chosen hash of a held extension
isn't running, `deploy`'s "already" check also asks that it runs: deploying
the same source again activates it again.

A deployment of an extension that ships with oxplow that is held with
nothing of it running — it doesn't load or doesn't prepare — **gives way**
to the copy that ships (`activate_kind`: out of the staged changes,
admitted again); its problems say "the shipped version runs until it is
deployed again", and it stays chosen as above. While developing oxplow
nothing ships, so a broken or missing foundation is just not there: the
rest activates and its problem says why. `start` reports a required
extension with no deployment at all; `explain_required` reports one whose
deployment doesn't activate. Both lines say what is missing without it
(the agent harnesses, Alerts, core's commands and pages) and how to bring
it back from outside — quit oxplow, run `cargo run -q -p oxplow-dev --
extension deploy`, start it — since the commands that deploy are
foundation's own. It is logged, listed in `v_extension.errors` by the
activation that finds it (startup's own problems too: `initialize` hands
them to the first activation), and stands across later activations until
it runs. The desktop shell draws one plain dialog for it
(`apps/desktop/src/foundationDown.ts`, `FoundationDownDialog` in
`App.tsx`): the problem lines and that command, Copy and Dismiss; nothing
else in the app handles a missing foundation. The usual cause is a
deployment an older oxplow recorded meeting a newer core: core and
foundation's source change together in one commit, but the recorded
deployment is the source as it was. `oxplow-dev extension status` shows
what the next boot would run and hold back without starting it
(`extension_deploy::next_boot`: the active deployments activated in a
throwaway in memory), and `oxplow-dev extension deploy [<name>]` deploys
with oxplow quit (`.context/working-in-this-repo.md` "oxplow-dev"). Boot
stops only when a generation can't publish at all (the host, the
database).

**Sharing is one explicit act.** `.oxplow/.gitignore` ignores everything
but `project.yaml`, so a new extension's source is personal.
`oxplow.extension.share { name }` appends `!extensions/`, `extensions/*`,
`!extensions/<name>/` and `!extensions/<name>/**` (git can't re-include a
file under an excluded folder, and `*` matches files at every depth) and
moves the name from the person's `enableExtensions` to the team's
`extensions`; the person commits. A built extension's `output` and
`exclude` folders get their own ignore lines (`extensions/<name>/dist/`)
after those: they aren't source. `oxplow.extension.unshare` reverses it,
removing every line under `extensions/<name>/` whatever the manifest says
now.
Only a source in `.oxplow/extensions/` is shared this way.

**Requires.** `requires: [name, …]` names the other extensions this one
needs on (`extension_deploy::requirements_met` / `not_required`). Deploying
or enabling it refuses while one is off (``"`acme` requires `tracker`,
which is off (turn it on first)"``); disabling or removing one refuses
while an enabled extension requires it. Activation holds an enabled
extension whose requirement isn't among the admitted ones, with the same
last-working-version rule as an unavailable import
(`CatalogSnapshot::admitted`: "it requires `x`, which isn't active"). An
entry must be a valid extension name other than its own, and a cycle is a
load error on each member (`refuse_requires_cycles`). A required bundled
extension (foundation) always counts as on. Unlike `imports:`, `requires`
names no contract: it is for composing another extension's commands or
reading its datasets without an export. **Using another extension without
declaring it is an error** (`check_dependencies`, in `validate_extension`):
reading its dataset (`v_<namespace>_…` in a page, a dataset or a script),
naming its page (`<name>/…`), mounting at its slot or running its command
(`<namespace>.…`, unless the required foundation shares the namespace) —
one finding per extension, at the first place, `fix: requires: [x]`. The
required foundation needs no declaration. A test turns on only what is
declared, so the check says why before a fixture fails on a missing table.

**Writing a page** (`oxplow.extension.write_page { extension, page, spec,
deploy? }`, `extension_deploy::write_page`) writes `pages/<page>.yaml` into
the extension's source (created with a scaffolded manifest when missing)
in the primary checkout, whichever stream asks. The spec must parse as a
page and the extension must still load with it, or the file is put back
and the call fails with the loader's reason. With `deploy: true` it then
deploys, like any deploy (it asks when the source gained a permission).
`oxplow.extension.new_dashboard` writes an empty dashboard page the same
way. A bundled extension is never written to
(`extensions::writable_extension_dir`).

A malformed extension shows an error for that extension only; it never
breaks the others or core.

## Permissions

An extension declares what it may reach beyond the **baseline** in its
manifest's `permissions:` (`oxplow_domain::permissions::Permissions`); a
person approves it when the extension is deployed. The baseline needs no
declaration: pages, panels, slots, decorators, replacements; its
own datasets; a collector's `input:` query (declared and reviewable); its
own commands and the effects, jobs and commands whose scripts compose them;
components that query their own datasets; Starlark and jaq collectors that
don't ask a model; `state.read`, `extension_config.read`, `jobs.read`.

```yaml
# A list, in a script's `needs:` grammar (a scope id, or `<scope>:<qualifier>`);
# absent or [] = baseline only. `oxplow_domain::permissions::PERMISSION_IDS`.
permissions:
  - sql.read                     # ad-hoc read-only SQL (a script's `needs: [sql.read]`)
  - commands:oxplow.work_item.*  # commands outside its own namespace (`*` one segment, `**` the rest)
  - files.read                   # or files.write (includes read): `needs: [tree.read]`
  - ai.compute                   # models, with the person's key
  - network:api.github.com       # hosts its programs reach (`network:*.example.com` covers the names under it)
  - process.run                  # runs programs: providers, `runtime: exec` collectors
  - secrets:GITHUB_TOKEN         # secrets it reads, by name
  - agent.write                # advisories and skills: text that reaches the agent
```

The map it used to be (`{ sql: read, process: true, … }`) is refused,
the refusal naming the list (`oxplow_domain::manifest_keys`). Inside oxplow
the declaration is still the typed `Permissions`; only its spelling is
the list — and what a deployment stores (`v_extension.permissions`,
`v_extension_deployment.permissions`, V117 rewrote the stored maps).

`process` is declared, **not confined**: a program runs with the person's
file, network and process authority (`.context/providers.md` "External
execution authority"), and what a deploy asks says so plainly.

**The loader checks the declaration against the contents**
(`extensions::check_permissions`, at the end of `load_one`): each
contribution that needs something the manifest doesn't declare is an error
at the contribution's line, naming what to add —
``provider `github` needs `process.run`, `network:api.github.com` `` in `permissions:`.
What needs what (`needed_permissions`): a provider with an `entry` or an
MCP `command` → `process`, its `network` and its credentials' names →
`network`, `secrets` (a URL adapter runs oxplow's own adapter: only its
hosts and credentials); an `exec` collector that isn't a bundled parser →
`process`, `network`, `secrets`; any declared `needs` of `ai.compute` /
`ai.discover` → `ai`, `sql.read` → `sql: read`, `tree.read` → `files:
read` (the *declared* needs: a collector's `input:` is baseline); a
Starlark collector whose script calls an `ai_*` builtin → `ai`; an
`ai_provider` implementation → `ai`; advisories, skills and
implementations with skills → `agent`; a custom component's `commands`
outside its namespace → `commands`. A script's composed commands can't be
known before it runs, so they are checked when it runs. A page's
`answers:` are suggestions a person picks, not agent text, and need nothing.

**Approval is the deploy** (`extension_deploy::approved`). The approved
set is what runs: the active deployment's permissions (the running catalog
entry's; a bundled extension with no deployment of its own runs what ships,
`shipped_permissions`). A deploy, rollback or enable whose permissions add
nothing to that goes live at once — a narrower one too. One that adds a
permission changes nothing (no copy, no row, not turned on) and returns
`DeployError::NeedsApproval { adds }`, one line per added permission
(`Permissions::added_over`: "process — runs programs with your authority;
oxplow does not confine them", "network — may reach api.github.com", …).
The command (`oxplow.extension.deploy`, `rollback`, `enable`) raises that
as `NeedsConfirmation` with the lines in `Preview::details`: a person
confirms inline (`CommandConfirm` lists them), an agent's call becomes a
proposal naming them (a deploy's consent check is a person's to approve), and the approval deploys it.
There is no separate approval store: each deployment row records the
permissions it was approved with (`v_extension_deployment.permissions`).
A fresh machine has nothing running, so a teammate's extension waits for
its first deploy here (Startup), which asks for what it declares.

**The Extensions page** (foundation's `extensions` page, which embeds
`extensions-list` and `extensions-deployments`; Settings → Extensions
embeds the same two pages): `extensions-list` is every
extension from `v_extension` joined to `v_extension_source` — the live
dataset of each source as it is now, so an edit shows at once ("out of date"
when its hash isn't the running deployment's) — with its permissions and
what it waits on, and the row actions Deploy (offered only when undeployed
or out of date), Roll Back, Enable, Disable, Share with the Team, Stop
Sharing and Remove, each offered by a `when:` column the query computes;
`extensions-deployments` is `v_extension_deployment`, newest first, with
Roll Back to This. A row action that adds permissions asks inline like any
deploy.

`v_extension.permissions` is the declaration as JSON (`{}`: baseline);
`v_extension.pending` is what an undeployed extension waits on a person
for (the same lines; `[]` when nothing).
The shipped extensions declare theirs like any other: `oxplow-foundation`
`{ ai: true }` (its AI providers), `oxplow-bundled` `{ sql: read, ai: true,
agent: true }`.

## `extension.yaml`

Manifest **version 2** (P1.11; `crates/oxplow-app/src/extension/extensions/manifest_v2.rs`).
Every contribution is declared as data, unknown keys are errors, and the
loader resolves cross-references with `file:line` messages.

```yaml
manifest: 2
name: oxplow-bundled   # must equal the folder name
namespace: oxplow      # its command ids' namespace (default: the name, `-` → `_`); `oxplow` only for oxplow's own
description: …
engine: ">=0.7"          # optional: the oxplow it targets
permissions: [ai.compute] # optional: what it reaches beyond the baseline ("Permissions")
requires: [tracker]      # optional: other extensions it needs on ("Sources and deployments" → Requires)
build: { run: bun run build, output: dist, exclude: [node_modules] } # optional: built at deploy ("Builds")
intent:                  # required
  purpose: What question it answers, or what job it does
  origin: oxplow:oxplow/effort?id=eff42   # the thread/effort URI that created it, or null
  examples:              # inputs → expected outputs; fixtures for `extension test`
    - { name: …, input: {…}, expect: … }
# kinds
measures:    [...]   # also what the project's own extension declares
metrics:     [...]   # `key:` definitions; `measure:` must be declared here or oxplow.*
dimensions:  [...]
collectors:  [...]   # entity collectors (exec / starlark / jaq / read → entities) and
                     # fact collectors (`facts:`; starlark / jaq only; facts must be declared here or oxplow.*)
                     # see semantic-layer.md "Collectors"
ui:                  # what it adds to the core UI (a command's own `ui:` puts it in core menus: see "Commands")
  slots:             # pages mounted at other pages' slots; see "Slots"
    - { page: change-review, at: oxplow-foundation/diff-view.sections }
  decorators: …      # labels from its datasets on core URIs (stable since P10; see "Decorators")
  replacements: …    # a page in place of a named core component (see "Replacements")
advisories:  [...]   # see "Advisories"
datasets:   [...]   # SQL datasets: DatasetDecl entries (`fields:`, optional `relationships: [{ from, to: <dataset>.<field> }]`) + datasets/<name>.sql → v_<ext>_<name>; `key: [fields]`; `materialize: on_change | { every: 1h } | { incremental: <field> }` stores one (semantic-layer.md "Datasets" vocabulary, "Extension datasets", "Materialized datasets"); the old `datasets:` / `columns:` are unknown keys with a hint
panels: …           # left-nav panels, each a page compact (see "Panels")
commands:  [...]   # Starlark scripts composing core commands, a scope's or its provider's operations (see "Commands")
resources: …        # kinds of thing a URI can name (see "Resources")
event_types: …      # its own namespace's event types (stable; see "Event types", P8)
implementations: …  # capability implementations it declares (oxplow-foundation declares the defaults)
skills: …           # skills and slash commands for the coding agent
providers: [...]    # external providers over the provider protocol — a program, or an MCP server behind oxplow's adapter (providers.md)
effects: …          # scripts reacting to logged events by composing commands (see "Effects")
custom_components: …   # sandboxed components a page's `component` node or `custom` viz draws (see "Custom components")
```

Pages aren't listed here: every `pages/*.yaml` file in the folder is
loaded. A page gets into the launcher through its own `category` (or
stays out with `hidden: true`). An old-format UI folder (`lenses/`) is a
load error that says to rewrite each file as a page.

To move an old extension over: each file in `lenses/` becomes a page
(a document's sections go into `view.stack`, its queries into named
`loaders:` loaders); fixtures and `intent.examples` name `page:` instead of
`lens:`, with table expectations under `expect.loaders.<loader>`. Then
delete the YAML files in `lenses/`: they are what the loader refuses.
`oxplow extension check/test <name>` says when it loads again.

**What the loader checks** (`Extension.errors` / `Extension.warnings`):
- shape: unknown keys, `manifest` version, `intent` present and
  `intent.origin` a canonical URI; an empty `intent.examples` is a
  warning;
- page files: each `pages/*.yaml` parses (`pages::schema::parse_page`:
  unknown keys are errors; a `replaceable` names a replaceable component
  and is placed once); an `icon` oxplow draws; the pages that a param's
  `options_from`, a column's `expand` and a form field's `from` name are in
  `pages/`; a page's renderer declarations (`for`, `accepts`, `default`,
  `pinned`) fit the resources it names; a `component` node or `custom`
  viz names a component the extension declares in `custom_components:`.
  Layout problems are **warnings** (`pages::checks::layout_warnings`:
  more than 6 filters, a `choice` of more than 12 options, a description
  over 200 characters, a `stack` of more than 8 sections, a table of
  more than 8 columns, a param no loader reads);
- lifecycle: `engine`, when given, is `>=MAJOR.MINOR[.PATCH]`;
- cross-references: a `ui.mounts` page is in `pages/` and its slot name
  has the shape `<extension>/<page>.<name>`; a slot mount whose host page
  is not loaded, or has no such slot node, is a warning (it shows
  nowhere); a `panels` body and a replacement's page are the extension's
  own pages; a replacement page declares every prop of its target's
  contract. A metric's `measure` or a fact collector's `facts` that
  is neither declared in the extension nor an `oxplow.*` built-in is a
  **warning** (it may come from the project's or another extension's
  `measures:`, which resolves when the catalog is assembled).

**Each entry's error is at its own line.** Every list block — `commands`,
`effects`, `collectors`, `panels`, `implementations`, `skills`,
`resources`, `custom_components`, `event_types.types`,
`ui.decorators` — and the manifest's own nested lists
(`intent.examples`, `ui.mounts`) type their entries one
at a time: a broken entry is left out with an error at its own line
(`manifest_v2::item_lines`, the line of each `- ` item, so a shape error
with no name to look the entry up by lands there too), and the rest of
the extension loads. `intent` and `ui.mounts` are read raw
(`IntentFile`, `Vec<Value>`) and typed by `intent_of` / `slots_of`,
which `check` and the loader share.

**Schemas.** `extension.yaml` has a JSON Schema generated from the
loader's own serde types (`extensions/schema.rs`: `ManifestV2` with
`#[schemars(with = …)]` naming each kind's on-disk struct), checked in at
`docs/reference/schemas/extension.schema.json` and published with the
docs. A test keeps it current (`OXPLOW_BLESS=1` regenerates it), and
every bundled manifest must validate. Page files have no JSON Schema;
`pages::schema` (`parse_page`) is their one reader. Scaffolds start with
a `# yaml-language-server: $schema=…` modeline. A schema holds the shape only; cross-references and
which keys go together stay the loader's to check. When you add a kind
or a key, derive `JsonSchema` on its file struct. The user-facing
reference is `docs/reference/extensions.md` and
`docs/reference/provider-protocol.md`.

**Decision (2026-09-28): `advisories` is a stable kind.** The plan sketch
had it experimental, but `oxplow-bundled` — bundled, so shared by
definition — ships on it, and a first-party extension depending on a
kind is exactly the evidence promotion requires (target §10.1, §12).

**Only v2 loads.** A manifest without `manifest: 2` is a load
error (`extension.yaml:1`); so is any key v2 doesn't have (`sources:`, a
top-level `slots:`, `gauges:` — `deny_unknown_fields`). There is no
reader or migrator for v1, and no recogniser for an old shape: a slot
name that isn't one is an unknown slot, an action that isn't `{ id,
label, command, input?, row? }` a field error.
`Extension.manifest_version` is the
version the file declares.

## The SDK

`crates/oxplow-sdk` (P1.14; target §10.5) is the one
implementation behind three doors: the `oxplow extension new|check|test`
CLI, the agent's `oxplow_validate_extension`, and `write_page`. Every
door gives an author the same report, so an agent editing from a terminal
and one calling MCP read identical `file:line: what — fix` lines.

- **`scaffold_example(root, example, name)`** (`extension new --from <example>`,
  `--list-examples`) copies one of `examples/extensions/*` into the project.
  The SDK carries them (`crates/oxplow-sdk/src/examples.rs`: compiled in by
  `build.rs` in a release build, read from the folder in a debug one, as
  `oxplow-app` does for `extensions/`). The copy keeps the example's name
  (its commands and datasets are named by it); another name is refused:
  rename after copying. `every_example_scaffolds_checks_and_tests_clean`
  keeps each one passing `check` and `test`.
- **`scaffold(root, kind, name, origin)`** (`extension new <kind> <name>`)
  writes `.oxplow/extensions/<name>/extension.yaml` (v2, an `intent` whose `origin` is the URI passed with `--origin`,
  one example) and, but for a bare extension, that example's fixture
  `fixtures/basic.yaml` — then the kind's starter (P7.C6):
  - `page` — `pages/<name>.yaml`, open tasks in the viewer's stream, with
    a Start row action (`oxplow.work_item.transition`, a `uri` column it
    binds but doesn't show) and an `answers:` entry; its example is
    `{ page: <name>, params: { stream_id: 1 } }`;
  - `component` — a `custom_components:` entry (a bundle under
    `components/<name>/`) and a page whose `component` node it feeds;
    `panel` — a `panels:` entry whose `body` is a hidden page;
  - `collector` — a Starlark collector `items` over `v_work_item` declaring
    entity `item`, a dataset `open_items` over `ref('item')` and a page over
    the dataset; its example runs the collector over fixture rows;
  - `metric` — a code metric, built by `oxplow_app::metric_scaffold::
    extension_files` (the one template: `oxplow.metric.scaffold` hands the
    same files to the agent to write). `--language` (default `rust`; the
    ones with a sample: rust, typescript, javascript, python, go, c, cpp)
    and `--glob` (`*` and `**` only; default the language's files) go
    through `scaffold_with` and `MetricStarter`. It declares a per-path,
    semi-additive `<ns>.complex_functions` measure on files, the
    `<ns>.area` dimension (the top folder), a Starlark collector `<ns>.scan`
    on `snapshot.taken` (`source_files()` + `code_metrics()`: one fact per
    file with functions over `LIMIT`, carrying the worst function's line
    and `dims: {<ns>.area}`), the `sum` metric (`lower-better`, sliced by
    the area) and a Code launcher page over `metric_findings()` whose
    `path` column links `file` at the `line` column. Its example runs the
    collector over a two-folder fixture corpus and expects the facts;
  - `command` — `commands: [note]` whose `handlers/note.star` composes
    `oxplow.work_item.comment`, with an example and a `ui` about
    `work_item`; its intent example dry-runs it;
  - `effect` (P8.D12) — `effects: [on-done]` on `work_item.state_changed`
    `where: { to: done }`, whose `effects/on-done.star` composes
    `oxplow.work_item.comment`; its intent example dry-runs it on a fixture
    event;
  - `policy` — an `implementations:` entry (`capability: effort_policy`,
    id = the name with `_` for `-`, `needs: [sql.read]`) whose
    `policies/<name>.star` opens an item's effort when it starts on a
    thread and closes its open efforts (read through `scope`) when it
    ends; its example is a checkpoint expecting `{ skip: $any }`, and it
    passes the effort-policy suite as written;
  - `provider` — below; `extension` — the manifest only.
  It refuses an existing folder, a bad name and an origin that isn't a URI. **What
  it writes passes `check` with no warnings and `extension test` clean**
  (a provider once a program replaces its stub) —
  `crates/oxplow-sdk/tests/just_works.rs` holds each kind to it, through
  to loading in a real oxplow: the page's row action names a registered
  command, the collector syncs and its dataset publishes, the command runs
  through the bus; a work list and an effort policy each pass the kit
  with the fake behind their stub, the policy with its scaffolded example.
- **Recorded fresh-agent runs** (`crates/oxplow-sdk/fixtures/just-works/
  <kind>/`): `scripts/record-just-works.sh <kind>` gives `prompt.md` to
  `claude -p` in an empty git project — `--safe-mode`, so nothing but
  the oxplow-extension skill (appended to its system prompt) and the
  `oxplow` CLI built from the checkout — and keeps `run.json` (without
  the denied commands, which carry local paths, the session id or the
  cost — `recorded_runs_carry_no_local_paths_session_or_cost`), the
  extension it wrote (`produced/`), and `check.txt` / `test.txt`;
  `notes.md` is written by hand. `recorded_agent_runs_still_check_and_
  test_clean` replays every `produced/` against today's oxplow. Recorded
  once (it costs a real run): `collector` (2026-10-02, 64 turns, clean).
- **`check(root, name, catalog, layer: Option<&SqlGateway>)`** is
  `catalog.named` (manifest shape, lifecycle, cross-refs, page shape;
  a Starlark collector's script must parse and define `transform`, an
  error at the collector's line) plus `extensions::validate_extension`'s
  dry run — **always** (P7.C6), and **never on the project's database**:
  on the throwaway oxplow — this build's schema, no data, what the
  extension declares turned on, its datasets and metrics seeded so a page
  may read `metric_grid()` / `metric_findings()` — or on the caller's own
  (`extension test`'s). What happens to be in a project's database is no
  part of a check, and a CLI older or newer than the app is never refused
  over its schema. The agent's `oxplow_validate_extension` checks the same
  way, starting a throwaway in the app. The dry run first compiles, in a rolled-back read,
  the extension's datasets beside the other enabled extensions' (P4.9 —
  resolution, lineage, and the contract, so a changed contract at a
  published version fails here before it publishes) over **empty
  stand-ins for every declared entity that hasn't synced**
  (`datasets::EntityStub`: a typed temp table under the entity's view);
  `datasets::check_extensions` returns those views (`CheckedDatasets.views`).
  Every later query of the check — command examples' reads,
  advisories, pages — reads through them as an **overlay**
  (`SqlGateway::with_overlay`, `SqlQuery::temp_views`: recreated on the
  query's own connection and dropped after), so a fresh collector →
  dataset → page checks clean before its first sync and a page may read
  its own unpublished dataset. The dry run of a page
  (`pages::checks::dry_run`, `extensions::viz_column_problems`) runs its
  `sql` loaders once with every param at its default (an earlier loader
  reads as a table; `command` and `page` loaders do not run, since a
  check acts as no one). A loader that fails is an error. So is a node
  that names a column its loader lacks: a role block's column
  (`chart`, `tree`, …), a column's `key`, `link`, `icon` or `tone`
  column, an action's `when:` or a `{{row.x}}` placeholder. Layout
  problems are warnings. It returns a `CheckReport { ok, errors,
  warnings, dry_run, extension }`; `render_findings` prints it as text
  (`error: <file:line …>` lines then a one-line summary) or JSON.
  `oxplow_validate_extension` also warns about a `fixtures/*.yaml` no intent
  example names (`check_fixture_pairing`; `dataset-*` / `provider-*` belong
  to a dataset or a provider); `extension test` warns about the reverse, an
  example with no fixture (a fresh scaffold has some). Each names the file
  and the fix.
- **`scaffold(…, Kind::Provider { capability }, …)`** (`extension new
  provider <name> [--capability <cap>]`, P5.D5) adds a `providers:` entry
  (id = the name with `_` for `-`, the capability). `Kind::parse("provider")`
  is `work_items`; `Kind::provider(cap)` refuses a capability without a
  provider contract (the CLI exits 2), and `--capability` on another kind
  is a usage error. `provider.json` comes from the capability's
  `ProviderContract`: the verbs it always needs (`confirm: never`,
  `access: record`; every feature off, so no feature-gated verb), the
  events it may emit with `Vocabulary::core()`'s schemas, a collector of
  its records' entity when it streams records, and an object config
  schema. What the contract doesn't say is the SDK's starter per
  capability (`provider_starter`, `verb_input`): each verb's summary and
  input schema, the intent example and its fixture — `create` expecting
  `{ uri: $any }` for a work list; `react` to a checkpoint expecting
  `{ skip: $any }` for an effort policy, whose manifest also `needs:
  [sql.read]` (it hears events only, and reads oxplow for what to close);
  `render` of `{ kind: ack }` expecting `{ body: $any }` for an agent
  harness, whose `launch` / `tool_use` / `render` inputs are the wire's
  (`{ session, endpoints, … }`, `{ body }`, `{ answer }`) and whose
  capability `data` starts as `{ env_markers: [], settings: [] }` (`ProviderStarter.data`; null for a
  contract that asks none). A capability with no starter gets its first verb with `{}` and `$any`.
  Then a stub `bin/provider` that exits 1 and `fixtures/provider-<id>.yaml`
  (`config: {}`).
- **`extension_test::test_extension(root, name, bless)`** (`extension
  test <name> [--bless] [--json] [--junit <file>]`, P5.D5, from either
  binary: `oxplow extension …` and `oxplow-daemon extension …` are the one
  CLI, `oxplow_sdk::cli`) runs on **a throwaway
  oxplow** (`throwaway::Host`) that is a real one: a temp project with a
  copy of the project's `.oxplow/extensions/` and of which it turned off
  (nothing else of its config — its provider instances would start its
  programs), `Services::in_memory_on_machine` with an in-memory keychain,
  its own global config and the environment it is given, activated by
  `ExtensionActivation::initialize` as boot does (the declared entities
  are created empty there; see "Database generation candidates"). What
  that activation held back is the report's first lines (this
  extension's an error, another's a warning). `start_runtime` then starts
  what examples need — the effect and collector triggers (each example
  delivers its event with `EventPump::settle`) and the job scheduler —
  and binds every AI role to the **fixture model** (`oxplow_ai::fixture`,
  a host-registered kind no activation replaces): an example's `dataset: {
  replies }` is what it answers, in turn. No paid call, no provider.
  It runs `check`, then every intent example with a fixture
  (`fixtures/<name>.yaml`: `input`, `expect`, `$any` matching anything):
  a page (`{ page, params? }` with `given:` steps, below), an event (`{ event
  }`: appended to the throwaway's log and delivered; the extension's
  effects react for real — `expect: { effects: [{ id,
  state }], commands }`, read from `v_effect_run` and `command.executed`),
  a job (`{ job, intentKey, request, cancel?, setup?, rows? }`: `setup`
  runs oxplow's own commands first, in order, each as a person or `as:
  agent` (thread 1, after `ensure_primary`), so its tools read real data;
  `rows` as a page's `given:` rows; then through the extension's
  commands over `jobs.write/enqueue` and `jobs.read/snapshot`; `cancel: true` cancels it through `jobs.write/cancel` once it
  has written — `expect: { state, result?, text?, toolCalls?, output? }`,
  `output` its rows `[{ kind, … }]` ("Jobs"); a model reply is a string
  or `{ text?, toolCalls?: [{ name, arguments }], chunkDelayMs?, when? }`
  (`when`: answers the request whose last message carries that text);
  naming `effects`, `commands` or `pages`
  also delivers the `contribution.job.completed`
  the job logs, asked in a thread: the effects that reacted, the commands
  they ran and which of the named wiki URIs now exist), a component (`{ component, queries?,
  invokes? }`: `load_component`, `run_component_query`,
  `invoke_component_command` as its frame would — `expect: { queries:
  [{ rows }], invokes: [{ ok, refused? }] }`), a form (`{ form: <page>,
  node?, params? }`: the page's form node — `node`, else its first — opened
  on a run of the page with `pages::actions::page_form`, as a person opening
  it — `expect: {
  values?, fixed?, fields? }`, the values the fields start from, the
  read-only ones and the shown ones, `null` for all), and the dry runs that
  stay dry (`{ effect, event }`, `{ implementation, event }`, `{ command,
  input }`, a derived or fact collector, an event type, a wikilink).
  **One rule compares every `expect`** (`compare`): an object only on the
  keys it names (a key the result lacks matches only `null`, not even
  `$any`); a list every item, in order; `{ contains: [..] }` the items a
  list has, in any order, among others; `$any` any value that's there;
  `{ $exact: v }` the whole value, every key; `refused: <text>` a reason
  that contains the text. **One way to say no**: a command, an invoke or a
  command example that refuses or fails is `refused: <its reason>`. Then, per declared provider, the conformance kit
  ([providers.md](./providers.md) "The conformance kit"), the effort
  policy suite for a policy script, `questions.yaml` and incremental
  datasets. `TestReport { ok, errors, warnings, blessed, ran, generation,
  contributes, unsupported, left }`: `contributes` is the extension's
  descriptors by public id, `unsupported` what only the app shows
  (panels, decorators, replacements, skills; a slot mount runs in its
  host page's run, so a fixture of the host page checks it); `render`
  prints it like `check`'s. Exit 0 / 1 / 2 as `check`. `check` with no
  running oxplow asks a throwaway (with no environment) for its command
  registry. `the_triage_example_reads_the_tree_the_layer_and_a_dataset`
  runs the triage example's job, event and component this way and holds
  its `contributes` equal to a real oxplow's over the same project.
- **The CLI** is `apps/desktop/src-tauri/src/extension_cli.rs`, dispatched
  by `main.rs` before Tauri boots exactly like `oxplow hook`. It takes a
  bare name (under `--root` or the cwd) or the extension folder's path
  (the project is read off `…/.oxplow/extensions/<name>`). `check` never
  opens the project's database (`check_never_reads_the_project_database`);
  only `--impact`, a review of what a change does to the person's pages,
  reads `<root>/.oxplow/local.sqlite` **read-only** (`Database::open_read_only`:
  no migrations, no writes). Exit 0 clean, 1 with errors (or an SDK error such as an unknown
  extension), 2 usage.
- **`oxplow_validate_extension`** returns the `CheckReport`, not
  the bare `Extension`; the extension is inside it.

The `oxplow-extension` skill requires `check` after every edit.

## Answerability

`crates/oxplow-sdk/src/answerability.rs` checks that an agent
reading the right skill can answer the questions oxplow, or an extension,
exists for. The questions live on the **pages** that answer them: a page's
`answers: [{ question, about?, params, skill? }]` (an entry with `about`
is phrased with "this" and offered on pages for that kind of ref). Core has
no questions files: oxplow-foundation's pages carry them, including small
hidden pages that exist to hold a question (`collectors`, `zones`,
`branch-sync`, `work-item-actions`, …), and a question that says "do this"
is held by a page with a whole-page action running that command.

- **`answerability::check_pages`** (with `live_pages` for the live half):
  the answer's `skill` names every dataset the page's `sql` loaders read,
  every command its `command` loaders run and every command its whole-page
  actions run (no `skill`, a user extension's, skips that); the skill's text
  is `skill_full_text`, so a dataset is named by the skill or any of its
  references, as a whole word in any case (`answerability::names`:
  `v_commit_file` doesn't name `v_commit`); a dataset is a table the query
  reads (`datasets_in`: after `FROM`, `JOIN` or a comma in a `FROM` list),
  never a `v_*` word in a string, a comment or an alias. An action's
  command must exist and its input fit the command's schema. The page runs
  with the answer's params with no loader failing (`RunTarget::Spec` over
  the throwaway `Services`), and `about` names a declared resource. A
  question whose skill doesn't lead there is a finding, which is the point:
  when a capability grows, its questions say which skill must learn about
  it.
- **In tree:** `every_page_answer_reaches_what_its_skill_names` runs it over
  oxplow's own extensions (and checks the kinds a person is offered
  questions about: file, commit, effort, work item);
  `a_page_answer_must_be_named_by_its_skill_and_run` the mechanism.
  `oxplow extension test` runs the second half for every extension's pages
  (`ran: answers`).
- **Live** (`OXPLOW_LIVE_ANSWERABILITY=1`): `answerability::live_pages` asks
  this machine's `decide` dataset (`AiService::for_this_machine`: the global
  `ai.yaml`, keychain read only), given only the skill text and the catalog
  (every `v_dataset` view, every command an agent may run and every tool it
  is offered), what it would reach for first; it must pick the page's first
  loader's dataset or command.
- **An extension's own** `questions.yaml` (`answerability::check`) runs in `oxplow extension test`:
  `skill` is a markdown file in the extension, SQL runs on the test's
  throwaway oxplow, and a command is one on its bus — core's or the
  extension's own `commands:` — or one of its providers' `<id>.<name>`
  (schema from its declarations).

**`oxplow extension test <name>`** (`extension_test.rs`) runs on
**a throwaway oxplow** (`Host`, "The SDK" above: activated as boot
activates, its entities created empty, the fixture model answering every
model call). It never reads what runs here — no deployment, no
database — so an extension is tested before its first deploy. **Which
extensions are on is what it declares**: the required
foundation, the one tested, and what each of those `requires:` or imports
from, in turn — and a fixture's own `requires: [<ext>…]` (a fixture key,
same meaning as the manifest's) adds more, on an oxplow of that fixture's
own (an unknown name is a finding). The default extensions are not on:
oxplow's own pages run complete without them. Oxplow's own work list,
effort policy and wiki are foundation's (always on), so a fixture that makes
work items, efforts or wiki pages needs nothing extra (`requires:
[oxplow-bundled]` only for what bundled adds: its review pages, verdict and
replacements), and an effort policy's conformance suite, which files work,
runs on an oxplow of its own — deployed in that order,
what one requires before it (`Host::start`). Never the project's list or
one person's `enableExtensions` / `disableExtensions`, and never a
sibling it doesn't name: a fixture passes or fails the same for everyone
and in CI, and a broken neighbour is no noise in its report. One named
but neither here nor shipped isn't turned on (an import of it reports it
isn't here). Each source is
copied to where the checkout keeps it
(`extensionSources`, so oxplow's own land at `extensions/<name>/`), and
every finding names that path — the file the author edits. It never opens
the project's database. It runs `check` there
(with that registry, so `commands:` examples dry-run), then — only if `check` is clean — each
`intent.examples[*]` by its fixture `fixtures/<name>.yaml` (the event,
job and component forms are in "The SDK"). A fixture's keys are `given`
(steps run first), `when` (what it does: one key names the kind, the rest
are that kind's), `expect`, `steps`, `view` and `model` (`{ replies }`, the
fixture model's answers); any other key is an error (`FIXTURE_KEYS`).
**The kinds are one list** (`extension_test::KINDS`: each kind's key, its
`when` and its `expect`): the runner's "fix:" hints and the skill's table
(`references/testing.md`, between `<!-- fixture-kinds -->` markers) are
made from it, and `the_skill_lists_every_fixture_kind_the_runner_runs`
fails when the table is stale (`OXPLOW_BLESS=1` writes it). A new kind is
a `KINDS` entry and a `dispatch` arm. The forms:

- `when: { page: <slug | ext/slug>, params? }` with optional `given: [..]`,
  `view: { stream?, thread? }` and `steps: [..]` beside `when`, and
  `expect: { loaders?: { <name>: { rows: n | $any | [ { col: val } | [ val,
  … ] ] | { contains: [..] }, columns? } }, nodes?: [{ id, kind?,
  actions? }], text?, alert?,
  badge?, inactive?, actions? }` — the page runs (`PageRunner`, as a person,
  from the `view`'s stream and thread) after the `given` steps, each `{
  command, input?, as?, confirmed?, bind? }` or `{ rows: { <entity>: [..] }
  }`, in order. Everything is compared by the one rule: a row is its
  columns by name (or its cells, where the example writes the row as a
  list), `rows: n` a count; `nodes` ids are in the
  run's tree (`<embed>/<id>` inside an embed); `alert`, `badge` and
  `inactive` are the run's own. `actions`
  (the page's, or a node's viz's) list ids, or objects compared on the
  fields they name, spelled as the page YAML spells them (`one_row`, which
  oxplow derives, included) — `actions_like`. A loader that fails
  is a finding naming it. `expect.text` is `{ contains: [..] }` or the exact
  text, against `page_text`.
  - **Bind and reuse.** `bind: <name>` on a step keeps its command's result;
    `$name.path` (`.key`, `[n]`) anywhere in a later step's `input`, the
    `params` or an `expect` is that value, its type kept (`$any` is never a
    name). A name nothing bound is a finding that says to bind it.
  - **Scenarios.** `steps:` runs after the first check (no top-level
    `expect`: no first run): `{ given: [..] }` changes the state, `{ expect,
    params?, bind? }` runs the page again — `params` over the example's,
    `bind: { <name>: loaders.<loader>.rows[0].<column> }` keeping a value of
    that run — and findings name the step (`steps[3]: …`).
  - **Materialized datasets are current when read.** The throwaway runs
    the change loop as boot does (`datasets_changed::spawn`), and before a
    read — a page run, a query, a command (`Host::current`) — after any
    step changed something, every asset recomputes at once, each after
    those it reads, a clocked one (`every: 1h`) too, without waiting out
    the coalesce window (`Assets::recompute_all`): a test has no clock to
    wait on. Once per read, not per step, so it costs the run nothing
    measurable.
  - **Inside embeds and mounts.** `loaders: { <embed>/<loader>: .. }`
    names a loader of the page an embed node or a slot mount runs (a
    mount's id is its page's with `/` as `.`: `beta.extra/words`), so a
    mounting extension's fixture runs the host page (`when: { page:
    host/items }`) and checks what it mounted (`loader_at`).
  - **A step acts as a person would** (`PageAt::act`): `{ press: { action,
    node?, row?: { <column>: value }, input?, confirmed? } }` presses one of
    the page's actions on an open run (`run_page_action`), `{ submit: {
    node, input, confirmed? } }` submits a form node (`submit_page_form`);
    both run for real and settle. What it gave — `{ ok: true, result }` or
    `{ ok: false, refused }` — is checked against the step's `expect`;
    without one, a no is a finding. `bind` reads it (`result.<key>`).
  - **Every kind has them** (`scenario`): a fixture that isn't a page's
    runs its `given` steps first, its `when` and `expect` read the values
    they bound, and each `{ when?, expect }` step runs the same kind
    again, its `when` keys over the fixture's. A step whose `when` names
    another kind (`kind_of`) runs that instead — an event, then a `sql`
    read of what it did — and in a page's scenario a step with a `when`
    runs it (another page, a query, a command) and the scenario goes on
    (`step_run`).
  - **`confirmed: true`** runs a command that asks a person first.
  - **What a step sets off lands before the next step.** After each
    command step the host settles (`Host::settle`): every async consumer
    hears what was logged (`EventPump::settle_all`) — the config reactors,
    registered as at boot, among them — and a generation one asked for is
    published (`ExtensionActivation::serve_pending`, what the owner does
    after its debounce). So `oxplow.capability.choose` is in effect for
    the next run, as it would be for a person.
  - **One oxplow per example that makes data.** An example with `given`,
    `steps`, `rows` or `setup`, or one that runs a job, logs an event or
    files a component's items (`makes_data`), runs on a fork of the
    throwaway
    (`Host::fork`: its files copied, and its database as the deploys left
    it — `Database::copy_in_memory` of a snapshot taken before anything
    ran — so the fork boots what was deployed and nothing is built or
    deployed again; ~0.8 s, most of it that boot's activation), so no
    example sees another's data. Being independent, they run side by side
    (spawned tasks, `side_by_side()`: half the cores, at most four), and
    their findings join the report in the examples' order after the
    shared host's; the CLI's runtime has threads for it. The rest — `check`, the other examples, the answers — share one.
  - **Answers.** After the examples every page's `answers:` entry runs on
    that shared, clean host (`ran: answers`): the page opens with its params
    and no loader fails, else the finding names the page file and the
    question.
  - **The report's cases.** `TestReport.cases` holds a `TestCase { name, ok,
    message, duration_ms }` per thing that ran (`check`, `example <name>`,
    `answers <page>`, `provider <id>`, `policy <id>`), and
    `render(report, Format::Junit)` writes them as one `<testsuite>`.
- `when: { command, input?, as?, confirmed? }`, `expect: { ok, result?,
  commands? }` or `{ ok: false, refused }` — any command run for real
  through the bus as a person (or `as: agent`), then settled
  (`command_run`): what it answered (a script's own answer, unwrapped from
  its composite's `{ result, children }`), and `commands`, the ones that
  ran because of it — the script's steps, then what reacted (audited after
  it). The extension's own command with `answers` is still the dry run
  below, until those fixtures make their data.
- `given: [{ observe: { turn: { prompt?, during?, tools?, write?, answer?, compactions?, context? } } }]`
  (`observe` in `extension_test.rs`; every kind takes `given` and `steps`) —
  what oxplow only learns from what happens around it, made through the API
  core makes it with, and a new kind is added to that `match`, red test
  first, one at a time. `turn` is one turn on the primary thread through
  `hook_ingest.ingest`: `UserPromptSubmit`, the commands made `during` it,
  one `PostToolUse` per tool (`ToolUse` as the harness maps it), the file
  written with a snapshot before and after (`SnapshotTrigger::Startup`,
  `enqueue_startup_diff`), then `Stop` with the answer
  (`last_assistant_message`). The log's projections are delivered between
  the calls and the end (the checkpoint counts the turn's writing tools
  from `v_tool_call`) and settled after it (the checkpoint, then what reacts
  to it). The other kinds: `event` (logged and delivered; effects react),
  `evidence` (`EffortEvidenceStore::replace`), `commit`
  (`SqliteGitStore::upsert_commit`), `change` (a change's files and
  functions, `SqliteChangeStore::store_results`; it never runs a deep
  analysis, and the materialized datasets over the commit index, like
  `co_change_pair`, fill on a recompute the throwaway doesn't start),
  `test_run` (a `tests` capture and its `oxplow.test_case` facts through
  `SqliteFactStore::record_facts`) and `snapshot` (one take in the
  snapshot log, `SqliteSnapshotStore::record_take`), `diagnostics`
  (`SqliteDiagnosticStore::replace_file`), `symbols` (`SqliteSymbolStore::record`
  at a snapshot taken with the file), `collect` (`run_effort_collectors`) 
  `run` (`extension_run_store::record_tx`; `days_ago` through
  `RunContext::began_at`), `health` (`disable_tx`), `token_usage`, `hint`
  and `nudge` (their stores), a `project:` given (keys of
  `.oxplow/project.yaml`, then `reload_config_from_disk`), and a `turn` that
  also takes `thread`, `session`, `prompt: null` + `subagent` (a hand-back)
  and `pre: true` on a tool; none can set the time of a hook (a tool call
  or a turn is as old as the run), only a `run`'s `days_ago`,
  `analyze` (`change_analysis::ensure_change`: a working or commit change
  analyzed to the end; real files come from `turn.write` and real commits from
  `oxplow.vcs.stage` / `oxplow.vcs.commit`, a throwaway's forks keep their
  streams in their own folder),
  `refs` (branches, tags and stream heads, `replace_branches` /
  `replace_tags` / `replace_stream_heads`),
  `visit` (a page the person opened, `PageVisitStore::record`),
  `facts` (findings of a declared measure through `record_facts`) and
  `retention` (`event_retention::sweep`, run `days` ahead). A step's `when: { sql }` reads the result, since a `steps` entry
  of a command, event or sql fixture takes the earlier kind in `KINDS`.
- `when: { advisory: <id>, thread? }`, `expect: { lines: [..] }` — the
  extension's advisory run for a thread (the primary one when none) as
  the app runs it on its `on:` (`advisories::for_thread`, over the
  thread's open effort), its text line by line, heading included; on an
  oxplow of its own, since it records what it said (`once_per`).
- `when: { sql, params? }`, `expect: { rows, columns? }` — a read-only query
  over the published datasets, run as a page loader would (an inline page of
  one `sql` loader, `sql_example`) and compared as a loader's: a dataset, a
  decorator's labels or a metric's `metric_grid` tested on its own.
- `when: { collector: <id>, rows?, event? }`, `expect: { entities: { <name>:
  n | $any } }` — a derived collector runs over `rows` (standing in for
  its `input` query, `oxplow_preview_collector(…, event, rows)`) and `event`
  (the event an `on:` collector reacts to, as the envelope), storing nothing, its
  rows typed against the declaration; a collector that calls a model
  (`ai.compute`) runs in the throwaway against the fixture
  model (`model: { replies }`, in the form the role expects, e.g. `{ answers:
  { label: { type: choice, choice } } }` for `ai_classify`); an exec or
  provider-reading collector's example is a warning, not run (it runs a
  program);
- `when: { collector: <id>, files: { <path>: <text> } }`, `expect: {
  facts: [...] }` — a **fact** collector (one with `facts:`) runs over
  `files` as its tree (`MetricsService::preview_fact_collector`, its other
  input keys — `report`, `rows`, `event` (made the envelope) — as its
  input), storing nothing;
  each fact is compared on the keys its expected fact names (it is not run as an
  entity collector);
- `when: { command, input }` — a provider's, run in its session
  (providers.md "The conformance kit");
Every example's `event:` is built by `EventView::fixture` through the
throwaway's vocabulary (core's plus the extensions' declared types,
synced at its start) — "The event envelope" above: the script gets the
envelope the runtime would hand over, and a fixture that couldn't happen
is an error at it (``fixtures/x.yaml:1: example `x`: event: …``). A
review reads each fixture through that version's own declared types
(`Vocabulary::with_declared`), so a version that adds v2 carries its v1
fixtures to it.

- `when: { effect: <id>, event: { type, payload, subject? }, answers? }`,
  `expect: { commands: [names] } | { skip: <part of the reason> } | {
  reacts: false }` (P8.D12) — whether the effect reacts (`on`/`where`),
  then `effects::dry_run`: its script, its scope calls answered
  from `answers` (per scope, in call order) or for real, the
  composed calls checked against the registry; nothing runs. An
  extension command's is `when: { command: <id>, input, answers? }`,
  `expect: { commands?, result? }` (each compared on the keys it names:
  a read command's example checks what it answers) or `{ refuses }`;
  `{ skip: $any }` matches any skip (`reaction_problem`, shared with the
  policy's);
- `when: { implementation: <id>, event: { type, payload, subject?,
  anchors? }, answers? }`, the same `expect` but `reacts` — an
  **effort policy script** (`implementations:` with `capability:
  effort_policy`): the event envelope, `effects::run_over` with the answers, the
  calls checked against the registry; appending events is an error (a
  policy emits nothing). Naming no policy script of the extension is an
  error naming the id. For each such script `test` also runs the
  effort-policy suite (`policy_suite`): on a fork of the test's
  throwaway (as a provider's suite runs), the
  script chosen as `activeProviders.effort_policy`, then
  `effort_policy_conformance::suite`; a finding is an error at the
  manifest, `ran` shows `effort_policy suite`;
- `when: { event_type, v?, payload }`, `expect: { valid: true | false,
  upcast?: <payload> }` — the payload against the extension's declared
  schema (the newest version without `v`), and its upcast;
- `when: { wikilink }`, `expect: { uri: <canonical URI> | null }` — what
  `[[…]]` names with the extension's resources beside core's;
- no fixture is a warning; an `input` naming none of these is an error.

Then `questions.yaml`, each provider's conformance kit, and (P8.B5) each
`materialize: { incremental: <column> }` dataset on its fixture
`fixtures/dataset-<name>.yaml` — `before:` and `after:`, each `{ <table>:
[{ column: value }] }`: in a rehearsal (rolled back), `before` is written
and the dataset built whole into a temp table keyed like its `m_<view>`,
`after` is written, the rows past its watermark are appended, and what
it holds is compared with its SELECT's rows now
(`datasets::incremental_matches_full`). A difference is an error at the
dataset's line in `extension.yaml` ("misses n row(s) a full refill holds"
— a watermark that doesn't grow as rows arrive drops a row below it
unseen, which nothing at run time can tell); an append that hits the key
passes (the runtime refills then); no fixture is a warning.

## Pages

A **page** is a user- or agent-built way of looking at your work: named
SQL queries over the semantic layer, plus a tree of nodes that shows them.
(Not "view", which is taken by the SQL `v_*` views.) A page is one file,
`pages/<slug>.yaml`, in an extension; its id is `<extension>/<slug>`. The
extension manifest lists no pages: the folder is the list. A source with
the old `lenses/` folder does not load.

The reader's guide is `docs/guide/pages.md`; the agent's is the shipped
`oxplow-extension` skill (`references/page-format.md`, `loaders.md`,
`view-nodes.md`). `.context/pages-and-tabs.md` "Page files" covers how the
desktop draws one. This section is the contract: what a page file takes,
and what loads, runs and checks it.

**Three parts** (`pages/schema.rs`, `parse_page`; `deny_unknown_fields`, so
a typo is an error at the line):

- **Metadata**: `title` (required), `description`, `label`, `icon`,
  `category`, `keywords`, `params`, `for` / `accepts` / `default` /
  `pinned` (a renderer), `comments`, `refresh`, `hidden`, `needs`,
  `actions`, `alert`, `tool`, `answers`, `badge`.
  A page's `description` says what the page is (what it shows, for
  what, over which scope and order), not what you can do on it. A page
  whose purpose is an action (a form to file an item, hand off work or
  verify a claim) may name that action; never list UI mechanics ("drag
  to reorder", "right-click for the rest") or the actions on offer.
- **`loaders:`**: named loaders ("Loaders" below).
- **`view:`**, and an optional `rail:`: a tree of nodes ("View nodes").

Where it lives, and what is loaded:

- **Origin** (`Page.origin`, `PageProvenance`): its `extension`, the
  `running` file (a deployment's copy in the store, or the source when
  loaded from it), the `deployment` hash, `bundled`, and `source` — the
  project-relative file to edit, none when no source is here (filled per
  extension by `locate`). An agent edits `source`, never `running`: the
  next deploy replaces the copy. `oxplow_get_open_page` adds `outOfDate`
  (`PageProvenance::out_of_date`, read from the files now: the source
  changed after the deployment was made). `v_page` has `source`, not the
  running path, and `running_uri` (V123; `PageProvenance.running_uri`, filled in `locate` by
  `filesystems::extensions::page_running_uri`, the one builder): the file URI of the running copy,
  `oxplow:oxplow/file?path=<extension>/pages/<file>&rev=<hash>&filesystem=oxplow:extensions`
  (built with `file_uri_at` from `origin.deployment`, so it pins the copy a
  redeploy replaces); NULL when the extension runs no deployment (read from
  its source, undeployed, not here). The same files are served by the
  `oxplow:extensions` filesystem (filesystems.md); `locate` and that
  filesystem both take the running hash from `Deployment::hash`. A deployed extension's errors and warnings name its source
  files, so they say what to edit.
- **Definition in About this page** opens the page's running file (`v_page.running_uri`)
  drawn as foundation's `definition` page, a display mode of a page's YAML
  file (`for: oxplow/file`, `accepts.content` over `oxplow.page.describe`):
  the page part by part (where it comes from, params, loaders, nodes,
  actions, answers), with its Source a link to the
  same running copy in the File viewer (the raw YAML). A page with no running copy has no Definition row
  (pages-and-tabs.md).
- **An inline page** (`parse_inline_page`: a markdown ` ```oxplow-page `
  block) only reads: no `command` loaders, no actions, forms, terminals,
  editors, components or slots. An agent's `oxplow_show_page { spec }` is a
  **temp page** (`parse_temp_page`): the same rules, `for:` refused, kept as
  the page file `<id>.yaml` on `oxplow:temp` and run as `temp/<id>`.

### Metadata

- `label` is the tab's name: `{{param.x}}` and `{{row.col}}` (the first
  authored loader's first row, else `content`'s), filters `|short`, `|basename`, `|url`. Any other
  `{{…}}` is refused (`label_refs`).
- `category` is the launcher section (`LauncherCategory`); none means the
  page opens only from elsewhere. `hidden` keeps it out of the launcher.
  A page with `for:` is a resource page, a display mode of its resource:
  it takes none of `category`, `hidden`, `keywords`, `badge` (load
  errors) and must declare a `uri` param. It is handed a `content`
  loader first, configured by `content: { text, max_bytes }`
  ([pages-and-tabs.md](./pages-and-tabs.md) "Resource pages and standalone pages").
- `needs: [capability]`: a page whose needs are not met runs nothing.
  Its `PageRun.inactive` (`PageInactive { needs, message }`) says what it
  needs, and the page shows that instead of an error.
- `tool: <name>`: the page is also an MCP tool `<namespace>_<tool>`, its
  input the params, its call `oxplow.page.run`. Kept for the few questions
  agents ask every session (foundation's `thread_work`, `work_next`,
  `effort_summary`, `catalog`); everything else is `oxplow_find_page`,
  then `oxplow_run_page`.
- `answers: [{ question, about?, params, skill? }]` (`PageAnswer`): the
  questions the page answers. `oxplow_find_page` scores shared words:
  an answer's question x3 (the best one), the title x2, keywords and
  description x1; ties go by title; `limit` defaults to 10 (1 to 50).
  The tests are `find_*` in `agent_reads/page_tools.rs`. The
  pages of `about`'s resource offer them to the person; `oxplow extension
  test` opens the page with each answer's `params` and fails the page when
  it does not open or a loader fails. `skill` (oxplow's own extensions)
  names a shipped skill whose text must name what the page reads.
- `badge: { loader, column? }`: a count on the page's nav entry (that
  loader's first row, else its row count). `alert`: `{ min_rows }` or
  `{ column, below | above }`, with a `label`, and the page marks itself
  while it fires; it watches the badge's loader, else the first.
- `for` / `accepts` / `default` / `pinned`: a renderer, see
  [pages-and-tabs.md](./pages-and-tabs.md) (`renderer_problem` checks
  them at load). `comments: true`: the person
  may comment on the page's text. `refresh: true`: a Refresh button.

### Params

A param (`PageParam`) takes `name`, `label`, `default`, `options`,
`options_from`, `type`, `presets` and `role`.

- **Types** (`ParamType`, `PageParam::take`): `text` (bound as-is; the
  default with no `options`), `choice` (one of `options`, or any text when
  `options_from` names a page that lists them; the default with either),
  `bool` (`true`/`false`/`1`/`0`, bound 1 or 0), `search` and `branch`
  (text, trimmed, NULL when empty; `branch` NULL means every branch), and
  `date_range`. A loaded page always says its type, so the desktop never
  infers it.
- **A `date_range`'s value** is one text: a preset (`1d 2d 3d 7d 30d 90d
  all`; `presets:` picks the quick picks, default `1d 7d 30d all`) or
  `<from>..<to>` of dates or RFC 3339 times, an end empty for open. Tab
  ids, `run.params` and agents see that one value. The runner binds
  `:<name>_from` / `:<name>_to` (RFC 3339, NULL for an open end; a date's
  `to` is the next day, so the day counts) just before a query runs, never
  as params.
- **Load checks** (`param_problem`): `options` or `options_from` off a
  choice; both on one; a choice with neither; a choice with fixed options
  and no `default`; `presets` off a date_range; an unknown preset; a
  default its type does not take; `role: selection` written; a param named
  like a view key (`RESERVED_VIEW_KEYS`: `section`). A run that supplies
  an unknown param, or a value its param does not take, is refused
  (`resolve_param_values`). Every param draws in the page's rail.
- **`role`** (`ParamRole`) says where its value rides in the page's URI.
  `resource` (the default) names what the page shows, so it is in the `?`
  and a change opens another resource. `filter` narrows or re-views what
  is shown, so it is in the `#` and a change replaces the entry; a
  `switch`'s param is a filter by default. `selection` is derived, never
  written: a column `select`s the param, the rail shows the picked row
  instead of an input, it is in the `#`, and a pick pushes an entry. `view`
  rides the `#` like a filter but is state the page or the system sets:
  the rail never draws it (not in Filters, not counted for Reset) and a
  person never edits it; a `for:` page may take one (`ParamRole::rides_view`
  is the one test for "rides the `#`"). A `filter`, `selection` or `view`
  param found in a `?` is refused.
- **`options_from: <page>`** (a slug of the extension, or
  `<extension>/<slug>`; `page_ref_problem` checks it exists and is not the
  page itself) makes a choice whose options a page lists: its rows are
  `value`, `label`, optional `group`. The rail runs that page with the
  params it declares, so an `end` picker's page can leave out what `start`
  is. No `default` is needed and none is checked; a run takes any text,
  and the query says what an unknown value means. An agent's page tool
  (`agent_tools::params_schema`) emits no `enum` for it. A param named
  `uri`, `effort_id`, `thread_id` or `stream_id` draws as a picker over
  the matching dataset, by title (`viz/vizParams.ts`); any other is typed.
- **Implicit params.** A param named `stream_id` or `thread_id` is bound
  to the viewer's context (`ViewContext`, numeric ids) unless the caller
  supplies it. Precedence: supplied, then context, then `default`.
  `view_context(svc, stream?, thread?)` resolves it: the given stream
  (else the thread's, else the primary) and the given thread (else the
  stream's selected-or-first one). IPC `open_page` and `run_page_action`
  resolve it from their `stream_id`; MCP `oxplow_run_page` also takes the
  agent's `thread_id`; `oxplow_get_open_page` uses the viewer's thread.
  A dry run (a check, `validate`) uses no context.
- **`head_sha`.** A param so named is, unless supplied, the viewed
  stream's HEAD commit (`v_stream.head_sha`, `PageRunner::params_of`).
  The run reads `v_stream` for it (`context_reads`), so a commit or a
  branch switch re-runs the loaders that bind it.

### Loaders (`loaders:`)

Named loaders, run in order; a loader reads only the ones before it
(naming a later one is an error). A loader has exactly one of `sql`,
`command`, `page` or `rows`, plus optional `columns` (keys, labels and
formats its nodes default to) and `empty`. Its name is lowercase letters,
digits and `_`, never a dataset's (`v_*`, `m_*`).

- `sql`: one read-only statement over the `v_*` datasets and the extension's
  own. `:param` binds a page param or a loader `params:` entry (`{ param
  }`, `{ from: <loader>.<column> }` (an earlier loader's first row) or a
  literal). An earlier loader it names reads as a temp table of that name
  (`SqlQuery.temp`). `config('/key')` reads the page's own extension's
  settings (`PageRunner::with_config`).
- `command`: a read-only command, run as the viewer
  (`Access::reads_only`). It says `reads` (datasets) or `refresh` (`manual`,
  `{ every }`). Its result is a `{ columns, rows }` table as is, an array
  of objects (a row each), else one `value` row; `items: <field>` takes a
  list inside the result as the rows.
- `page`: another page's loader (`{ page, loader, params }`). Pages nest 8
  deep (`MAX_DEPTH`).
- `rows`: inline rows, the first row's keys the columns.
- `content` (never declared): a resource page's first loader, the record its
  `uri` names; read as `FROM content` / `{ from: content.<col> }`. The name
  is reserved: no loader or param is called `content`, and a standalone page
  can't read it.

**Running one** (`pages/runner.rs`, `PageRunner::run`): params resolve as
above. Loaders run in order. A loader that fails carries its `error` and
the rest of the page still shows. Within one run, embeds included, a
loader with the same definition and inputs (its `key`) runs once. A loader
only a `switch`'s hidden children show does not run; one no node shows
does (it feeds an embed, the badge or a later loader). A page whose
`needs` are unmet runs no loader (`inactive`). A command loader runs as
the viewer, `Actor::Page { page_id, on_behalf_of }` (the page's URI is the
source of what it records). One run keeps at most 5 MiB of results
(`result_budget.rs`): later rows are cut whole, never half a row, and the
loader says so. Do filtering and `LIMIT` in SQL.

**Open runs** (`pages/open_runs.rs`): a window's open page keeps its run
and the request that made it. A change to a dataset one of its loaders read
(a command loader's `reads`), or to a param it binds (`head_sha` reads
`v_stream`), re-runs the loaders that depend on it (`dirty_loaders`).
`OxplowEvent::PageRunChanged` names the loaders whose rows differ (an
embedded page's as `<embed id>/<loader>`), and the window fetches only
those. A catalog change (a deploy) re-runs every open run
(`page_runs.notify_everything()`).

### View nodes

`view:` is one node, usually a `stack`. Every node has an `id` (default:
its loader, embedded page, slot or component; else `stack`, `stack-2`, …),
unique in the page. `#node=<id>` in a link lands on it. `title`,
`replaceable` and `size` are on any node. An embedded page's nodes are
`<embed-id>/<node-id>`; `oxplow_run_page` text prints each node under
`[node <id>]` (`pages/text.rs`).

- `stack: [nodes]`; `grid: [nodes]` (a child's `size`: `small`, `wide`,
  `tall`, `full`).
- `switch: { param, children }` shows the one child whose id is the
  param's value (else the first).
- `embed: { page, params }` shows another page inline. Its params flow by
  name: the embed's `params`, else the host's param of that name, else
  that column of the host loaders' first rows, in order. Its page actions
  run on the embedded run (`pages/actions.rs`, `locate`).
- `slot` and `component`: below. `markdown: "text"`: static text.
- `viz` (+ `from: <loader>`, a role block, `columns`, row `actions`,
  `empty`): a `VizConfig` with no query of its own. A viz that reads no
  rows (`form`, `approvals`) has no `from`. The YAML only binds; logic is
  SQL or a read command.

**Vizes** (`Viz`): `table`, `list`, `number`, `markdown` (the first row's
`markdown: { text: <column> }`, else its first column, drawn by the one
Markdown component, `.context/markdown.md`; with an `edit` action it is
that component's editor, saving through the action), `bar`, `line`,
`treemap`, `tree`, `timeline`, `detail`, `steps`, `hunks`, `board` (the
groups as columns of cards, each a list row; needs `group`, whose
`values: [..]` fix the columns' order and keep empty ones as drop
targets), `terminal` (a terminal per row, `terminal: { of, title?,
stopped?, conversation? }`; `.context/terminal.md` "The terminal viz and
its host"), `editor` (the working tree's file, edited: `editor: { path,
lines? }`; saved through `oxplow.file.save`; `useTabEditing`;
`.context/editor-and-monaco.md` "The editor viz"), `embed` (a web page:
`embed: { url }`, the first row's `http(s)` URL, in oxplow's sandboxed
window; `.context/external-url-tabs.md`), `code` and `diff` (read-only:
a file at a revision, or one file's diff; `.context/editor-and-monaco.md`
"Read-only code and diff vizes"), `approvals` (core's consent component:
it reads `v_command_proposal` itself and decides through the approve and
decline commands; params `thread` and `setting` narrow the list), `form`
and `custom` (below). `terminal`, `editor`, `embed`, `form`, `approvals`
and `custom` are refused in an inline page. A viz that fills its page
(`code`, `diff`, `editor`, `terminal`, `embed`) draws no in-body title.

Each component names the columns it draws from in its own block: `chart`
(`x`, `y`, `series`, `label`, `size`, `group`, and a `line`'s `targets`: a
target line from that column's last non-null value), `tree` (`id`,
`parent`, `label`; lazy: `lazy`, `branch`), `timeline` (`at`, `label`,
`uri`), `steps` (`label`, `status`), `hunks` (`path`, `from`, `to` — two
revisions, `working` / `snap:<id>` / `git:<rev>`), `code` (`path`,
`revision`; optional `lines`, a column holding `12-40` or `12`, marked and
revealed, and `sync: true`), `diff` (`path`, `from`, `to`, optional
`lines` and `base`; the first row only). `role_columns` lists every
result column a block names; `extension check` and `validate` fail a page
whose loader does not return one (`viz_column_problems`). An agent reads
`code` as each file's lines in a fence and `diff` as its unified diff
(`viz_text`).

- **A lazy tree: `tree.lazy: <param>`, `tree.branch: <column>`.** A tree
  whose loader returns one level (a folder of `v_file`, `WHERE
  parent = :folder`): `lazy` names the page's own param the level is read
  with (it needs a `default`, the top level, and cannot be one the context
  supplies), `branch` the column that is truthy on rows that open. A
  branch opens by running the page again with the param set to the row's
  `id`, its rows spliced under it (`useLazyBranches`). A column whose
  `select` names the lazy param is refused (`lazy_tree_problem`).
- **A `form` node** is one command's input: `form: { command, defaults?,
  fixed?, fields?, mode?, choices?, then?, again? }`. `defaults` takes
  `{{param.x}}`; a `query`-less form prefills from its loader's first row.
  `fixed` fields show their `defaults` value, imposed again on submit.
  `fields` lists which to show, the rest keep their starting values; an
  entry is a name or `{ name, visible_if?, secret?, from? }`:
  `visible_if: { kind: [http, https] }` shows the field only while each
  named field holds one of the values (hidden, it is neither required nor
  submitted); `secret: true` masks it, keeps it out of any draft, and
  leaves it out of the input when blank. A command input field whose
  schema says `writeOnly: true` is masked without the flag
  ([commands.md](./commands.md) "Secrets"). The command must keep a secret
  out of its record (`unrecorded`), or the form refuses to open
  (`form_start`). `mode: edit` edits what is saved: it starts from the
  saved values, Save is enabled only when the form is changed, Discard (or
  Escape) puts them back, and it lists its `fields`. `create`, the
  default, gathers something new. `choices` maps a field to a page of the
  extension, run with the form's params, whose rows (`value`, `label`,
  optional `group`) are the field's choices. A field `{ name: native,
  from: <page> }` is an object whose children that page names, one row per
  child — `name`, `title`, `kind` (`enum`, `text`, `number`), `values`,
  `read_only` — and its value is the object of those children; the form
  reads again when what the page read changes (`viz_actions::subfields_from`).
  `again: { label, keep: [...] }` adds a second submit ("Save and
  Another"). `form_fields_problem` and `page_ref_problem` are the load
  checks.
- **`custom` and `component`.** `viz: custom` (`custom: { component,
  props? }`) and the `component: { id, props }` node draw one of the
  extension's `custom_components` in a sandboxed frame, fed the page's
  loaders. "Custom components" below. A name the extension does not
  declare is a load error (`component_problem`).
- **A `slot` node** is a place other extensions mount pages at. See
  "Slots".
- **A `rail:` tree** draws in the page's rail; a row it picks sets the
  page's view param.

**Columns** (`PageColumn`): `key`, `label`, `link`, `unit`, `icon`,
`tone`, `width`, `format`, `highlight`, `select`, `expand`.

- `link: { kind, from, line?, base?, head? }` makes a cell a link
  (kinds: `task`, `file`, `wiki`, `effort-diff`, `commit`, `metric`,
  `page`, `diff-at`, `compare`). `page` opens any oxplow URI. The pages
  a column's links open are the page's **origin**
  (`viz/linkOrigin.ts`, `.context/pages-and-tabs.md` "Origins"): Up and
  Down step through the rows that have one.
- `select: <param>`: a click sets that param to the column's value, so
  whatever reads it re-runs (master and detail). It names a declared
  param (`highlight_problem`).
- `highlight: <param>` marks every case-insensitive occurrence of the
  param's value in the cell's text. `expand: <page>` puts a ▸ in the cell
  that opens that page under the row, with the row's same-named columns
  as its params (`page_ref_problem`).
- `unit` names another column of the row holding the unit, so a number
  shows as a metric value (`16.4 s`, `42.5%`).
- `icon` and `tone` each name another column of the row. `icon`'s value
  is an icon drawn before the cell from a fixed vocabulary
  (`viz/vizIcons.tsx`): the work glyphs (`ready` / `todo`,
  `in_progress`, `blocked`, `done`, `canceled`, `archived`), a file's
  status (`added`, `modified`, `deleted`, `renamed`, `untracked`,
  `conflicted`), the kinds (`epic`, `task`, `wiki`, `file`, `folder`,
  `commit`, `diff`, `metric`, `dashboard`, `comment`), the resource icon
  names (`extension_resources::ICONS`), and any page kind's icon. `tone`'s
  value is `accent`, `success`, `warning`, `danger` or `muted`. Values
  come from the query, so the loader cannot check them: an unknown name
  draws nothing, an unknown tone is plain.
- `width` is pixels (a table lays out fixed; cells are cut to one line).
  `format` (`ColumnFormat`): `date`, `datetime`, `relative`, `bytes`,
  `duration` (ms), `integer`, `percent` (a 0–1 fraction), `sparkline` (a
  JSON array of numbers), `badge` (a pill in the column's `tone`). It
  changes only what a person sees; an agent reads the raw value.
- A timestamp cell shows in local time, its full time on hover.

**Row styling** (`row_style_problem`), each naming a result column:
`group: { by, link?, values? }` (`list` / `table` / `board`) puts rows
under a heading per distinct `by` value, in the order each first appears,
folding (▾ / ▸) and counting its rows; `emphasis` (`list` / `table` /
`tree`) highlights rows with a truthy value; `depth` (`list` / `table`) is
a whole number 0–8 that indents. They are never shown as cells.

A `list`, `table` or `tree` row whose first linked column resolves is
draggable into the agent's context (`CONTEXT_REF_MIME`); the row menu has
Ask About This. A table or list is keyboard-driven (`useListNav`: ↑/↓,
Enter opens the cursor row's first link or runs its first row action;
usability.md "Keyboard").

### Actions

`actions:` on a page or a viz node: `{ id, label, command | open, input, row?,
group?, form?, fixed?, choices?, then?, when?, primary?, prominent?,
submenu?, icon?, drag?, bulk?, key?, edit?, confirm? }` (`PageAction`).
`input` binds `{{param.x}}` and, on a row action, `{{row.x}}`
(`{{row.x|json}}`: the JSON a text cell holds, typed; the one filter,
checked at load). A page action with `when` or `{{row.*}}` and no `row`
reads the page's first loader's one row. A drag action also binds
`{{before.x}}`, `{{after.x}}`, `{{onto.x}}`, `{{group.value}}`, and
`{{order.x}}` (a list of column `x` in the new order).

- **`open: <column>` is an action that is not a command**: exactly one of
  `command` or `open` (`parse_actions` refuses both and neither). The column
  of the row the action is offered on (a whole-page action's first-loader
  row, a holds-one-row viz's, a row action's) holds the URI to open. Nothing
  runs: no bus run, no audit, no `command.executed`. The press still goes
  to the server (`viz_actions::prepare_resolved` → `Prepared::Open`), which
  checks the row and `when` as for a command, and answers `{ "opened": <uri> }`;
  the desktop then opens it (`performVizAction`, as `open_after` does) and
  an agent's `oxplow_run_page_action` reads it. An `open` action takes no
  `input`, `form`, `fixed`, `choices`, `drag`, `bulk`, `edit` or `group`.
  The extension check names a column the loader lacks (`when` and `open`
  alike, for page and viz actions). `PageAction.command` is nullable in the
  bindings; `actionDoes` (vizActions.ts) is what an action's title says
  (`opens <column>`). The File viewer's "Open current version" is one:
  `open: current_uri` over its loader's column.
- An action runs as the page for whoever pressed it
  (`Actor::Page { page_id, on_behalf_of }`; `pages/actions.rs`,
  `viz_actions::prepare_resolved`), so it grants no power. A press names
  the open run (`run_id`), the node and the action; a row action names a
  row, and the server checks it is one of the node's rows. An agent
  presses one with `oxplow_run_page_action`; a command it may not run, or
  that needs a person's confirmation, is refused.
- `form: true` opens the command's form instead of running (a slideover,
  or, when the form less its `fixed` fields is one text field, a prompt
  strip above the rows). What the person fills in runs, its `fixed` fields
  put back to the bound values. A refusal naming a field (`INVALID` with
  `field`) shows under that field. A run that came back undoable toasts
  with Undo.
- `primary: true` (a row action) is what a click on the row, or Enter,
  runs: the first one the row is offered (`when`). `prominent: true`
  keeps a page action a button in the nav bar; the rest go in the ☰ menu
  (`submenu` groups them). `bulk` runs once per marked row; `key` runs on
  the cursor row; `group: <value>` puts the button in that group's
  heading (it needs `group` on the viz and binds only `{{param.x}}`).
- An action's command may declare where a person goes after it
  (`ui.open_after`, [commands.md](./commands.md) "Offering a command to a
  person"): the page opens it once the action ran, bound from the run's
  result. What becomes of the acting page is `then`: `open` (default; Back
  returns), `replace` (it leaves history), `stay` (nothing opens) or
  `close`. Outside a page (a panel) a page opens in a new tab.
- `edit: true` makes an action the Save of a `markdown` viz's editor. Its
  one input field bound to `{{row.<text column>}}` and not `fixed` takes
  the text (`markdown_problem`).

### Checks

- **Load** (`parse_page`, then the extension's load): a page file's own
  errors at `file:line`; each `options_from`, `expand` and form `from`
  page is one of the extension's (`page_ref_problem`); a `component` node
  or `viz: custom` names a declared `custom_components` entry
  (`component_problem`); a `replaceable:` node names a target in the
  table, placed once per page (`check_page`); a renderer's `for`,
  `accepts`, `default`, `pinned` and `label` fit its resources
  (`renderer_problem`); a slot mount names a page of the extension and a
  `<extension>/<page>.<slot>`. A page that fails is dropped, with its
  error on the extension.
- **Layout warnings** (`pages::checks::layout_warnings`): load, validate,
  `v_extension.warnings` and a deploy's result carry them, each naming
  its page file and the fix after the dash. A warning never stops a page
  loading.

  | Rule | Limit |
  |---|---|
  | `filter` params on a page (a `switch`'s do not count) | 6 — fold related ones into one `choice`, or split the page |
  | options of a `choice` | 12 — use a `search`, or a page that lists them |
  | a param no loader of the page reads (`:x` whole token, a date_range also `:x_from` / `:x_to`, an action's `{{param.x}}`, a column that `select`s or `highlight`s it, a form's defaults, the `label`); not checked on a page with an `embed`, `slot`, `component`, `replaceable` node or `viz: custom`, which pass the params on | none — remove it, or read it |
  | description of a visible page (`hidden: false`) | 200 characters |
  | children of the top `stack` | 8 |
  | declared `columns` of a `table` / `list` (not on a page with `tool:`, whose rows are for an agent) | 8 — put the rest in a node a row `select`s |

- **`extension check` dry-runs pages** (`pages::checks::dry_run`): each
  `sql` loader runs once with the params' defaults (an earlier loader
  reads as a table), so a query that fails, and a viz node naming a column
  its loader does not return, are found before deploy. `command` and
  `page` loaders do not run (a check acts as no one), so a node they feed
  is not column-checked. `pages::checks::text` renders the dry run as
  text; the impact review compares it between two versions ("Reviewing by
  impact").
- A slot mount whose host page is not loaded, or has no slot by that
  name, warns (`"… shows nowhere"`).

### Custom components

A page's `component` node and `viz: custom` run a web bundle from the
extension's `custom_components`, sandboxed, fed the page's loaders. The
reference is "Custom components" below and `docs/guide/pages.md`. A page
whose view is one `component` node fills the page under oxplow's bar
(a custom page; foundation's Timeline is the example).

### Agents

`oxplow_find_page { question }` (answers, then titles and descriptions),
`oxplow_run_page` (the page's text and rows; up to 200 rows a loader as
JSON, 50 as text, with `truncated`), `oxplow_run_page_action`,
`oxplow_show_page` (an inline `spec`, or an existing page with params,
beside the conversation), `oxplow_get_open_page` (the page's tree, the
viewer's filters and picks and the focused node). `oxplow_validate_extension`
checks a page before deploy, and `$OXPLOW_BIN extension test <name>` runs
its fixtures.
`v_page`, `v_page_answer` and `v_page_param` list the loaded pages
for SQL.

## Slots

A **slot** is the only way an extension reaches another extension's page.
A `slot` node in a page's `stack` or `grid` (`- { slot: sections }`) is a
slot named `<extension>/<page>.<name>`; with nothing mounted it draws
nothing. A mount is an entry in the mounting extension's manifest:

```yaml
ui:
  mounts:
    - { page: item-tokens, at: oxplow-foundation/work-item.sections, position: { before: links }, title: Tokens }
```

`slot` is the host's slot name; `page` is a slug of the mounting
extension; `at`, `title` and `size` are optional. The mounted page runs as
an embed (`PageRunner::run_mount`): its params by name from the host's
params and its loaders' first rows, so it declares the ones it needs. Its
node id is the mounted page's id with `/` written `.` (`beta.extra`), and
the desktop marks it with the mounting extension's name. A disabled
extension mounts nothing.

- `at` (`SlotAt`, untagged because serde_yaml reads tagged enums only as
  YAML tags): `start`, `end` (the default, at the slot node) or `{ before:
  <sibling id> }` / `{ after: <sibling id> }` beside a sibling of the slot
  in its parent stack. The id is the contract, so a mount survives the
  host's other code changing. An `at` beside an id the slot's parent does
  not hold shows nowhere: the runner drops that mount. `size` (`small`, `wide`, `tall`, `full`) is for a
  grid host.
- Load errors at the mount's line (`extensions.rs`, `slot_files`): the
  page is not in `pages/`; the slot is not `<extension>/<page>.<slot>`.
  Load warnings: the host page is not loaded, or has no slot by that name
  (the mount "shows nowhere").
- A mount's contribution id is `<host>.<slot>-<n>` with the host's `/`
  written `.` (`PageSlot::local_id`). Slot mounts are `ExtensionUi.page_slots`.
- A top-level `slot_mounts:` / `decorators:` / `replacements:` is an
  unknown field: they live under `ui:`.

The slots foundation's pages declare (each `slot` node is one; its page's
loaders' first rows and params are what a mount can take):

| Slot | Host page | Params a mount may take |
|---|---|---|
| `oxplow-foundation/diff-view.sections` | the Changes page (an effort's or a range's diff; nodes `effort`, `header`, `description`, `verdict`, `concurrent`, `files`, `extensions`) | `uri`, `start`, `end`, `stream_id`, and the page's row (`effort_id`, `change_id`, …) |
| `oxplow-foundation/work-item.sections` | the Work Item page (nodes `state`, `fields`, `body`, `info`, `links`, `comments`, `activity`) | `uri`, `thread_id`, `stream_id` |
| `oxplow-foundation/tasks.sections` | the Tasks page (nodes `list`, `summary`, `backlog`; `at: { before: list }` puts a strip above the list) | `show`, `priority`, `thread_id`, `stream_id` |
| `oxplow-foundation/commit.sections` | a commit's page (nodes `detail`, `files`) | `uri`, `sha`, `stream_id`, and `change_id` (the page's row) |
| `oxplow-foundation/uncommitted-changes.sections` | Uncommitted Changes (nodes `commit`, `files`; `at: { before: commit }` puts a strip above everything) | `stream_id`, and `change_id` (the page's row) |
| `oxplow-foundation/new-session.sections` | the New Session page (nodes `start`, `earlier`) | `stream_id`, `thread_id`, `q` |
| `oxplow-foundation/diff.file-header` | the Diff page (one file between two revisions; the slot sits above the diff). oxplow-bundled mounts its `file-co-change` page here | `path`, `left`, `right`, `label`, `base`, `lines` |

`change_id` points at the page's change in `v_change`, with its analysis
in `v_change_file`, `v_change_function`, `v_change_import`,
`v_change_co_change` and `v_change_duplicate`. To learn a page's slots
and params, read it with `oxplow_find_page` or `oxplow.page.describe`: its
slot nodes and loaders are listed.

The launcher is not a slot: a page lists itself with `category`.

## Replacements

A `ui.mounts` entry whose `at` names a core component (no `/`; one of
`v_replaceable`) puts a **page in place of that named core
sub-component** — never a whole page. The same list mounts a page at
another page's slot (`at: <extension>/<page>.<slot>`, with `position`,
`title`, `size`); `version` goes only with a core component, the slot
keys only with a slot. The old `ui.slots` / `ui.replacements` keys are
refused, the refusal naming the `ui.mounts` spelling. Panels stay their
own `panels:` declaration: a panel is a thing with an id, title, icon and
scope made of part-pages, not a placement of one page.

```yaml
ui:
  mounts:
    - { page: swimlanes, at: work_item.board, version: 1 }
```

The targets are one table of data, `crates/oxplow-domain/data/replaceable.json`
(read by `oxplow_domain::replaceable::all()`; `fixtures/replaceable.json` pins
its targets so it only shrinks):
each a `target`, the `capability` it belongs to, whether it is **bound to
a provider**, its contract's **version**, its **props contract** — the
params the replacement page is given instead of any host state — and its
**view**: the host page's view params the replacement shares, read
(`init.view`), written (`setViewParams`) and heard (`viewParams`) as the
core component reads and writes them. Only those keys: any other is
refused, and a target with no view gives its replacement none. The table
is published as `v_replaceable` and generated into the reference
(`the_reference_lists_every_replaceable_view`). A declaration's optional
`version:` must be the contract's: a changed contract refuses it rather
than handing it props it does not know.

| Target | Core component | Capability | Bound | Props | View |
|---|---|---|---|---|---|
| `work_item.board` | the Board's columns of cards (the `board` page's node) | `work_items` | no | `scope` (`thread` / `backlog` / `all`), `thread_id` (applies when `scope` is `thread`) | — |
| `work_item.list` | the Tasks page's list: the scope picker, detail pane and backlog drawer stay oxplow's | `work_items` | no | `scope` (`thread` / `stream` / `all`), `thread_id` (the thread when `scope` is `thread`, else null) | — |
| `work_item.detail` | a work item's description editor (the `body` node of the `work-item` page): the state, fields, About, links, comments, activity and the mounts stay oxplow's | `work_items` | no | `uri` (the item) | — |
| `work_item.detail.state` | a work item's State (the `state` node) — never its Move To (the About node's menu), so a replacement cannot strand an item | `work_items` | yes | `uri` (the item) | — |
| `effort.verdict` | an effort's verdict, the first line of its page (the `verdict` node of the `diff-view` page). Core has **none of its own**: with no replacement, or `oxplow` chosen, the page shows nothing. oxplow-bundled's `effort-verdict` is the default | `effort_policy` | no | `effort_id` (the effort's row id) | — |
| `vcs.history.graph` | Git History's commits (a node of the `git-history` page): oxplow's own is a plain list | `vcs` | no | `head_sha` (the stream's head), `reveal_sha` (a commit to scroll to and mark, or null), `limit` | `q`, `author`, `branch` (Git History's filters; v2) |

**Who places a target.** A page node marked `replaceable: <target>`
(`Node.replaceable`; `check_page` refuses an unknown target and a target
placed twice on a page). The node's own content is oxplow's own and the
fallback. **The page run decides** (`PageRunner::replacement`, the rules in
`replacements::chosen`): when a replacement stands in, the node runs it as
an embed (`run_embedded`, as a slot's mount runs) — its props by name from
the run's `params`, a prop the params leave out from the first row of the
page's loaders (the Tasks page's `row` loader derives `scope` from its
`show` param), only the props the replacement page declares — and
`RunNode.replaced_by` names the extension; else the node's own content
runs. So a fixture of the host page sees the replacement
(`loaders: { <node>/<loader> }`). The desktop draws what the run decided
(`PageTree` → `components/Replaced.tsx`: the embed under a badge, sharing
the view its contract lists; a replacement that didn't run shows oxplow's
own and why). The page's params must cover the contract so
the props exist; a page with a `replaceable` node is not warned about
params it passes on.

**Bound or not.** A bound target shows one provider's own thing — the
state control lists that tracker's states — so only the extension of the
provider whose item it shows replaces it. Every other target is a **view**:
the Board, the list and the editor show the capability as a whole, reading
`v_work_item` and changing items through the interface commands
(`oxplow.work_item.update`, `transition`, `reorder`, `move`), so they work
with whichever list is active and any extension may bring one — a view
needs no provider. Choosing a presentation is separate from installing a
work list.

**Whose shows** is the person's: `replacements: { <target>: <extension> |
oxplow }`, one project key validated against the table. An extension
chosen shows (an unbound target only); `oxplow` keeps oxplow's own (any
target); unlisted is the default — the only extension replacing it,
several and no choice keeping oxplow's own, and for a bound target its
provider's extension's. Settings → Integrations lists each replaced
target with that choice (a `<select>`: Default, oxplow's own, each
extension). Every target has a `label` a person reads, in the same table
("board", "state control", "history graph"), carried on each
`UiReplacement`. A state control replacement lists the tracker's own
states, each with **Move Here** (`oxplow.work_item.transition` with the
`native_state`); oxplow's Move To stays beside it.

**At load** (`replacements::parse_replacements`, errors at the entry's
line): the target is in the table; the page is one of the extension's and
declares **every** prop (a replacement takes the whole contract — a Board
that ignores `scope` is not the Board); a pinned `version` is the
contract's; for a bound target, the extension brings a provider of the
target's capability (`providers:`); a target is replaced once per
extension. The viewer's stream is not a prop: a page that declares
`stream_id` gets it, as everywhere. Valid ones are
`ExtensionUi.replacements` (`UiReplacement { id: <ext>/<target>,
extension, target, capability, bound_to_provider, page, version, view,
label }`).

**In the run** (`PageRunner::replacement` → `replacements::chosen`), from
the enabled extensions' replacements of the target, only when one exists:
- **bound**: the **item's own provider** decides, whichever is active, so
  another provider's item never gets a page that sends this one's states.
  The provider is the `provider` param of the node's `uri` param (a work
  item's URI carries it); which extension brings it is the capability
  registry's answer at run time, not at load, so installing or switching
  one needs no reload;
- **unbound**: the person's choice (`replacements`), or the only
  candidate; no provider is read;
- no candidate, a provider no extension brings (oxplow's own), several
  unbound candidates and no choice, or `oxplow` chosen → the node's own
  content, `replaced_by` unset;
- chosen → its page runs as the node's embed with the props it declares;
  a run that fails sets the node's `error` and the node's own content
  runs instead.

**Drawn** (`components/Replaced.tsx`, chosen by `PageTree` when the node
has `replaced_by`): the embed (kit vizzes, or `viz: custom` with its
sandbox and "custom" badge, sharing the view its target's contract lists)
under a **"replaced by <extension>"** badge (`replacement-<target>`,
`replacement-badge`), kept fresh as any page run is. The page's own chrome
(the Board's scope picker) stays oxplow's. A run with an `error` draws
the core component under a line saying whose it was and why ("beads's
board couldn't load: …. Showing oxplow's.", `replacement-fallback`),
outside the replacement's frame: no badge. Nothing is decided in the
desktop, so the wrong component never flashes.

Showing one records `usage { kind: "replacement", key: <ext>/<target> }`
(the evidence a kind needs to be promoted), once per replacement drawn
without an error. Agents read work items from `v_work_item` either way.
The rules are tested in Rust (`replacements::chosen`'s tests, the runner's
`a_replaceable_node_runs_the_replacement_that_stands_in_for_it`); which
replacement a page shows is a fixture's (`loaders: { <node>/<loader> }`,
a `ui` step on the embedded component); the badge, the fallback line and
choosing one on Settings are the e2e specs'. The pages with every
enhancement off are `tests-e2e/specs/pages/plain.spec.ts`'s
([work-items.md](./work-items.md), "Conformance").

**The first real one: the history graph.** oxplow-bundled's
`history-graph` component (`components/history-graph/`, classic scripts on
the client library and the kit) draws the stream's commits as `git log
--graph` does — a lane per line of descent, a dot per commit — from its
hidden `history-graph` page (a recursive walk of `v_commit` from
`:head_sha`, with local branch and tag labels from `v_branch` / `v_tag`).
Its text, author and branch filters only dim or hide rows, so typing never
re-runs the page. On Git History they are the page's `q`, `author` and
`branch` (the contract's `view`): they start from the page's, each change
writes them (`setViewParams`; an echo of its own write is ignored, so
typing is not undone by a late one) and Back sets them again
(`onViewParams`). With no view (a tile) they are its own. The host re-runs
the page when a commit lands or a ref moves (its reads). A row opens the
commit's URI (`oxplow:oxplow/commit?sha=<sha>`); a Cmd/Ctrl-click opens it
in a new tab (`navigate(uri, { newTab: true })`). It declares no commands,
so it is not a program to approve. Core keeps no lane code: `CommitList`
(the fallback, and the Git dashboard's recent commits with their stats) is
a plain list. `viz/historyGraph.test.ts` runs the bundle's own script
against its page — lanes, merge lanes, child-before-parent order,
filters, reveal, Cmd-click.

**The example that proves views and forms: `examples/extensions/task-views`.**
An extension with no provider of its own, replacing `work_item.board`
(the `swimlanes` component: a column per canonical state, drag a card to
another column to `oxplow.work_item.transition` it, drag within a column to
`reorder` it only when the active provider's `features` include `ordering`,
click a title to rename it through an inline `<form>` with `ox-input` and a
field refusal shown in `ox-error`, a click on a card `navigate`s, a
Cmd/Ctrl-click opens a new tab, and a `DENIED` refusal is one line above the
board), `work_item.list` (`compact-list`, a `viz: table` with Done and an
`Edit…` form action) and `work_item.detail` (`edit-item`, a `viz: form` over
`oxplow.work_item.update` with `uri` fixed). It reads `v_work_item` and
changes items only through the interface commands, so it works with any
list. `viz/swimlanes.test.ts` runs the bundle's script against its page;
`every_example_scaffolds_checks_and_tests_clean` checks it clean and runs its fixtures.

A form (`FormViz`) re-reads its start values when it resets, so after a
successful Save it shows what was saved, not the title it first opened
with. Activation takes a cloned snapshot of the config (`read_config`)
rather than holding its read guard across the transaction: a nested read
behind a waiting config reload would otherwise deadlock the daemon when
the active work list switched
(`tests-e2e/specs/components/task-views.spec.ts` is its regression test).

## Reviewing by impact

A review looks at an extension's **impact**: what changing it would
change. It does not only look at what the extension declares
(`extension_impact.rs`). `ImpactReport` holds these lists, each entry
with a `Change` (`added`, `removed`, `changed`, `unchanged`):

- `pages` — each page's rendered text
  before and after (`PageImpact`);
- `datasets` — each view, the parts that differ (`changed`: `query`,
  `columns`, `description`, `tests`, `version`, `deprecated`), its
  columns before and after, the first contract difference
  (`contract_change`; a SQL-only change keeps the contract) and its
  `downstream` readers (`datasets_diff`). When the contract held, the
  review names the parts ("its description and tests changed");
- `collectors` — each collector's `Grants` (entry, runtime, hosts,
  credentials, env) before and after and the views it fills
  (`collectors_diff`);
- `providers` — grants, each declared command by name (`CommandChange`)
  and the capability's features before and after, plus where spec and
  declarations first differ (`first_difference`, so a change that no
  grant, command or feature shows still reads as something)
  (`providers_diff`, over each spec and its checked-in declarations);
- `effects` — each effect contribution's trigger and reactions
  (`EffectImpact`, below);
- `contributions` — every other contribution kind, added, removed or
  changed, as one generic row each from the descriptors
  (`ContributionImpact`, `contributions_diff`; `differs` names which of
  `version`, `needs`, `runs_on` and `global` changed).

`extension_impact::json_difference` is the one "where do two JSON
values first differ" walk (the provider host's `first_difference` uses
it too). A provider's changes are worded once, in Rust
(`extension_impact::provider_phrases`): the review's `summary`
prefixes each phrase with `Provider <id>:`, and `approval_lines` — sent
as `ProviderImpact.lines` — capitalizes them for the review's provider
rows.

The report is built from the two loaded versions. **It never runs a
program, an exec collector or an external provider** — a version that is
not yet deployed has no permissions approved. What it does run is
read-only or sandboxed: page and dataset queries on each version's
overlay, and derived collectors' and effects' scripts (below).

`extension_impact::impact(layer, before, after)` builds it. Each
`Version` carries its pages **already dry-run once**
(`pages::checks::dry_run_all` → `PageChecks`): every `sql` loader runs
once with its params at their defaults and no viewer context; `command`
and `page` loaders do not run, because a check acts as no one. For the
candidate these are the very runs `check_extension` made, so a review
runs each page once. Each page is rendered from its dry run
(`pages::checks::text`: each viz node under its title, through
`viz_text::render`) against **its own version's datasets** (each side's
overlay). So a page over a dataset that the candidate changes renders
against the changed SQL. A page that fails the same way on both sides
is `unchanged`. A dataset's `downstream` is its direct readers from
`v_dataset_lineage` (`downstream_of`) other than this extension's own
views, by exact name. A provider's declarations are read from each
version's files. `extension_impact::summary` is the one wording that
`extension check --impact` and an effort's review share: collectors'
and providers' grants first ("now reaches x (was y)", a provider command
added (destructive)), then what the collectors make of their inputs,
then datasets with their readers and rows, then pages ("Page x: changed",
"Page x: its query fails: …"), then the other contributions. A changed
page carries its text before and after in the report (`before`,
`after`); there is no line diff yet.

**An extension at any revision.** `extension_at(trees, ws, rev,
name)` loads project extension `name` as revision `rev` of the
workspace holds it — a commit (`git:HEAD`), a snapshot, the working
tree — through `Trees::corpus` into an in-memory `Tree` (the
`ExtensionFiles` the loader reads). A custom component's bundle is
looked up three ways (`BundleLook`). On disk it is `Found` or `Absent`
(an error). In a revision's tree it is checked from the tree's own paths
when they are there, and `Unknown` — taken as declared, not an error —
when they are not, since a built bundle usually is not committed. At
`git:HEAD` of a clean worktree it equals the disk load. It is `None`
when that revision has no `extension.yaml`. This lets a review compare
two revisions of an extension, neither of which needs to be on disk.

**Each side on its own overlay.** A check is `prepare` →
`Prepared { pages, overlay }`: the extension's datasets (with the other
enabled extensions') compile to temp views, published nowhere, and its
pages, advisories and commands read through that overlay. A review
prepares **both** sides — the candidate and the version before it — so a
page over a dataset whose SQL changed renders against each version's own
SQL, whatever the database has published (a disabled extension, a dataset
that failed, another worktree's copy).

**A dataset's rows.** Each changed, added or removed dataset's
`DatasetImpact.rows` is a `RowDiff { before, after, keyed?, note? }`: each
side's rows read through that side's own overlay (up to 10 000 — the read
gateway's own cap, `MAX_ROW_LIMIT`; `ROW_DIFF_LIMIT`). When both versions
declare the same non-empty `key`, it is a merge-join by key —
`KeyedDiff { key, added, removed, changed, samples }`, up to 20 sample
rows in key order, each with its key and its row before and after (key
order is SQLite's: numbers by value, then text). Otherwise — past the
limit, or when a side's key has a NULL part or repeats (one row would
stand for several; the key test runs only at publish) — the review gives
counts with a note that says why (or that a side's query failed). An
unchanged dataset has none. These rows are the review's only reads of
dataset data; nothing is written.

**A collector's outputs.** Each derived collector (Starlark,
jaq) whose script or declaration changed is dry-run on the same inputs
in both versions: each version's intent-example fixtures that name it
(`fixtures/<example>.yaml`, `when: { collector, rows }`), the latest
five events its `on:` trigger matches, else its `input:` query once. The
runner is `collector_runner::dry_run_collector`: each version's own
script text, its `input:` read through that version's own overlay,
storing nothing, as a pure run. A script that names an `ai_*` builtin
is refused before it runs (a review never spends or sends). The run has
the command scripts' `COMMAND_SCRIPT_BUDGET` (5 s, not a live
collector's 120 s: someone waits on a review).
`CollectorImpact.outputs` holds each input's `Ran { counts, rows (20 per
entity), error? }` before and after. An exec or read collector is never
run and says so in `not_run`. A collector whose entry's text changed
though its declaration did not is `changed` with `script_changed`, and
says "its script changed" even when there is nothing to run it on.

**An effect's reactions.** `ImpactReport.effects` lists each
effect by id (`EffectImpact`) with its trigger before and after
(`EffectTrigger { on, filter, needs }`; a script-only change is
`changed` too). When it changed, it also has `outputs`: each input — both
versions' fixtures that name it (`when: { effect, event, answers? }`)
and the latest five events of its `on` types — composed by each version
that reacts to it (`effects::dry_run` through that side's overlay,
nothing runs): `Composes { commands, skip?, error? }`. The lines read
"Effect x: added — on t where k = v", "Effect x: on … → on …", "Effect x
on fixture basic: runs [a] → skips (why)".

**A bounded review.** Script dry runs — collectors' and
effects' — share one deadline, `REVIEW_DEADLINE` (60 s, from the start
of `impact`; `impact_within` takes another). Past it, no further input
runs, and each collector or effect that did not get through its inputs
is in `ImpactReport.out_of_time` (`collector <id>`, `effect <id>`), with
the line "Out of time: collector a and effect b — the review stops
running scripts after 60s". Only the later side is *checked*
(`prepare`: commands and their examples, components, advisories, page
shapes); the earlier side is only *read* (`read_side`: its datasets'
overlay and its pages dry-run on it) — what it was, not whether it
was right.

**`oxplow extension check <name> --impact [--against <rev>]`.**
The same review on the CLI: `oxplow_sdk::check(…, against)` — the check
and its impact with one throwaway oxplow's command registry. It loads
the extension at git `HEAD` (or `--against`) — `extension_tree_at`
through a `Trees` over the VCS alone — as `before` and the working tree
as `after`, and runs `extensions::impact_between`. Each side is on its
own overlay over the project's database read read-only (else the
check's empty one) — a review, the one place a check reads the project's
data — so the check **writes nothing** — the database file is
byte-identical after. Text output adds `impact against <rev>:` and the
report's lines; `--json` adds `impact` (the `ImpactReport`) and
`against` to the check's JSON.

**On an effort's review.** The diff page (`oxplow-foundation/diff-view`)
has an "Extension Changes" table, empty unless the range touched
`.oxplow/extensions/**`. Its `extensions` loader reads the live dataset
`v_extension_change` (pinned to `base` and `head`, each a `v_change`
revision spelling; columns `name`, `change`, `errors`, `lines`,
`page_changes`, `stream_id`). The server's
`extensions::extension_changes_between` names each extension whose files
differ between the two revisions (`Trees::diff`), loads both versions as
those revisions hold them (`extension_tree_at`) and reviews them with
`impact_between`. Each row has the name, the change, the problems, the
report's lines and the pages that differ (`page_changes`). A removed extension has no report. The pages'
before/after text is not shown (the changed file's diff has it). A failed
review is a row's problems, never a spinner that does not end.

## Commands in core menus (a command's `ui.about`)

A command an extension declares meets a person through its own `ui`
([commands.md](./commands.md) "Offering a command to a person"). One
**about a resource** (`ui: { label, group, about: oxplow/work_item, input: {
uri: "{{uri}}" } }`) is offered on a URI of that resource. It shows in
its page's nav-bar **Commands** menu (`RefCommandsMenu`, beside Ask) and
in the right-click menu of a row for the row's URI (a page's table or
list rows, by the first URI the row links to — `rowRef` — and Board
cards), grouped by `ui.group`. The input's strings may be exactly
`{{uri}}` / `{{uri.id}}` (or `{{stream}}` / `{{thread}}`, which expand to
bare ids; `{{uri.id}}` is the URI's key). Without an `input`, a command
gets `{ uri: "{{uri}}" }`. The desktop builds the offers from the
person's command listing (`refOffers`, `components/refCommands.ts`) and
runs each as search does (App publishes its offer host,
`setRefOfferHost`). To put another command on a URI's menu — oxplow's
transition to done, say — an extension declares its own command over the
same operation (`scope:` / `provider:` + `op:`, with its `ui.input`) or
a script that composes it. The old `ui.commands` block is gone.

**The desktop reads the extensions from one store**
(`extensionsStore.ts`, `useExtensions()`): one `listExtensions` (the
live set, the same in every stream) and one event subscription
however many readers are mounted (slots, decorators, rail panels,
extension pages, the launcher). It reloads on `extensionsChanged`. The
listing lives while something reads it, so a later mount loads afresh.
Only Settings → Extensions lists them itself — it reloads after its own
deploys and toggles.

## Custom components

`custom_components:` (**stable**; `extensions/custom_components.rs`) are
web bundles that a page shows in a sandboxed frame. A page uses one
through a `component` node (`view: { component: { id, props } }`, alone
as the whole page or inside a layout) or a `viz: custom` node
(`custom: { component, props }`). **The sandbox bounds what it
reaches.** The frame has an opaque origin (the iframe's
`sandbox="allow-scripts allow-forms"` *and* the daemon's `sandbox
allow-scripts allow-forms` CSP directive), no daemon token and no way to
send data. It has no fetch/XHR/WebSocket (`connect-src 'none'`), no form
submission (`form-action 'none'`: a component's `<form>` is its own, its
script handles `submit` and runs a command with `invoke`) and no
storage. The one way out is navigating itself. The page's `frame-src`
(`http://127.0.0.1:*`, `http://localhost:*` — each loopback name the
daemon serves a bundle to; not `[::1]`, which no CSP host source can
name) bounds that to this machine. That is Tauri's CSP in the app, and a
one-directive meta CSP in `index.html` for a plain browser, where
whatever serves `dist/` sends no header (`viz/frameBound.test.ts` keeps
them one value; only `frame-src`, since a fuller policy would bound what
the page connects to, and a daemon over a tunnel is not on loopback).
The host also ends the component on its second `load`. So a component
reaches only what its page feeds it, its own read-only SQL (`sql:
read`) and the commands it may invoke (`commands`).

**A component that acts runs under its extension's permissions.**
`invoke` runs a command with the viewer's rights, so the commands a
component declares are the only ones it can run. A command outside its
extension's namespace needs `commands:<pattern>` in the manifest
("Permissions"; the loader refuses the extension otherwise). A person
approved those permissions when the extension was deployed. An `invoke`
of a command that the loaded bundle does not declare is `Invalid`. An
`invoke` from a version that is not the current one is `Denied` (the
frame must reload), and the host shows a refusal's
reason under the frame (`custom-component-refused`, `componentRefusal`)
— the component's own code may not. A component that declares no
commands only shows and reads.

**What runs is what was deployed.** Before it shows a frame the host
loads the component's bundle (`load_component`, from the deployed
catalog, the same whatever the stream) into `Services::component_bundles`
(`component_bundles.rs`): the bundle's folder in its deployment and its
file names, keyed by its **version** — a hash over those files, its
commands and its `sql` access (`bundle_version`). No bytes are held. The
daemon serves the frame (`/components/v/<version>/…`) by reading the
named file from that folder, which never changes. The frame's `invoke`
names the version it loaded, and runs only when that version is the
current one and declares the command. So an edit to the source changes
nothing that runs until it is deployed. The folder is read as served —
the extension's path joined with the manifest's `bundle` — so a spelling
that differs from the disk's only in case still covers every file. Every
version loaded in the process stays servable while its deployment is in
the store (a folder and a list of names each, so nothing is dropped).
Once the deployment is pruned, its files 404 and the frame reloads.

**A hot update reaches a mounted frame.** Activation stages every
enabled extension's bundles into the generation it publishes
(`PreparedComponentBundles`), each stamped with that generation. A frame
whose snapshot belongs to a generation that is no longer active has its
`querySql` and `invoke` refused. The host loads the bundle again on every
`extensionsChanged` (sent after an activation publishes,
`CustomComponentViz`): the same version keeps the frame it has, a new
one mounts a new frame, and the old version's `invoke` is refused.

**Bundled and project components share one runtime.** A bundled
extension runs from its shipped deployment on disk (`Sources and
deployments`), so its component's bundle is staged, hashed, served at
`/components/v/<version>/` under the same CSP and — when it declares
`commands` — held to its extension's permissions exactly like a project's.
What still differs by origin, none of it about components: a bundled
extension with no deployment of its own runs what ships
(`shipped_permissions`), the `oxplow` command namespace is reserved
for bundled extensions, and a bundled extension's name wins a clash with
a project one's.

```yaml
custom_components:
  - id: burndown                       # [a-z0-9-]+
    title: Burndown
    bundle: components/burndown        # default components/<id>; holds index.html
    commands: [oxplow.work_item.transition]
    sql: read                          # optional: its own read-only SQL (querySql); needs sql.read in permissions
```
```yaml
# pages/burn.yaml
title: Burndown
loaders:
  burndown:
    sql: SELECT day, remaining FROM v_x_burndown WHERE stream_id = :stream_id
params: [{ name: stream_id }]
view:
  viz: custom
  from: burndown
  custom: { component: burndown, props: { color: accent } }
```

**A component reads what its page feeds it.** A component has no
`assets`: that key is refused as an unknown field. The page's loaders are
its data (`init.data`, `update`). For its own queries it declares `sql:
read` (`querySql`). Load checks, each at its line: the id, a duplicate,
`bundle` a relative folder without `..` that holds `index.html`, no
symlink anywhere in it (`stat_bundle`; a bundle has no size limit, see
Deploy) and each command a command name. A page that names a component
its extension does not declare is refused (`component_problem`: "`x`
isn't one of this extension's `custom_components`"; `<extension>/<id>` for
another extension's component is that extension's to have). A `viz:
custom` node needs `custom.component` and a `from` loader (its rows are
what the component shows and what an agent reads). An agent reads a
`viz: custom` node as its table, prefixed ``(custom component
`<ext>/<id>`; its table rendering)``, and a `component` node as
``(custom component `<id>`: it shows the page's loaders)``.
`check_extension` checks that the declared commands are registered, and
warns when a custom viz also fills a kit role block (`chart`, `tree`,
`timeline`, `steps`, `hunks`, `code`, `diff`, `editor`, `embed`,
`terminal`) — the kit may already render it. That is the honest extent of
a "this reimplements the kit" lint.

**The bridged calls** (`viz_actions.rs`; the frame reaches them only
through the host). Each names the component by `id`, `<extension>/<component
id>` — the component itself, whatever shows it — and the bundle `version`
its frame was loaded at (`load_component { id }`). A version that is not
the current one is `Denied`. `invoke_component_command { id, command,
input, confirmed, version }` runs `command` when it is one of the
component's `commands` at that version, as **`Actor::Component {
component: id, on_behalf_of: Human }`** — the component acting for the
person, so every policy applies as if they ran it. It is audited as kind
`component` with the component's contribution ref as source
(`command.executed@5`). The input is literal (no placeholders). A command
that asks comes back `NEEDS_CONFIRMATION` and the **host** asks, never
the frame. A component acts for a person, so it never proposes.
`run_component_sql` and `component_form` are the same kind of call. All
are UI RPCs (`ui` in surface parity).

**The host** (`viz/CustomComponentViz.tsx` for a `viz: custom` node,
`pages/PageComponent.tsx` for a `component` node, both over
`viz/ComponentFrame.tsx` and `viz/componentBridge.ts`):
`<iframe sandbox="allow-scripts allow-forms" referrerPolicy="no-referrer">`
at `componentBundleUrl(base, version)` (the daemon's
`/components/v/<version>/`, once `load_component` answered; a bundle that
cannot be loaded shows the table and why; with no daemon base —
`remoteBaseUrl()` null — the page shows its table), under a **custom**
badge. On the frame's first `load` the host posts `init { protocol, run,
data, params, props, page, view, tokens }` (`initMessage`, the one
builder) with one end of a `MessageChannel` (to `"*"`: the frame's origin
is opaque; the port goes to that frame alone) and listens on the other
end only. `data` holds each loader's `{ columns, rows, truncated, error? }`
by name (`pageData`), `params` what they ran with, `props` the node's
`props` (or a viz's `custom.props`). `run` is a `viz: custom` node's own
rows as a `VizRun`, and null for a `component` node. `page { streamId,
threadId }` is set when the component is a page's `component` node, else
null. `view { viewParams, historyState }` is its host's view, or null. A
change follows as `{ type: update, run, data, params, changed }`, with
`changed` the loaders whose rows differ (posted only when something
did).

Frame messages (`parseFrameMessage`): `ready`; `{ id, method: invoke,
command, input }`; `{ id, method: navigate, uri }` (an oxplow page only:
`componentMayOpen` refuses an external-url URI with `INVALID`, since an
outside tab would carry whatever the frame put in its URL out); `{ id,
method: querySql, sql, params }` (positional SqlCells;
`run_component_sql`: the component must declare `sql: read` and its
extension hold `sql.read`; it runs as the component's `Mode::Pure`
run under its extension's grant, `sql.read` decided and recorded,
read-only over the published datasets, 1,000 rows by default); `{ id,
method: openForm, command, input }` (the host shows a declared command's
form — `component_form` — in a slideover and runs the submission through
`invoke_component_command`; closing it answers `CANCELLED`); `{ id,
method: openForm, form: { title, schema, values? } }` (a form of the
component's own: the host draws `schema`, a JSON Schema where a
`writeOnly` string is masked, with the same `SchemaForm` and answers the
values submitted, running nothing); `{ id, method: signIn, instance, name
}` and `{ id, method: cancelSignIn, instance, name }` (the host runs the
whole OAuth redirect flow for a provider instance's credential,
`viz/hostSignIn.ts`: the shell listens on the credential's registered
port, the system browser opens the service's page, each redirect goes to
the core; it answers `{ state: signed_in }`, `{ state: failed, error }` or
`{ state: cancelled }`). Each step is a command the component must declare
(`oxplow.provider.begin_sign_in`, `complete_sign_in`, `cancel_sign_in`),
run as the component. Without the desktop app it answers `INVALID`. A
newer sign-in of the credential, a `cancelSignIn` or closing the frame
ends the one under way.

A component that is a whole page also sends `{ id, method: setTitle, title
}` and `{ id, method: setActions, actions: [{ id, label, primary?,
prominent?, submenu?, icon?, disabled? }] }` (the page's title and its
nav-bar actions, data core draws: the primary and prominent ones as
buttons, the rest in the bar's ☰; a press posts `{ type: action, id }` to
the frame), and `{ id, method: showView, spec }` (an inline page's YAML,
read-only, `parse_inline_page`, that the page opens as a run of its own
— `useInlineView`, held open while it shows, so it stays live — and draws
under the frame with `PageTree`, with Save as Page when it is one viz
over one `sql` loader). `showView` answers its first loader's rows, for
the frame's own pickers, or `INVALID` — the open's refusal, or a failed
loader with `field: /loaders/<loader>/sql`. A null spec clears it. While a
view shows, the frame sits at its content's height: the library posts
`{ type: size, height }` (a message, not a request, from a
`ResizeObserver` on its root; a component leaves `html` and `body` at
`height: auto`). `setTitle`, `setActions` and `showView` are a
whole-page component's. `openForm`, `signIn` and `cancelSignIn` are any
host's that draws a slideover — a whole page and a component inside a
page's layout, a settings section included (`viz/frameForms.tsx`,
`useFrameForms`). A host with none answers `INVALID`. And — each kind of
page state by the name of its scope — `{ id, method: setViewParams,
params }` (string values, null drops one: the `#` of its tab entry,
replaced; a component has no selection role, so it never pushes, and
going somewhere is `navigate`) and `{ id, method: setHistoryState, patch
}` (this step of its tab). The host posts `{ type: viewParams, params }`
and `{ type: historyState, state }` when the entry's change (Back,
Forward, an echo; `createBridgeHost` sends only what the frame has not
seen). `init.view` carries both.

### Testing a component

A page fixture's step `{ ui: [..], node? }` runs the page's custom
component — its `node:`, else its first `component` / `viz: custom` node,
else the first in a page it embeds; `node: <embed>/<node>` names one inside
an embed, a slot's mount or a replacement (`component_at`; on each re-run
the harness is fed that embedded run again, `embedded_at`) — **in the app's own webview, against the test's oxplow, with no app
around it** (`component_harness.rs`, `PageAt::ui`):

- **The server** (loopback, per step) serves the harness's page (`/harness/`,
  `crates/oxplow-sdk/assets/harness-host.js`), the bundle exactly as the
  daemon serves it (`oxplow_app::component_http`: the same files, CSP and
  library) with the agent (`assets/harness-agent.js`, served beside the
  library so the CSP allows it) put first in its `index.html`.
- **The host** (`apps/desktop/src/viz/harnessHost.ts`, built by `bun run
  build:harness`; `harnessBuild.test.ts` fails when the committed build is
  stale or pulls in React) mounts the frame with the app's own
  `createBridgeHost` / `initMessage` / `pageData`. What the core answers —
  `invoke` (`invoke_component_command`, as the person), `querySql`
  (`run_component_sql`), `openForm` / `askForm`, `showView` — runs on the
  test's oxplow; what only the shell answers (`setTitle`, `setActions`,
  `setViewParams`, `setHistoryState`, `navigate`, a confirmation) is
  recorded, in the order made (one queue of posts, which the next op waits
  for). `showView` runs one at a time in the order asked and is recorded
  once the core has answered: `shown` is the spec when the core drew it,
  `null` when it refused it or the component cleared it. The server drives
  it one op at a time (`/harness/next`): a DOM step for the agent, a nav-bar
  press, a view change, new rows.
- **The window** is the shell's `oxplow component-harness <url>`: one
  window, no Dock icon, never focused, no project, daemon, menu or IPC; it
  quits at `/harness/done`. Tests run while the person works, so it is
  never seen: on macOS it is shown (WebKit draws no frames for a hidden
  window, or for one off every display, and macOS moves a titled window
  placed off-screen onto a display left of or above the main one) but
  borderless, fully transparent and click-through (`unseen` in the
  shell's `main.rs`); elsewhere it goes to -20000, -20000. Found as `OXPLOW_SHELL`, else an
  `oxplow` beside the running program or up to two folders above it. With
  none, the `ui` steps aren't run, nor the steps after them (they read
  what the `ui` steps did), and the report warns — never a pass.
  Likewise when the window draws no frames: WebKit draws none while the
  display is locked or asleep, so a component that lays itself out never
  draws. The page asks first (`drawsFrames`: a frame within 2 s) and
  records `drawing`; `false` leaves the steps, and those after them, not
  run with a warning, never failed (`Harness::drawing`). CI builds the app and runs the tests
  under `xvfb-run`.
- **Steps**: `click: <sel>`, `fill: { <sel>: text }`, `key: { on, key }`
  (`Mod+Enter`; a plain `Enter` in a form's `<input>` that nothing stopped
  submits the form, as a browser's implicit submission does),
  `drag: { from, to }` (synthetic DOM events, not a
  person's: focus and keyboard details can differ); `press: <action id>`
  (the nav bar), `view_params: {..}` (Back/Forward; `null` drops one). The view
  params the frame set (`setViewParams`) and the ones a step sent are kept,
  merged, and when they change the page runs again with them over the
  step's `params`, and the frame hears the rows, as the app's tab does with
  its `#`; `given: [..]` (the
  page runs again and the frame hears its rows, as after any command it
  ran); `confirm: false` (the next confirmation's answer; default yes),
  `form: { submit: {..} }` (the next form's values; without, it's closed);
  `expect: { visible, count: { <sel>: n }, text: { <sel>: text | {
  contains } }, title, actions: [ids], view_params, invoked: [{ command,
  input, ok, result | refused }], navigated, confirmed, forms, shown }`,
  compared by the one rule and waited for (5 s). A finding names
  `steps[i]: ui[j]` and carries the frame's HTML (4 kB) and the errors
  the agent caught.
- **What it can't see**: how the app draws the frame, the nav bar or the
  slideover (the generic `tests-e2e/specs/pages/component.spec.ts`), real
  input events, or how it looks.

**A page's rail.** A page file may have a `rail:` node tree beside its
`view:`. The page host draws it in the page's rail, and a row it picks
writes the page's view by the rail's rule (`pageViewChange`: a selection
pushes a history step, a filter replaces), which a whole-page component
hears through `onViewParams`. A page's rail stacked above its body (a
narrow window) is capped and scrolls, so the body keeps the height.

**Whose view a component has** (protocol 9, `viz/hostedView.ts`
`useHostedView`): its host's, where the host shares one, and only what
the host's contract names.

- A page that **is** the component (`component` as the whole view) has
  the whole entry: all its params (`patchViewParams`) and its history
  state.
- A **replacement** has the host page's view params that its target's
  contract lists (`UiReplacement.view`, `v_replaceable.view`: Git
  History's `q`, `author`, `branch` for the history graph). These are the
  ones the core part it replaces reads, so the two leave the same
  address. Another key answers `INVALID` naming the contract.
- A **`viz: custom` node** on a page has the page's non-resource params
  (`pageViewKeys`: its filters and picks), set by the rail's rule
  (`pageViewChange`).

A hosted component's history state is its own object in the entry,
`component:<target>` or `component:page`, so it never overwrites its
page's (a draft, a pick). Every other host — a dashboard tile, a slot,
the rail HUD — gives none: `init.view` is null, `setViewParams` and
`setHistoryState` answer `INVALID`, and the component keeps its filters
itself. Any component, page or not, has **page memory**: `{ id, method:
pageMemoryGet, key }` and `{ id, method: pageMemorySet, key, value }`
(null forgets), scope `component:<ext>/<id>` in `ui_preference` — for
the person, in every tab of it. A value over 64 KB or a component over
1 MB is refused `INVALID` (`componentMemoryProblem`, before the write).
A component keeps no view state of its own: `check` warns
(`bundle_warnings`) at a bundle script that uses `localStorage` or
`sessionStorage`.

Replies are `{ id, ok, result | error: { code, message, field? } }`
(`field`: the input an `INVALID` refusal names, so a component's form
marks it). A re-run sends `update` — only when its params or result
differ from what the frame last got (`createBridgeHost` takes the run
`init` carried). A second `ready` is ignored. The frame is keyed by its
bundle URL (`ComponentFrame`): a stream switch mounts a new frame whose
first `load` is its own, and a failure is remembered per URL. An invoke
that asks shows the host's `CommandConfirm`; confirming re-runs it
confirmed, declining answers `CANCELLED`. No `ready` within 3 s, or a
second `load` (the frame navigated itself), tears it down and shows the
table. `ready` records `usage { kind: "custom_component", key: <ext>/<id>
}` — the evidence a kind needs to be promoted. `tokens` are the root's CSS
custom properties (`tokensFromStyle`). There is no gesture check on
invoke: the frame's clicks do not reliably activate the host across
webviews, and the real bounds are the declared list, the person's policy
and the host's confirmation.

**The client library.** A bundle does not speak the protocol by
hand. The daemon serves `oxplow-component.js` (and its `.d.ts`) at
`/component-lib/<file>` — embedded in the daemon
(`crates/oxplow-app/assets/component/`, `component_http::library`), served
like a bundle (loopback `Host` only, ungated, outside CORS, `nosniff`),
and named in every bundle's CSP `script-src` beside the bundle's own
folder (`bundle_csp(source, lib)`). There is no `'self'` in that CSP:
even in a sandboxed frame it matches the daemon's whole origin — the
response URL's, checked in Chromium and WebKit — so it would let a
component load another bundle's code. The check lint reports a page path
that leaves the bundle (`..` past its folder, an absolute path other than
`/component-lib/`, a `data:` script or sheet). The library is a **classic
script that defines the global `oxplow`**, not a module: module scripts
are fetched with CORS, which an opaque origin never passes and nothing
served to a frame allows — the same reason a bundle's own scripts are
classic. `oxplow.connect()` waits for `init`, answers `ready`, and
resolves with `{ data, params, run (the latest of each), page, props,
tokens, protocol, onUpdate(fn({ run, data, params, changed })) →
unsubscribe, invoke(command, input), navigate(uri, { newTab, replace }),
querySql, openForm (a command's, or `{ title, schema, values }`),
signIn, cancelSignIn, setTitle, setActions, showView, onAction,
viewParams, setViewParams, onViewParams, historyState, setHistoryState,
onHistoryState, pageMemory, applyTheme(doc?) }` (the `.d.ts` beside it
is the reference; `viewParams` / `historyState` are null where the host
gives no view). A failed request rejects with the host's `{ code,
message }`. `init` carries `protocol` (`BRIDGE_PROTOCOL` in
`componentBridge.ts`, `PROTOCOL` in the library — one number, bumped when
a message's shape changes; now 12), and `connect` rejects a host that
speaks another. One app is one library version. `componentClient.test.ts`
drives the served file against the real `createBridgeHost` and the real
`init` (`initMessage`, which `CustomComponentViz` posts), so the two
cannot drift. `oxplow extension new component <name>` scaffolds an
extension — the component, a page that shows it and a bundle that uses
the library — that checks and tests clean (`just_works.rs`). The
reference is in `docs/guide/pages.md` "Custom components".

**`custom_components` is stable**, on the evidence rule: a shared
extension's component acts with it. The github example's **PR Lifetimes**
(`components/pr-lifetimes/`, shown by a page) draws each pull request as
a bar from opened to merged — a range the kit's charts do not draw. Its
`state` filter is a param of the page, drawn in the page's rail. A bar opens `oxplow:github/pr?id=<n>`
(`navigate`), and Sync runs `oxplow.collector.sync` (`invoke`) — a
command outside the extension's namespace, so the manifest declares
`commands:<pattern>` for it. The host's own re-run (`onUpdate`) is
asked again with a selected filter rather than drawn over it. A pull
request whose dates cannot be read is left out and counted in a note,
never blanking the rest. A rejected `connect()` is said in the status. A
component author can use each of these as a dataset. The loader limits
components to no kind of extension. The browser suite's
`specs/components/pr-lifetimes.spec.ts` runs it in Chromium **and
WebKit** (the engines of browser mode and the macOS window; the suite's
one WebKit project): the frame loads, the filters and navigation work,
`invoke` is held to the declared commands, and a self-navigation off this
machine sends no request in either engine — the blocked page's load then
ends the component and its table shows.

**The kit's stylesheet.** `/component-lib/oxplow-kit.css`
(`assets/oxplow-kit.css`, served beside the library as `text/css`, and
named in every bundle's CSP `style-src`) is a few `ox-` classes in
oxplow's look — text, card, badge, buttons, forms (`ox-form`, `ox-field`,
`ox-label`, `ox-input`/`ox-select`/`ox-textarea` with the accent focus
ring, `ox-error`), drag and drop (`ox-drag`, `ox-dragging`,
`ox-drop-target`, the Work board's dashed accent drop look), table,
states, chart series. They read only the theme's tokens (`kitSheet.test.ts`:
every `var(--x)` is one the app's root defines, and no literal color). A
bundle links it and calls `component.applyTheme()`, which sets each of
`init`'s `tokens` on the frame's root through the CSSOM — no `<style>`
text crosses the bridge.

**Check lints**: what a bundle's CSP refuses without a word, or what ends
the component, is an error at check, at its file (`bundle_problems` /
`page_problems`, from `check_components`):

- an inline `<script>` that runs (no `type`, or a JavaScript one — a
  `type="application/json"` data block is fine) or an event handler on a
  standard element (a custom element's `on…` attribute is its own);
- a `type="module"` script;
- a load from outside the bundle (`src`, each `srcset` candidate, a
  `<link>`'s `href`) — a scheme other than `data:`, `//host`, an absolute
  path other than `/component-lib/` (matched before `?`/`#`), or `..`
  past the folder;
- a `<base>`, an `<iframe>`/`<frame>`, an `<object>`/`<embed>`, a
  refresh `<meta>` (a `<form>` is allowed: it submits nowhere);
- an `index.html` that never loads the client library.

Only each bundle's **`index.html`** is linted — the one page a frame
shows, since a nested frame is refused and navigating away ends the
component. It is read from the **version under check**: the candidate of
a review (`ReviewSide::read`), the folder on disk for a plain check. A
version without the built page (a revision's tree) has nothing to lint;
loading reports a bundle with no `index.html`. A bundled extension's
pages are linted from its embedded files, the same way. The scan is a
start-tag reader, comments skipped and raw-text elements (`script`,
`style`, `textarea`, `title`) read to their close, not a full HTML
parser.

## Decorators

`ui.decorators` (**stable**; `extensions/decorators.rs`) put labels from
one of the extension's **datasets** on URIs of a resource:

```yaml
ui:
  decorators:
    - { dataset: flags, resource: oxplow/work_item, placement: ref-chip, label: label, color: color }
```

The dataset (`v_<extension>_<dataset>`) must declare a `uri` column and the
`label` (and optional `color`) columns named. This is checked at load
against the dataset's declared columns, which are its contract. Column
names must be plain identifiers, since the desktop names them in its
query. The desktop (`components/decorators.ts`, `useDecorations`) runs
each decorator's queries over the URIs it shows (`SELECT uri, "<label>"
AS label[, "<color>" AS color] FROM <view> WHERE uri IN (…)`), re-run
when the dataset changes. **Bounded**: the URIs go 200 to a query
(`REFS_PER_QUERY`), each query with a row limit that has room for all of
them, so a large table loses no badge. One extension adds at most 3
decorations to one URI (`MAX_DECORATIONS_PER_REF`), each label at most 40
characters (`MAX_LABEL`, then `…`). `ref-chip`: a chip in the header of a
page whose URI the dataset lists, after the page's own chips (`Page`, by
the page's URI from its navigation context). `row-badge`: a `RefBadge`
(tone `label`) after a table or list cell that links to a listed URI. A
color is used only when it is a plain one (`#rgb…` or a CSS color name,
`safeColor`). Decorations are additive: a decorator whose query fails
shows nothing.

**Promoted to stable** on its first-party use: oxplow-bundled shows each
effort's latest verdict on the effort — a `ref-chip` on its page and a
`row-badge` where a row links to it. Its dataset `verdict` (`Accepted`,
`Accepted (forced)` or `Changes requested`; green, orange, red) reads
`verdicts`, which keeps every verdict event (`materialize: {
incremental: seq }`, appended as each lands; a rewrite of the log —
retention's payload expiry, a restart's refill — refills it whole). It
reads only the events' **envelopes**: the type says the verdict
(`oxplow_bundled.accepted` / `.changes_requested`) and the subject what
it was about — the effort first, then its work item, then each claim and
decision an acceptance took unchecked (any makes it forced). Retention
keeps envelopes and an extension's payloads go after 30 days at most, so
a verdict a decorator shows must not live in its payload. It shows; it
never acts.

## The run context

Every run of extension code — and every bus run — carries one
`RunContext` (`oxplow_domain::run`; the app's constructors are
`run_context.rs`): its run id (a UUIDv7), its parent run, the
contribution it belongs to (its URI, the extension's
`version`, the catalog generation it was built against), its actor,
anchors, subject and cause, its mode, its budget, its cancel flag and
its scope trace.

- **Built by a named constructor, never assembled.** `for_effect`,
  `for_policy`, `for_collector`, `for_command`, `for_dry_run` and
  `RunContext::core` fill what that run has and leave the rest `None`.
  Nested work is `child()`: a new run id with this one as parent, the
  same actor, cancel flag and generation, the budget no larger than
  what's left. A child can be another contribution's (a collector a
  person's `oxplow.collector.sync` ran) but never another actor's —
  `placed_in(stream)` only fills an agent's stream its transport left
  out.
- **Modes are explicit.** `Pure` (a composer, an effect's or policy's
  script: it may be run again, no I/O but scope reads), `Transactional`
  (a `Tx` handler), `External` (may call a program, a provider, a
  dataset). What a mode may use is the services table's (`modes`), enforced
  by `RunContext::decide_service` ("Services" below): a pure run asking a
  dataset is refused before its script runs.
- **A run is checked for authority, not for staleness.** `check_authority()`
  refuses a cancelled run or one a live guard fails (and cancels it). A
  generation going out of date is the activation's read lock (a run that pins
  a generation holds it; one that doesn't belongs to its extension's
  deployment, `run_context::live_owner`), so a run carries no "stale" outcome: it ends
  `cancelled`.
- **Who carries it:** the bus (`run_in`, `TxCtx::run`, `Invocation::run`
  — an `External` call's run is a child of the command's), scope calls
  (`scope_calls::Calls::new(run, needs, read)`, counted on its trace),
  a composer (`Composer` gets a pure child; what it calls becomes the
  run's scopes), effects (`for_effect`: the effect actor on the event's
  thread and stream, caused by the event), the effort policy
  (`for_policy`, its composed calls `run_in` it so `command.executed`
  is caused by the event), entity and fact collectors (a child of the
  run that ran them, named for the collector), provider invokes (on the
  wire as `InvokeParams.context`; `host/call`s are answered only inside
  a registered run) and knowledge, snapshot and harness verbs. A page
  acting is `Actor::Page { page_id, on_behalf_of }` (source: the page's
  URI; a node's id is in `actor_id`) and a component acting is
  `Actor::Component` (source: its contribution ref); both act for the
  viewer.
- **Composed commands keep child ownership.** Routing captures each child's
  registered contribution factory with its retained handler. Nested composers,
  transactional leaves and external steps receive that child's context, not
  the parent extension's namespace or declaration. Transactional leaves narrow
  their mode; all children inherit cancellation and live guards. Native scope
  operations capture their implicit backing scope in declared authority.
  Routing and dispatch check run authority. This is handler attribution, not
  a coordinated DB/catalog activation or a new per-child command audit.
- **Every contribution's run is recorded once** (`extension_run`,
  `v_extension_run`; [DATA_DATASET_X.md](./DATA_DATASET_X.md) "extension_run"):
  its identity, actor, anchors, cause, outcome, cost (`cpu_ms`,
  `peak_bytes`, from each script's `Ran`), the scopes it called and its
  model calls — never what it read or wrote. Nested work of the same
  contribution adds to its parent's usage (`child()` shares it); another
  contribution's child is its own row, its `parent_run_id` the run that
  started it. A person's `oxplow.effect.retry` / `backfill` links the
  reaction's run to theirs (`under`) without making it theirs: the
  effect still runs as itself.
  **Reading it:** the bundled `runs` page (`oxplow-bundled/runs`, titled
  "Extension runs", in the launcher under Activity) lists, per extension
  and for the last 7 days, its runs, failures, denials, mean `cpu_ms` and
  model calls, the ones needing attention first. Settings → Extensions
  shows each extension's worst current run (`v_extension_run`, `latest =
  1`). Opening an extension's row (the hidden `oxplow-bundled/extension-runs`
  page, param `extension`) lists its runs one by one for the same week:
  contribution, version, generation (blank for a run that holds none),
  actor, causing event, outcome, error, wall and CPU time, model calls — the
  small inspector, with no log or secret to read.
- **How agents use oxplow's interface.** The bundled `agent-interface` page
  (launcher, Activity) is a dashboard of three tables over the last 30 days:
  `interface-tool-calls` (oxplow's MCP tools by tool and harness, from
  `v_tool_call` joined to `v_agent_session`; a call whose session oxplow has
  no record of reads harness `unknown`), `interface-commands` (agents' runs by
  command from `v_command_audit`: runs, invalid, denied, failed and the share
  not ok; opening a command shows the hidden `interface-command-errors` page,
  its most common error texts counted together up to the "Expected one of"
  list) and `interface-hints` (`v_hint_stat` per hint, those that never fired
  first). The bundled `hint-never-fires` advisory tells the person, once per
  hint, about a hint evaluated 100 or more times that never fires.
- **Failure, retry and freshness are the same everywhere it matters.** A run
  that ends `failed` counts toward the contribution's failure policy (three in
  a row disable it; `v_contribution_health`); one that ends `cancelled` — a
  person's cancel, or its extension changing or being disabled mid-run — does
  not (an effect's script is checked once more when it returns, so a disable
  during it is a `cancelled` run and a `failed` attempt that doesn't count,
  `Reacted::Stopped`). What a *refused* run is: `denied` (a caller or grant the contract
  rejects) is never retried; a *failed* effect attempt is sent again by itself
  only when it was `Unavailable`, every step it composed goes to a provider
  keeping `idempotent_writes`, and it has retries left (otherwise a person
  retries it); a job replays only if it declares `replay: idempotent` (up to
  three attempts); a collector is not retried by itself, its schedule runs it
  again. A collector whose last run failed is not `fresh` in
  `v_contribution_health` — its rows are the last good run's — and a failed
  attempt keeps its own `extension_run` row, reachable from its data effects
  (`effect_run.run_id`, `collector_run.run_id`, `job_run.run_id`). A program's
  words in an `error` have the credentials it was handed replaced. Runs are
  kept 90 days (`EXTENSION_RUN_DAYS`). Pinned by `extension_run_health_tests.rs`
  and `event_retention::tests::extension_runs_past_their_window_are_swept`.
- **Cancel reaches the worker.** Every script a collector, command,
  effect, policy or fact collector runs is passed its run's cancel flag;
  an exec collector's program is killed when it's set. Upcasts, report
  parsers and the MCP adapter's mappings still run
  under their own fixed budgets without one.

## Services

A script reaches oxplow through **services**, each a scope it names in
its `needs` (`oxplow_domain::service::SERVICES`, one table every reader
uses):

| Service | Builtins | Needs permission | Modes |
|---|---|---|---|
| `sql.read` | `scope("sql.read", …)` | no | any |
| `tree.read` | `files`, `source_files`, `duplicate_blocks` | no | any |
| `ai.compute` | `ai_classify`, `ai_score`, `ai_summarize`, `ai_extract`, `ai_complete` | `ai` | external only |
| `ai.discover` | `ai_capabilities`, `ai_models`; `scope("ai.discover", { role })` | no | any, when the host supplies AI metadata |

- **Who may declare what** (checked at load): a Starlark collector's
  `needs:` any service (`check_declared`; another runtime has no
  builtins, so `needs` there is an error); a composing script — a
  command's `entry:`, an effect, an effort policy script — reads only
  (`scope_calls::check_script_need`: it runs pure, so `ai.compute` is
  refused, naming why). A **command's** script reads the semantic layer
  (metric functions included) and not the tree: it decides what runs,
  and reading a whole tree belongs to an effect or a collector (refused
  at load, naming them). A provider's
  `needs` stay `scope()` calls (`check_callable`: `host/call` answers
  `sql.read`).
- **`scope("ai.discover", { role })`** (callable from an effect, a command
  or any composing script that declares it) says whether a dataset serves
  `role` — a built-in, or the bare id of one of the run's extension's
  `ai_profiles`, qualified as `Role::parse_in` does — without spending:
  `{ role: "<qualified>", model: { provider, model }, problem: null }` when
  bound (an unbound profile reports its fallback's binding), or `{ role,
  dataset: null, problem: "<why>" }` when no dataset is bound or no active
  extension declares the profile; any other settings failure is an error.
  It reads settings only (`AiService::binding_for`): no key, no capability
  probe. An effect uses it to skip, with a readable reason, instead of
  failing for want of a model. `answers: { ai.discover: [...] }` stand in
  for fixtures.
- **What a kind implies:** a Starlark fact collector reads its tree
  (`CollectorSpec::service_needs` adds `tree.read`); an entity collector
  whose script names a model-calling builtin uses `ai.compute`. Discovery
  builtins require an explicit `ai.discover` declaration. A collector's
  descriptor lists declared ∪ implied.
- **What a script uses** is read from its syntax
  (`oxplow_script::uses::services_used`, which `calls_ai` is a view of),
  and **checked at load**: `services::check_uses` refuses a script that
  uses a service its needs don't name (a collector's needs include what
  its kind implies), or one its kind may not use
  (`scope_calls::check_script_need` / `check_command_need`) — a load
  error at the script, "uses `files()` without `tree.read` in its
  `needs` — fix: …". The router's refusals are the run-time backstop.
- **Discovery.** `services::of_extension` lists each script's services
  (declared / implied / undeclared, used or not, permission) —
  `CheckReport.services`, `oxplow extension check --services`; the live dataset `v_scope`
  lists every scope (id, access, host, summary) for an agent; `docs/reference/extensions.md`
  "Services" is generated from it (`the_reference_lists_every_service`,
  sharing `test_fixtures::generated_section` with the kinds table).
- **Where a person sees them.** Settings → Extensions shows a line per
  script under each project extension's row — "Collector triage — Read the
  captured tree · Ask a dataset (you approve it)" — worded from the service
  titles (`list_services`, one rpc over `SERVICES`; the surface-parity
  manifest has it as a UI read, with `v_scope` for the agent), a service its script uses without naming it
  in `needs` flagged "(not in its needs)". The row's lines come from the
  extension's check (`oxplow_validate_extension` → `CheckReport.services`).
  Bundled extensions show no
  line (they are not checked in the section). Row dataset:
  `serviceLines` in `extensionRowModel.ts`. `oxplow extension new
  collector` scaffolds `needs: []` under a comment naming the three
  services.
- **At run time, one router** (`crates/oxplow-app/src/services.rs`):
  every host that runs a Starlark script — entity collectors
  (`collector_runner::given`), fact collectors (`MetricsService::given`),
  commands (`compose_calls`), effects (`effect_triggers::run_script`,
  `effects::run_over`) and effort policy scripts — calls
  `services::decide` / `prepare` with the run, the script and its needs.
  For each service the script uses it refuses what the run's mode
  excludes (`RunContext::decide_service`: "a pure run can't use
  `ai.compute`"), then what `needs` doesn't name ("`files()` needs
  `tree.read` in this script's `needs`" — a run with no authority too),
  then what the authority denies — recorded on the run either way — and
  builds the rest from the caller's `Sources`: the tree (`Trees::From`
  a `TreeSource` — the fact engine's, which reads the run's anchored
  snapshot, else its stream's latest, else the primary stream's, and
  keeps the last one — or `Trees::Fixed`, a fact collector's sweep or a
  fixture) and the dataset (a `CollectorOracle` on the run, caller
  `collector:<owner>/<id>`). A `sql.read` call is decided as it's made
  (`Calls::serve`), not twice. A caller with nothing to give (a command;
  a dry run, which is pure) says the service isn't available there. The
  worker gets the result as its host set (`Given::onto`).
- **Permission** stays the host's: a script that names a model-calling
  builtin needs the extension's `ai.compute` (the loader refuses it
  otherwise, "Permissions"), for an entity collector and a fact collector
  alike.
  Metadata-only discovery does not spend a key or require that permission;
  it still checks the declaration and live run authority. It never accesses
  credentials or probes a model. See [ai-providers.md](./ai-providers.md).
  Contained entity and fact collectors also pin their loaded script text to
  the current entry; text loaded for an older deployment cannot answer a
  subsequent service call or final result.

## Grants

A **need** is what a contribution declares; a **grant** is what its
extension may reach (its `permissions:`, approved when it was deployed —
"Permissions"); a **decision** is what a run was allowed
(`oxplow_domain::grant`, `grants.rs`).

- **The request.** `GrantRequest { scopes, program, hosts, credentials,
  env, commands }`, derived from the declaration: an exec collector's
  entry, `network`, `credentials` and `env`; a collector's services
  (`services::collector_needs`: declared ∪ implied, `ai.compute` among
  them for a model-calling script); a script's (effect, command, effort
  policy, job, provider `host/call`) scopes from its `needs`.
- **The grant.** `Grant { namespace, permissions }` (`grants::grant_of`):
  the extension's command namespace and its declared permissions. There is
  no approvals store and nothing hashed per program: the deployment is
  immutable, so what runs is what was deployed, and the deployment's
  permissions are what a person approved.
- **The authority** a run carries (`RunContext::authorized`,
  `run::Authority { declared, grant, enforcement }`,
  `grants::authority(ext, declared)`). Every extension run has one: a
  collector, an effect, a job, a script command, a scripted effort policy,
  a provider's `host/call`s. Core's own work has none and is allowed what
  it does (core's built-in effort policy, core commands).
- **Decisions.** `run.decide(Need)` → `Allowed`, `Denied(why)` or
  `Unenforced(why)`, recorded on the run (`v_extension_run.decisions`).
  `grant::decide` refuses what the contribution didn't declare ("not
  declared"), then what its extension's permissions don't allow — the
  refusal names the permission: ``needs `ai.compute` in the
  extension's manifest`` (`Grant::permits`: `sql.read` → `sql`;
  `tree.read` and paths → `files`; `ai.compute`/`ai.discover` → `ai`; a
  program, its files and env → `process`; a host → `network`; a
  credential → `secrets`; a command outside the extension's namespace →
  `commands`). Then `Enforcement { hosts }` says what this machine can't
  hold it to: a program's files always, its hosts wherever the network
  isn't confined — `Unenforced`, said rather than passed off as held.
  Decided: each scope call (`Calls::serve`); a program collector's
  `program`, `files`, each `env` name, each `host` and each `credential` —
  one not permitted is never read from the keychain; a script's services
  (`scope` `tree.read` / `ai.compute`, by the router). A denied need stops
  what asked for it. A model-calling Starlark collector or fact collector
  is refused at its `ai.compute` decision when its extension lacks `ai`
  (`RunCollectorError::Denied`, recorded as the run's `error`).
- **What the loader already refused.** Most of this is decided before
  anything runs: an extension whose contents need a permission it doesn't
  declare doesn't load ("Permissions"). The runtime decisions are for what
  only a run reveals — a script's composed commands, a path or dataset it
  reads.
- **Bounded reads.** `sql.read:v_work_item` names an exact published
  dataset; use multiple needs for multiple datasets. SQLite preparation checks
  the query's datasets and transitive registered `ref` inputs, including
  materialized datasets, before evaluating rows. Bare `sql.read` preserves
  the explicit whole-layer grant. Bounded queries cannot currently use
  `metric_grid` / `metric_findings` because their host precomputation has
  no dataset authority contract; query granted published datasets instead.
  `tree.read:src/main.rs` names a captured file; `tree.read:src/` names a
  directory. Absolute paths, traversal, backslashes and globs are refused.
  The host filters its captured tree before giving it to a script; these
  needs do not grant raw filesystem access. Bare `tree.read` grants the
  whole captured tree. A collector's `input:` query is the host's, under
  its declared dataset bounds, and needs no `sql` permission.
- **External execution.** Every external program (an exec collector, a
  fact program, a report parser, an external provider, a harness or MCP
  stdio server) runs from its extension's deployment — working directory
  and `OXPLOW_EXTENSION_DIR` both — under the `process` permission, with
  the person's file, network and process authority, and a deploy that adds
  `process` says so plainly. Confining external processes is shelved (the
  owner's decision); there is no `execution:` key. What oxplow does do is
  cleanup, not confinement: on Unix each program runs in an owned process
  group, killed on completion, timeout, cancellation or when its extension
  stops being live; a process can escape its group. Pipe I/O has a
  deadline, so escaped children cannot leave host threads blocked.
  Credentials are handed to a process at spawn and are process-readable;
  contained script services never receive keychain values. Disk extension
  reads refuse special files and files larger than 16 MiB; Unix opens
  refuse a final symlink and avoid blocking on replacement FIFOs.
- **Live checks.** A run that outlives a generation holds a
  `run_context::LiveCheck` (`live_owner`: its extension still enabled, the
  same deployment, still declaring it), composed through nested runs and
  rechecked before sensitive calls and publication. A program's process
  group is killed when it fails (`external_process`); an in-process script
  runs under `run_context::watch_authority(run)`, which sets the run's
  cancel flag (the script worker polls it). Disabling or redeploying an
  extension therefore stops its running work. This cannot undo an external
  side effect already performed.
- **Collector previews.** A preview stores no rows, but receives the same
  authority as an ordinary run; it never runs a program.
- **Component authority.** Query messages carry the loaded bundle version.
  Asset and command allowlists come from that bundle; a component's
  commands outside its namespace need `commands:<pattern>` (the loader
  checks them).
- **Not yet:** OS confinement and model call-count/monetary budgets.
- **Deliberately still open:**
  - adversarial tests for bounded `sql.read` (CTEs, UNIONs, table-valued
    and pragma functions, the `metric_grid()` refusal) and bounded
    `tree.read:` (sibling prefixes, `..`);
  - a shebang's interpreter resolves from `PATH`;
  - credentials are handed to a process at spawn: stopping it can't take
    a credential back, and a keychain deletion mid-run isn't watched;
  - a run built without authority is unbounded (core's own runs, by
    design; not yet a tested invariant);
  - network confinement is `sandbox-exec` on macOS only.

## The event envelope

An extension sees an event as one shape, `EventView`
(`oxplow_domain::events::view`), whichever path hands it over:

| Field | What it is |
|---|---|
| `id` | the event's id (a UUIDv7) |
| `type` | its type name |
| `v` | the newest registered version of `type`, which `payload` is at |
| `seq` | its place in the log, the delivery order |
| `at` | when it happened (RFC 3339) |
| `source` | what emitted it: `human`, `agent:thr3`, `system:<c>`, `collector:<ext>/<id>`, `effect:<ext>/<id>`, `provider:<id>` |
| `subject` | the URIs it's about |
| `anchors` | `stream_id`, `thread_id`, `effort_id`, `turn_id`, `agent_session_id`, `snapshot_id` |
| `payload` | its payload, carried to `v` |
| `cause` | the id of the event that caused it |

- **Every key is always there.** What an event doesn't have is `null`,
  never made up: a stream-level event's `thread_id`, an event nothing
  caused's `cause`.
- **Always at the newest version.** `EventView::of` carries a logged
  event through its type's upcasts first (`events::at_latest`, the one
  routine the pump uses too), so a script is written against one shape
  and older rows still reach it. A type the running vocabulary doesn't
  know (its extension is gone) passes as logged.
- **Anchors in JSON are typed ids:** `"thr3"`, `"str1"`, `"eff9"`,
  `"ses2"`; `turn_id` and `snapshot_id` are integers. A SQL input binds
  them as integers (`:thread_id`), because they're join keys.
- **A fixture's event is built the same way** (`EventView::fixture`):
  `{ type, v?, payload?, subject?, anchors?, id?, seq?, at?, source?,
  cause? }`, no other key. Its `type` must be registered; its payload
  must be valid at the `v` it gives (the newest when it gives none) and
  is carried to the newest from there, so a fixture replays an older
  version exactly as the runtime would; its subjects must be canonical
  URIs and its anchors typed ids. What it doesn't give is `null`, except
  `source`, which is `fixture`. A fixture that couldn't happen is an
  error naming the field (``event: `anchors.turn_id`: …``).

Every path hands over the same envelope, built once where the event
reaches the extension:

- a derived (Starlark/jaq) collector's `input.event` and a fact
  collector's (`collector_runner::produce`, `MetricsService::fact_input`)
  — on its trigger, on a paced run that re-reads the event later, and in
  a review's dry run alike;
- an `on:` exec collector's stdin, `{ "event": … }` (`{}` when nothing
  triggered it);
- an effect's `{ event }` (`effect_triggers::run_script`), a review's
  effect dry run included;
- the effort policy's event, and a provider's `react { event }`, keyed
  `react:<id>`.

An event that can't be carried to its type's newest version (its
upcast fails) is a failed run, with the reason — on a review, that
input's error (`Composes.error` / `Ran.error`, starting "its event: …"),
which the review lists on the input's line even when both versions fail
the same way (`Effect <id> on fixture <name>: fails (its event: …)`), so
the Review page (`ImpactReportView`) never drops it.

## Event types

`event_types:` (P8.D3; **stable since P9.D6**, so a shared or bundled
extension may use it — below; `extension_event_types.rs`,
`vocabulary_reactor.rs`) declares event types
the log accepts, under the extension's own namespace — its name with `-`
read as `_` (`acme-pr` → `acme_pr.*`):

```yaml
event_types:
  retention: { payload_days: 7, content_days: 3 }   # optional; only shorter than 30 / 14
  types:
    - type: acme_pr.merged
      v: 1
      schema: event_types/merged.v1.json   # the payload's JSON Schema, in the folder
      summary: A pull request merged.
    - type: acme_pr.merged
      v: 2
      schema: event_types/merged.v2.json
      summary: A pull request merged, with its reviewers.
      upcast: event_types/merged.star      # required past v1: transform({from_v, payload}) → the v2 payload
```

**Promoted to stable (P9.D6)** on the evidence rule every kind is held to
— a first-party, shared extension depending on it: oxplow-bundled's
`oxplow_bundled.accepted@2 { effort, work_item, unverified, inferred,
deviated }` and `oxplow_bundled.changes_requested@2 { effort, work_item,
unverified, inferred, deviated, note? }` (`effort` is the bare id, `eff12`; v1 held its URI and an upcast reads it), which `accept.star` and `request_changes.star` return in
`events:` (subjects: the effort, its work item and, for an acceptance, the
claims and decisions it took unchecked; caused by the run's
`command.executed`). They replaced a single `verdict@1` type, whose
verdict lived in an expiring payload.
Before them, a verdict lived only in a comment's text; now the effort's
timeline carries who decided what, and another extension can react to it
("Reacting to another extension's types"). What makes the kind safe to
promise is already built: a published `type@v` is a contract
(`event_type_contract` refuses a changed schema — stronger than a
dataset's drift warning), a new shape is a new version with an upcast, and
a removed type's rows stay readable. It was promoted to a stable kind
on that evidence; every kind is stable now (`ui.decorators` and
`ref_kinds` (now `resources`) in P10, "Decorators" and "Resources"; `effects` and
`custom_components` in P11, below and "Custom components";
a `ui.mounts` replacement with oxplow-bundled's history graph; `providers` once
the provider host was), so a kind carries no stability.

**`effects` is stable** (P11), on the evidence rule: a bundled
extension acts with one. oxplow-bundled's **`verify-unchecked`** reacts to
`oxplow_bundled.accepted`: its script takes the effort and its item from the event's payload
(`effort`, `work_item`) and the claims and decisions accepted unchecked from
its envelope (`event["subject"]`), links the item it files `about` the
effort, and reads (`scope("sql.read")`) only what the event
can't say: which of those are still unverified or inferred now, and its
own earlier reactions. It is empty when
the effect already filed (or proposed) a follow-up for an earlier
acceptance of the same effort (its `v_effect_run` reaction to that event
is `ok` or `proposed`; an earlier acceptance from before it
started, or whose reaction skipped or failed, doesn't count). With
nothing unchecked it skips; otherwise it files **one** item on the
active tracker, like every new item — "Verify what the review of <effort> accepted
unchecked", a checklist naming each claim and decision. Each line is
text, not markdown (`md_text`: whitespace becomes one space, markdown's
punctuation is escaped and a long one is cut at 300 characters), and the list stops
at 50 items, saying how many more the review has. A bundled effect
starts like any (K1), when the generation that first runs it publishes.
A task an effect (or oxplow
itself) files has no `author` — it isn't the person's; its
`work_item.created` and the run's audit name the effect (its `native`
`author` is null). The loader limits effects to no kind of extension.

**Retention** (P8.D5) is the namespace's window for payloads and large
content (DATA_DATASET_X.md "event_log" retention): either omitted part is the
default, and a longer one is a load error. The reactor records it in
`extension_event_retention`, which keeps it after the extension is gone.

**Loading** checks each type the way the vocabulary registers it (a
scratch `register_declared`): a core or foreign namespace, a schema file
that's missing, isn't JSON or doesn't compile, v0, a version past 1
without an upcast, an upcast that's missing or doesn't define
`transform`, a duplicate — each `extension.yaml:<line>`.

**The vocabulary follows the catalog.** `VocabularyService` (on
`Services`, spawned at boot) rebuilds the whole `Vocabulary` — core
types and kinds plus every enabled extension's declarations — on the
catalog's change signal and swaps it in (`VocabularyHandle`); a writer
already in a transaction keeps its snapshot. Refused at that point,
listed among the extension's errors (`Services::listed_extensions`, the
one listing that adds dataset and vocabulary health):
- a schema that differs from the one `event_type_contract` recorded at
  that `type@v` — a published shape is a contract, so a new shape is a
  new version with an upcast;
- two extensions whose names make one namespace (`acme-pr`, `acme_pr`)
  and declare different types — neither registers. One *type* two
  extensions both declare is settled earlier, at load, by the
  descriptor registry: the first keeps it and the second's declaration is
  a load error naming both.

The same pass restates `event_type_contract` (read as `v_event_type`:
type, v, extension, summary, `registered`, `latest`, schema). A removed
extension's types stay listed with `registered = 0`: their rows can't be
appended any more but still read, and the pump delivers them at their
logged version (`DATA_DATASET_X.md` "event_log"). `registered` is 0 too for a
type whose extension is there but whose declaration was refused (a
schema changed at a recorded version, a namespace collision) — its
health says why (`v_event_type` v2's column doc says so).

**Appending them (P8.D4).** Nothing else appends an extension's types:
a command script's result (and an effect's, P8.D10) may carry
`events: [{ type, payload, subject? }]`. `own_events` turns each into an
envelope at its type's newest version, from
`extension:<extension>/<command>`, refusing (the run is `Invalid` and
writes nothing) any type the running vocabulary doesn't list as this
extension's own — a core type, another extension's, or one in its
namespace it doesn't declare. They ride the run's `Composition.events`
and are appended after the children's events, caused by the run's
`command.executed`, in the run's transaction; on the steps path (a call
outside the transaction) they're recorded with the run only when every
step landed.

**A collector may emit them too (P9.D2).** An entity collector's output
— a derived script's result, an exec collector's stdout, parsed alike —
may carry `events: [{ type, payload, subject? }]` beside `entities`.
`collector_runner::plan_events` holds them to the same rule
(`own_events`: the extension's own declared types, at their newest
version), from `collector:<owner>/<id>`, and the payload is validated
before anything is written. They are appended **in the run's
transaction**, after its rows, its `collector_run` and its
`collector.synced`, each **caused by that `collector.synced`** — with
the run or not at all. A run whose events aren't allowed fails whole
(nothing stored, the run recorded `error`). Refused besides: a
`project` collector's (no extension, no namespace), a type
the collector's own `trigger.on` names (its run would trigger itself),
and more than `MAX_RUN_EVENTS` (100) a run. A fact collector doesn't
emit. A preview (`CollectorPreview.events`) and a `extension test` example
(`expect: { events: [{ type, payload? }] }`) show what would be logged;
a review's dry run counts them per type ("2 pr; emits 2 acme_pr.merged").

**The boundary.** A collector **ingests**: it turns the outside (or
existing state) into rows, and its events say *what it saw* — "a pull
request merged". It has no rights to act, needs nothing beyond its
extension's permissions, and may run again and again over the same input. An effect
**reacts**: it composes commands with an agent's rights, runs under its extension's
permissions, and runs at most once per event. So "when a PR merges, close its
work item" is a collector that emits `acme_pr.merged` and an effect that
reacts to it — never an effect with no commands whose only job is to emit
from data, and never a collector that acts.

**The loop guard** (`event_lineage.rs`) is one for both kinds of
event-triggered code. Walking an event's causes, a *hop* is an effect's
run (`command.executed` from `effect:…`) or a collector's run for an
event (`collector.synced` with `trigger: on`). Nothing reacts to an event
its **own** run led to, or to one `MAX_CHAIN` (4) hops already led to. A
guarded effect reaction is `skipped` in `effect_run`; a guarded collector
run is `collector_run.status = skipped` with the reason in `error` (V154;
its last good counts stay, and it isn't a failure toward a disable).

**Reacting to another extension's types (P9.D1).** A collector's
`on: [...]` and an effect's `on:` may name core types, the
extension's own declared ones, and **another extension's**
(`acme_pr.merged` from an extension that isn't `acme-pr`). There is no
`depends:` key: the qualified type name is the dependency, as a dataset's
`ref('<ext>/<name>')` is. `extensions::subscribes` sorts each name:
- **known** — a core type or one the extension declares;
- **foreign** — a well-formed name in a namespace that is neither core's
  nor its own: accepted, kept on `Extension.subscriptions`
  (`ForeignSubscription { by, event_type, declared_at }`), and a
  **warning** at load ("…another extension's event type; it runs once an
  enabled extension registers that type") — which is all `oxplow extension
  check` says, since a check sees one extension;
- **unknown** — a core namespace's type that doesn't exist, its own
  namespace's that it doesn't declare, a name that isn't a type's: an
  error, as before.

The vocabulary pass then holds each foreign subscription against the
vocabulary it just built: one **no enabled extension registers** is an
error on the *subscriber* (`extension.yaml:<line>: effect \`note\` reacts
to …`), in the same list a refused declaration shows in. It clears when
the owner is installed and enabled, and comes back if the owner goes or
its declaration is refused. Until then the reaction simply never fires:
nothing can append a type that isn't registered. There is no `type@v` in
`on:` — every consumer, and every script through the event envelope
("The event envelope" above), gets an event at its type's newest
registered version (`events::at_latest`, the owner's upcasts), so a subscriber is
written against the owner's latest shape and a breaking change is the
owner's new version plus upcast. A subscriber still **appends** only its
own types (`own_events` is unchanged), and the effects' loop guard
already counts every extension's runs.

**Health is the extension's errors, not `contribution_health`** — a refused
declaration is a load problem, like a dataset's contract drift, not a
failing run that counts toward a disable.

## Resources

`resources:` declares what the extension's URIs name
(`oxplow:<extension>/<resource>?<params>`): its params, the dataset that
titles one, search, an icon and `[[prefix:…]]` sugar. oxplow-foundation
declares oxplow's own the same way. The grammar, the registry, the
vocabulary and the stored forms are in `.context/resources.md`
"Resources are declared"; this section is the manifest's side.
`ref_kinds:` is gone: it is an unknown field now.

```yaml
resources:
  - name: pr                  # oxplow:acme/pr?id=12 (resource `acme/pr`)
    label: Pull request
    params: { id: { required: true, pattern: '^\d+$' } }
    additional: false         # true: takes params beyond its own (page)
    resolve: prs              # one of its datasets, with `uri` and `title` columns
    title: title              # the column of `resolve` that titles one
    searchable: found         # optional: one of its datasets, with `uri`, `title` and `body`
    icon: git-pull-request    # one of ICONS
    wikilink: { prefix: pr, param: id }   # optional `[[pr:12]]` sugar
```

**Loading** (`extension_resources.rs`, after datasets and pages) refuses, at
`extension.yaml:<line>`: a resource declared twice, a param key that
breaks the grammar, a broken or non-portable `pattern` (the desktop runs
it in JS's backtracking engine, so it is held to a subset both engines
read alike and JS can't be made to hang on; parsed by `param_pattern`:
printable ASCII, at most 256 characters, anchored `^…$` (an escaped `\$`
isn't the anchor); characters, classes (not negated, no `[` inside, no
`&&` `--` `~~` set operations), `\d`, `\w` and escaped punctuation (not
`\<` `\>`, word boundaries in Rust), each with at most one quantifier
(`? * + {n} {n,} {n,m}`, at most 256; no lazy `?`, no `{,m}`); no groups,
alternation, `.` or other escapes. Two repeats of varying length must be
fenced by a character the first can't match — `^[A-Z]+-\d+$` loads,
`^\w+_\w+$` doesn't — so a failing match gives each back at most once,
and no length needs capping. What is kept spells every set out — `\d` is
`[0-9]`, never Rust's Unicode digits — escaping exactly what either
engine reads as syntax), a `resolve` or `searchable` that isn't one of
its datasets with the columns it needs, a `wikilink.param` that isn't one
of its params, a `wikilink.prefix` that isn't lowercase letters, digits
and `_`, and an icon that is neither in
`ICONS` (lucide names the desktop maps; a list that only shrinks,
`fixtures/core_icons.json`) nor an `assets/<name>.svg` of the extension's own
(it must exist, sit inside the extension, be an svg of at most 32 KiB and inert: no
script, event handler or `foreignObject`). Such a file is read at load and
published as a `data:image/svg+xml;base64,…` icon, which the desktop draws as an
`<img>`.

**Registering** is the vocabulary reactor's: oxplow-foundation's
resources plus each enabled extension's go into one `ResourceRegistry`,
which refuses a second resource of a name and a second wikilink prefix
(the later one's load error names both). `validate_ref` and `[[pr:12]]`
know a resource while its extension is installed and not after. The pass
restates the `resource` table, read as `v_resource` (resource, extension,
label, params, additional, resolve, title, searchable, icon, wikilink;
dataset v4 — V103 dropped `page`); unlike event types, a removed extension's resources leave, and
URIs to them dangle (they show as raw text).

**The desktop** (`apps/desktop/src/resources.ts`) reads `v_resource`'s
rows (core's too) once (`useResourcesLoader`, in `App`, re-read on
`DatasetsChanged`) into one process-wide list that the pure helpers consult
and `useResources` subscribes to:
- `resourceIcon` / the page-kind helpers fall back to a resource's icon
  (`ICONS`, a lucide component per allowed name — a Rust test
  keeps the two lists equal; or the extension's own svg, drawn as an image)
  and label;
- opening a URI of the resource (`openRef`, resources.md "The desktop")
  opens it as itself, drawn by the renderer the daemon offers
  (`pages/renderers.rs`) and the window picks (`pickRenderer`); a page renderer is run with
  `{ uri }`, a URI nothing renders shows on the fallback `resource` page;
- `preprocessWikilinks` turns `[[pr:12]]` into a link when the value
  matches the param's pattern (`extensionWikilinkRef`, as the backend's
  `canonical_wikilink` does), and the link's text becomes its title from
  the `resolve` dataset (`useExtensionRefTitle`) unless the author
  labelled it.

**Searchable resources** (P9.D3, `kind_search.rs`). `searchable: <dataset>`
names one of the extension's datasets with `uri`, `title` and `body`
(checked at load, like `resolve`); `v_resource.searchable` is its view.
Its rows are in the site-wide search index (`search_fts`) under the
resource, so the launcher finds a pull request by its title or text and
the hit opens the resource's page as any URI of it does (the row shows the
resource's icon and label). A feed may add a `version` column (a row's
`updated_at`): a recompute then reads every row's version and the title
and body of the changed rows only.

- It is **index-time ingestion, as an asset** — not a query-time UNION:
  `search_fts` owns the text it ranks (BM25 and `snippet()` need it in
  the index), and a UNION would run every extension's SQL per keystroke,
  unranked. One `Materializer` per searchable resource
  (`search:<resource>`), its inputs the tables behind the dataset's view
  (the registry's lineage; a materialized dataset's own table): a commit
  touching one re-reads the view after the assets' quiet window and
  restates the resource's entries in its own transaction — never the
  writer's.
- `Assets::sync_search_kinds` keeps them in step with `resource` and the
  dataset registry (the change loop runs it at start and when `resource`,
  `dataset` or `dataset_input` change). A resource that stops being
  searchable — its extension removed, the key dropped — leaves the index
  with its entries; so does one that went while oxplow wasn't running (an
  `asset_state` row `search:<resource>` with no resource behind it).
- Bounded: 20,000 rows a resource, 16 KiB of body a row. A row whose `uri`
  isn't a URI of the resource is skipped (a hit must be something that
  opens), with a warning in the log. oxplow-foundation's resources aren't
  bounded.
- Entries are project-global (`stream_id` NULL): a dataset is.

**Revisions.** A revision is a URI param (`rev=git:abc1234`), and only
files, directories and symbols take one; no extension's resource has one.
A resource with revisions (a pull request as it stood at a head commit)
is designed, not built: nothing first-party reads one yet, so building it
would be an unproven contract.

## Ordering

When one event reaches several effects or collectors, their `after:`
says which run first (`oxplow_domain::ordering`, compiled by
`extensions::descriptors::check_catalog`).

- **Targets are URIs.** `effect:<ext>/<id>`, `collector:<ext>/<id>` (any
  enabled extension's) or `consumer:<name>` — short forms of
  `oxplow:oxplow/effect?effect=<ext>/<id>` and so on — one of core's event consumers
  (`event_pump::CORE_CONSUMERS`, kept equal to what a booted app registers
  by `the_core_consumer_list_is_what_a_booted_app_registers`; the two
  that run extensions' contributions aren't targets). A bare name is this
  extension's own effect or collector, else a core consumer.
  `Descriptor.after` and `v_contribution.after` hold the resolved URIs.
- **Mistakes are load errors.** A target nothing declares, a cycle (each
  member's extension gets the error once), or effects and collectors that
  wait on each other both ways (the two consumers would each wait on the
  other) take the contribution out of its extension with a `file` error.
  A target in a disabled extension is a warning and the edge is dropped.
- **Levels.** Each ordered contribution gets `order_level` (0 first),
  stored in `contribution.order_level` (V68).
  `extensions::descriptors::placement` reads it with the resolved targets.
- **Running.** `effect.triggers` runs an event's effects sorted by
  level, then URI. `collector.triggers` runs level by level: a level's
  fact collectors over one corpus (`run_fact_collectors`' `select`), then
  its entity collectors. Both keep their one checkpoint (renaming a
  consumer restarts it from seq 0). Cross-consumer targets become pump
  gates (`descriptors::gates`): a core consumer by name, the other
  kind's consumer for a contribution of the other kind; a gate on a
  consumer the pump doesn't run (a test's partial pump) is left out.
- **Inspection.** `oxplow extension check` prints an `order` line per
  contribution that names an `after:` (`oxplow_sdk::order_lines`).
- **Not dependency, not causality.** `imports:` is a dependency (a
  missing contract holds the consumer back); an event's `cause` is
  causality. `after:` only orders one event's handling.

## Effects

`effects:` (stable since P11 — a shared or bundled extension's
too; `effects.rs`) are scripts that react
to a logged event by composing commands, run as
`Actor::Effect { effect: "<extension>/<id>" }` for the event's thread and
stream — an agent's rights and gates there (the open-thread gate, its own
stream's VCS; an event with no thread, like a contribution's health, is
an anchorless effect: no thread gate, since it writes into no worktree —
its extension's permissions still apply), never a confirmation (commands.md "Only a person
confirms"):

```yaml
effects:
  - id: announce-done          # [a-z0-9-]+, unique in the extension
    summary: Note a finished item on its thread.
    on: [work_item.state_changed]  # core types, its own, or another extension's ("Event types")
    where: { to: done }            # optional: payload fields equal to these
    needs: [sql.read]              # the scopes its script calls: only what a script can (`scope_calls::CALLABLE`)
    entry: effects/announce.star   # transform({event}) → {commands, events?} | {skip}
    after: [page_ref.work_item]    # optional: what it runs after on one event ("Ordering")
```

**One trigger grammar.** An effect's `on`/`where` are a collector's —
the same keys on the entry, one parser (`collectors::parse_trigger`) and
one matcher (`collectors::fires`, `Trigger::fires`). An effect reacts to
every matching event, so it takes event types only: no clock, no report
run, no pacing (`settle`, `at_most`, `idle`, `force` are a collector's).
An effort policy's `.star` and a provider's `react` verb are the same
script shape handed an event by the host; they are not manifest
triggers.

**Loading** (P8.D9) reads `on`/`where` through the collectors'
`parse_trigger` (one rule for both) against core types and the
extension's own `event_types`, and needs the script in the folder,
defining `transform`; what's wrong is `extension.yaml:<line>`.

**Permissions and start position** (P8.D9). An effect runs under its
extension's `permissions:` ("Permissions"); the commands its script
composes are checked when it runs, and the effect itself needs nothing
declared. What runs is what was deployed, so an edit takes effect with the
next deploy. An effect starts when the generation that first runs it
publishes: that publication sets `effect_state.start_after_seq` (V148) to
the log's head (`effect_state_store::start_if_new_tx`, in
`extension_activation.rs`, for each enabled extension's effect not yet
started). An effect never reacts to an event logged before it started —
not the backlog, not what happened while its extension was disabled or
undeployed; a later deploy keeps the position it has. A person's backfill
reaches what came before. `effects::gate(start_after, seq)` is the one
check: `NotStarted`, `BeforeStart` or `Runs`; only the live consumer
applies it. The consumer hashes nothing: an edit reloads the catalog, and
the effect that runs is the deployed one.

**A bundled extension's effect starts the same way.** It has no deployment
of its own and runs what ships, under `shipped_permissions`; its start
position is set when the generation that first runs it publishes, and
there is no bundled branch in the gate.

**Running** (P8.D10, `effect_triggers.rs`): one async pump consumer,
`effect.triggers` (registered at boot beside `collector.triggers`). It
runs an event's effects in their compiled order and waits, through the
pump's `after_for`, for the consumers they name ("Ordering"). Per
event, each enabled effect whose `on`/`where` match **reacts at most
once**, keyed by `effect_run (effect, event_id)` (V149, `v_effect_run`):
1. a row exists — a redelivery: nothing. A `started` row the live
   consumer claimed is a run that claimed a step outside oxplow and was
   cut off (`effect_triggers::cut_off`): recorded `failed`
   ("interrupted"). When its claim kept what it composed — every step a
   write to a provider keeping `idempotent_writes` (`claim_tx` writes
   `resend_json` then, P11) — it is **sent again by itself** like
   a failure that may pass, its delay counted from when it started (so
   one found more than an hour late is a person's, and so is one whose
   start can't be read); otherwise it is a person's, never sent
   again by itself. Recording the failure and scheduling its retry are one
   transaction (`effect_triggers::fail`), here and for a failure
   that may pass. One a person's retry or
   backfill claimed is theirs, under way, and the pump leaves it be
   A person's retry or backfill — or an automatic attempt — is
   no pump delivery, so nothing redelivers one cut off between its claim
   and its record: at start (`boot.rs`,
   `effect_triggers::recover_interrupted`) every attempt still `started`
   whose origin isn't `live` is recovered the same way, with its
   `effect.result` saying what started it — Delivery lists it, and a
   person may retry it — and counted toward the effect's health
   as the pump counts one it finds;
2. `effects::gate` isn't `Runs` (a live reaction only): nothing;
3. the loop guard (`lineage`, walking the event's `cause` chain): an
   event its own run caused (source `effect:<extension>/<id>`) never
   triggers it; one that `MAX_CHAIN` (4) effect runs already led to is
   `skipped`;
4. the script runs sandboxed over `{ event }`, the event envelope ("The
   event envelope" above), reading with the scopes its `needs`
   lists — `scope("sql.read", { sql, params })`, binding what it
   wants (the payload's fields, the event's `id` / `seq` to read its own
   `v_event` row, its subject or cause) — served through the SQL gateway,
   each call its own read (`effects::run_script` with
   `scope_calls::Calls`, for the run and `dry_run` alike; a dry run
   answers from a fixture's `answers`): `{ skip: "why" }` is `skipped`;
   `{ commands, events? }` runs. The script and its reads run **before**
   the run's transaction: the composed calls are fixed by then, so a row
   can be stale against what the run then sees (each command's own
   validation is what holds — the composer doesn't re-read). What it
   called is added to the run's trace (`ScopeTrace::add`), so the
   reaction's audit row records it.

A run is the registered `oxplow.command.sequence` — its spec and compiled input
schema, shared (`Command::with_handler`) — over what the script composed
(calls, and its own events through `own_events`), run by
`CommandBus::run_effect` as `Actor::Effect` with
`RunOrigin::Effect(key)`: its `command.executed@4` is caused by the
triggering event, and the reaction's `effect_run` row and
`effect.result@5 { effect, event, outcome, reason?, proposal?, attempt,
origin }` (caused by that run) land **in the run's transaction**; with a step outside it, a
`started` claim commits first (`claim`) and the record finishes it; a
command that asks becomes a proposal whose transaction records
`proposed`. A failure before any command ran (the script, the input,
denied, invalid) is recorded `failed` by the consumer. An effect that
fails never dead-letters the event. An approved proposal runs its calls
as the person; the script's own events go only with a run that doesn't
ask.

**Attempts and a person's retry** (P9.D4). A reaction is `(effect,
event)`; `effect_run` holds its **attempts** (`attempt` from 1, `origin`
`live | retry | backfill | auto`, V156/V160), and the reaction's state is
its latest attempt's (`v_effect_run.latest`). The live consumer makes one
attempt and never another; a failed one is sent again **by itself** only
when that is safe (below). Otherwise a failed reaction is attempted
again only by a person: `oxplow.effect.retry { effect, event }` (`commands/effect.rs` —
`Confirm::Always`, `External`; an `effects.run` operation
declared in `oxplow-foundation`, its services filled when the consumer
registers at boot — `ServicesSlot`) runs `effect_triggers::run_reaction(…, ReactionOrigin::Retry)`,
the same steps as a live reaction from the loop guard on, as the next
attempt:
- the reaction's latest attempt must be `failed` (`Invalid` otherwise: one
  that ran, was skipped or left a proposal isn't retried; one never made
  has nothing to retry);
- the effect must be enabled and in the running catalog — a retry runs
  today's script, composing afresh from the event, not what the failed
  attempt composed;
- when the event was logged doesn't matter (`start_after_seq` is the live
  consumer's rule; the person named the event);
- it counts toward the effect's health like any attempt, and is recorded
  `effect.result@5 { …, attempt, origin: retry }` (v3 upcasts keeping its
  attempt and origin; v2 as `attempt: 1, origin: live`).

It asks every time because of what a failure can hide: an attempt
interrupted with a step outside oxplow under way may have landed that
step, and a retry sends it again. The confirmation's text says so.
Settings → Data → Delivery lists the reactions whose latest attempt
failed (`useFailedReactions`), each with its reason and **Retry**
(`InlineConfirm`).

**Sent again by itself** (P10, `effect_triggers::auto_retry_due`). An
attempt that failed in a way that may pass (`CommandError::Unavailable`:
a step's provider erred, timed out, lost its reply, or asked to
wait — a composite keeps its failing step's kind) while **every** step it
composed was a write
to a provider keeping `idempotent_writes` (`safe_to_resend`, through
`work_item::provider_for`) is scheduled again — `effect_run.retry_at`, at
most two in a row (`RETRY_DELAYS`: 10 s, then 60 s after the failure
before — or later when the service asked for longer; one asking for more
than `MAX_ASKED_WAIT`, 15 minutes, is a person's). A refused input, a
refusal of credentials renewal can't fix, a method or configuration the
provider lacks — `Failed` — is never retried by itself. The failed attempt keeps what it composed (`effect_run.
resend_json`, V162) for as long as its retry is scheduled — finishing an
attempt drops it, scheduling a retry writes it, and claiming the next
attempt moves it there, so no composition outlives its retry — and the automatic attempt **sends exactly that**
instead of running the script again: composing afresh could read
changed rows, change a step's input and so its key, and make a write that
landed again. It is sent only while every step still goes to a provider
keeping the promise; otherwise it isn't sent (`Reacted::NotResent`) and the
failure counts, a person's. A composed `oxplow.work_item.create` files on the
tracker active when it's **sent**, like every create: pointed
elsewhere meanwhile, the retry goes there — or, when that tracker keeps
no idempotent writes (oxplow's own), isn't sent by itself and waits for a
person. Each step carries its idempotency key, the
same on every attempt ([commands.md](./commands.md) `effect_step_key`), so
a write that landed lands once. A person's **Retry** composes afresh from
what the effect reads now — the person decides it should. A loop every 5 s (`spawn_auto_retry`, at boot — its first tick only
after the provider registry's first generation, so the providers the
config names are up — one row's error logged and the rest
going on) runs what is
due as the next attempt, `origin: auto` (`effect.result@5`), on the event
at its newest version (`at_latest`, as every runner hands it over; one that
no longer upcasts drops the retry), when the effect is still
there, enabled and running as deployed now and the retry is at most an hour
overdue (`MAX_RETRY_LATENESS`: one due longer ago — oxplow was closed — is
a person's, its failure counted; a due time that can't be read
counts as late) — otherwise the
retry is dropped and the failure is a person's. An attempt awaiting its
retry isn't counted against the effect's health (`Reacted::Retrying`);
the attempt that exhausts the retries counts once. A step inside oxplow, or
a provider that doesn't declare the promise, waits for a person. An
attempt cut off by oxplow stopping is sent again by itself on the same
terms: its claim keeps what it composed when every step is safe to send
again, and the store keeps it after only for a failed attempt
(`finish_tx`). Delivery
says "sent again by itself shortly" on a reaction awaiting its retry; a
person may still retry it first.

**Backfill** (P9.D5). The live consumer never reacts to what was logged
before an effect started. A person has it react to that past with
`oxplow.effect.backfill { effect, from_seq? | since?, to_seq? }` (`Confirm::Always`,
`External`): the events in the range that match its
`on` and `where` — each read at its type's newest version, as the pump
delivers it — and that it **never reacted to** (`unreacted_tx`: no
`effect_run` row) and its own run didn't lead to (the loop guard's `own`,
`event_lineage::lineage_tx`: such an event is never its trigger, so it
isn't planned — an effect that changes what it reacts to would otherwise
meet its own changes on every backfill and never get past them),
oldest first, **up to the effect's `start_after_seq`** — what was logged
after it started is the live consumer's, so the two never attempt one
event at once; a range past it is capped — each through
`run_reaction(…, ReactionOrigin::Backfill)`: attempt 1 with `origin:
backfill`, the same dedupe, loop guard, permissions and health as a live
reaction. So a second backfill finds nothing, the live consumer never
reacts to a backfilled event again, and three failures in a row disable
the effect and stop the run (`stopped`) — three of the run's own in a row
stop it too, counting ones that will be sent again by themselves (whose
health waits for the retry), so a backfill doesn't press on into
an outage. A run makes at most
`BACKFILL_BATCH` (200) reactions; the result `{ planned, ran, skipped,
proposed, failed, remaining, stopped? }` says what is left for another
run. The effect must be enabled and in the running catalog.

`oxplow.effect.backfill_plan` (a read, anyone's) answers `{ planned, from_seq,
to_seq, batch }` for the same input: what a backfill would react to. Both read
the candidates by log position a page at a time (`SCAN_PAGE`) to the end
of the range, applying `where` to every one — never to a first window of
them, which a selective `where` over a busy type would leave empty for
good — counting all and keeping the batch's first. The bus's
confirmation shows a command's summary and input, not a count, so the
count is the plan's: an effect's row in Settings → Data
has **Backfill…**, which reads the plan, says how many events
the effect never reacted to and that it may call outside oxplow for each
(`backfillAsk`), and runs only on the second click — on the range it
showed (`to_seq` from the plan, so what was logged in between isn't in
it), its button saying what one run does: "Run on the first 200 of 500"
when the plan is more than a `batch` (`backfillRunLabel`).

**There is no consumer-level replay.** Core's consumers are re-derivable
(a projection is rebuilt, not replayed); replaying the log through every
consumer would re-fire collectors noisily and effects nobody asked for.
Reacting to the past exists only as `oxplow.effect.backfill`.

## Commands

An extension's `commands:` (a stable kind, P6b; `extension_commands.rs`)
are commands on the bus whose handler is a Starlark script that
**composes core commands**, reaching anything else only through the **scopes**
it declares in `needs` ([commands.md](./commands.md) "Scopes") — `scope("sql.read", { sql, params })` today.
A script's scope needs are the scopes a script can call
(`scope_calls::CALLABLE`, `check_callable`): one it can't is refused at
load, the one rule a command's `entry:`, an effect and a provider's
`host/call` meet. A capability need (`work_items.comments`) gates the
command instead.

`sql.read` returns complete row lists only: more than 1000 result rows
fails in `scope_calls::Calls` before composition can act on a partial
set. Exactly 1000 is allowed. A script that means to walk a large result
asks for a `page: { limit, offset }` and gets `{ rows, more,
next_offset? }` ([commands.md](./commands.md) "Scopes"). Aggregate in SQL when all matching data
must contribute, or explicitly filter/limit a query when a subset is
intended. Commands, effects, effort policies, provider host calls and
dry runs share this check. A complete query result does not establish
collection coverage or freshness; the row-only scope carries neither.

```yaml
commands:
  - name: review.finish                # <area>.<verb>, each [a-z][a-z0-9_]*; id <namespace>.review.finish
    summary: Mark the task done and leave a note.
    input_schema: { type: object, required: [uri], properties: { uri: { type: string } } }
    entry: handlers/finish_review.star # defines transform(x), x = { input }
    confirm: never                     # never (default) | always | destructive; children only add
    access: write                      # write (default) | record | read (composes nothing, returns a result)
    job: false                         # true: a durable job's model may call it (needs entry:, never over a scope's operation)
    needs: [sql.read, work_items.comments]   # scopes it calls; capabilities / features it needs active
    ui:                                # how a person meets it (`.context/commands.md` "Offering a command")
      label: Finish Review
      group: Review
      about: oxplow/work_item          # resource-bound; absent: search offers it
      input: { uri: "{{uri}}" }
    examples:
      - { name: happy, input: { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }, expect_commands: [oxplow.work_item.transition] }
      - { name: gone, input: { uri: "oxplow:oxplow/work_item?id=tsk9&provider=oxplow" }, answers: { sql.read: [[]] }, refused: no such }
    example: { uri: "oxplow:oxplow/work_item?id=tsk1&provider=oxplow" }   # the input an agent is shown; a script's defaults to its first example that runs
```

`example:` is the command's example input, for any handler (script,
scope operation, provider op): it registers as JSON Schema's `examples`
on the input schema the command ends up with, an operation's schema
included, so `list_commands { id }` shows it to an agent and a refused
input quotes it ([commands.md](./commands.md) "Exposure to agents"). A
script command without one uses its first example that runs (not one
expecting a refusal). Every command an agent is offered has one that
fits (`every_command_an_agent_is_offered_has_an_example_that_fits`).

A command may be **backed by an operation of one of the extension's
providers**: `provider: fake` + `op: estimate`, the operation one its
checked-in declarations list (not a capability verb — those run as
`oxplow.work_item.<verb>`). Its input schema (plus an optional
`instance`), access and undo are the declaration's, its `confirm` the
stronger of the manifest's and the declaration's; it runs on the
instance its input names ([providers.md](./providers.md) "The host").

A command may instead be **backed by an operation of a scope**
([commands.md](./commands.md) "Scopes" → "Operations"):
`scope: bookmarks.write` + `op: set` in place of `entry:` — no
`input_schema`, `access` or `examples` (the operation's and its
scope's access), the scope implied in `needs`; the bus refuses
to register one naming an operation that isn't there. oxplow's own
commands are declared this way in the required `oxplow-foundation`.

A bundled extension's commands are `Stable`, any other's
`Experimental`. `transform` returns `{ commands: [{ name, input }], result?, events? }`, or
`{ refuse: "<why>" }` to decline — the run is `Invalid` with that reason
and writes nothing (`composed` → `Composed::{Run, Refused}`). An `access:
read` command composes nothing and logs no events (refused at run time
otherwise): it reads and returns a `result`, is never confirmed and needs
no scope that changes things; the bus runs it unrecorded. An example may give `answers:` — per scope, its calls'
answers in order, standing in so it doesn't depend on the project's data
(a scope without any is served for real; one that runs out is an
error) — and may expect a refusal with `refused: <part of the reason>` instead of
`expect_commands` (not both). A command's manifest `name` is
`<area>.<verb>`; its **command id** is `<namespace>.<area>.<verb>`. The
**namespace** is the manifest's `namespace:`, else the extension's name with
`-` → `_` (`command_namespace`; `check_namespace`: one snake_case segment,
and `oxplow` only for an extension that ships with oxplow — the bundled one
declares it, so its commands are `oxplow.review.accept`,
`oxplow.review.request_changes`, `oxplow.work.clear_finished`). Its event
types are declared by full name and unaffected. Who holds a namespace is the
bus's to say (`CommandBus::namespace_owner`: `oxplow`, or the one source —
`extension:<name>` / `provider:<instance>` — holding it) — there is no
hand-kept list: checked against the running registry (`RunningCommands`,
which the bus implements), a namespace another source holds is an error, and
at run time the generation's namespace reservation refuses it all-or-nothing
under one lock (reported as the extension's problem). The registrations key
registrations by extension (several of oxplow's share `oxplow`) and removes a
disabled one's with `unregister_source`. Two enabled extensions declaring one
namespace (other than `oxplow`) are both refused at load
(`refuse_shared_namespaces`, after disabling applies). Each entry is
checked at load, its error at its line (`entry_line`: the line whose
`name:` is exactly that name — `a` never lands on `abc`; ids, pages,
and datasets find their lines the same way): the name, `access`, `confirm`,
the schema compiles, each `needs` entry is a scope or a
capability / feature, and the entry is a file in the
extension that parses and defines `transform` (`check_starlark`), and
it declares at most `MAX_EXAMPLES` (10) examples. The
script's text is kept on the `ExtensionCommand` (not serialized).
`check_extension` (Settings → Extensions, `oxplow extension check`) runs
`check_commands`: with a running oxplow's registry, each example is
dry-run (`dry_run`) — `compose_calls`, its `sql.read` calls answered by the
example's `answers` or through the check's `SqlGateway` (its overlay
included, so a read of a raw table is an error at the example), and what
it composes against the registry — every command exists, its input
fits, the names are `expect_commands` in order. Without a registry it
warns that the examples weren't checked. **`compose_calls`** is the one
compose step, for the dry run and the handler alike: the script in the
sandbox under `COMMAND_SCRIPT_BUDGET` (5 s of the script's own time, not
the collectors' 120 s runaway catch: the bus composes it once, on a read
snapshot it holds while the script runs; its services are the router's,
`services::decide` — `sql.read` only, see "Services"), its `scope`
calls served on the calling thread (`run_command_script` with
`ScriptRun::with_scope`, through `scope_calls::Calls`; a busy database
retries the run), then the
`{ commands, result? }` shape (`composed`; any other key is `Invalid`).

**Running** (`extension_command`): each is a composite
(`Handler::Compose`, atomicity `Dispatch`) `<namespace>.<name>`
(summary "… (extension `x`)", the declared confirm / access,
`Lifecycle::Experimental`). Its composer runs `compose_calls` on the bus's routing thread,
answering `sql.read` through the SQL gateway as an effect's script is
answered (`SqlGateway::run_in`: the `query_sql` authorizer, row cap and
timeout, the metric functions computed first), `state.read`,
`configuration.read` and `jobs.read` through the gateway's sources, and
counting each call in the run's `ScopeTrace`. It runs once per run, when
the bus routes it.
The bus runs what it composes ([commands.md](./commands.md) →
"Composition"): when every call stays in oxplow's records, as children
in one transaction (`steps::run_composed`; the script runs once, when the bus routes the run) — each child's policy and
confirmation apply (a person's run whose child asks is asked first; an agent's run asks nothing), one audit row with `{ result,
children }`, the children's events caused by the run, the reversed
children as its undo; when one leaves it (`work_item.*` on another
provider's item), as **steps** — all checked first, then run in
order, each landing as it runs, stopping at the first failure, one audit
row, no undo. A script can't do I/O of its own: `files()` sees
nothing, `ai_*` is refused without a host, and `scope` reaches only
what it `needs`. **Registration**
(`ExtensionCommands`, `Services.extension_commands`): the enabled
live extensions (one bus, like providers — a
command registers when its extension is deployed), one
namespace at a time, all-or-nothing; registered by the generation at boot and on the
extension catalog's change signal (`ExtensionCatalog::changes`, P7.B6):
a deploy, rollback, enable or disable publishes it, and the
`config.extensions` reactor fires it for the `extensions` key.
Editing a source fires nothing. The extension datasets, the provider registry and
the metric catalog follow the same signal; a namespace something else
already holds (a provider) is refused and kept as the extension's
`problem`. Replacement is prepared against a cloned command registry and
published with one write-lock swap; rejection keeps the previous command batch.
Missing/disabled sources are removed. Registration tracks namespace, declarations,
package version and catalog identity, so provenance changes rebuild the batch
without requiring a script change. Each handler captures version and generation
from the same `CatalogSnapshot` as its declarations; a retained handler never
looks up a newer candidate's identity when invoked. Other registries and SQL
datasets still refresh independently. A launcher `{ command }` entry may name one.

## Contributing metrics (current)

`extension.yaml` takes `measures:`, `metrics:` and `dimensions:` (the
catalog schema), checked with the same `oxplow_config::validate_*` functions
given `CatalogLayer::Extension(ns)` (`Project` and `Global` say where a block
is written elsewhere), and fact collectors (a `collectors:` entry with
`facts:`, parsed by `oxplow_config::collectors` with a `Declarer`), plus:

- Metrics must be `key:` definitions; `use:` belongs to the project's
  `.oxplow/project.yaml`.
- Entity metrics (`entity:` + `where` / `time` / `value`) work
  here too, usually over the extension's own source views. Their fragments
  are checked at seed time, so one over a view whose source hasn't synced
  yet stays out of the catalog until the next reseed.
- Dimensions, fact or entity (`entity` / `expr` / `join`), layer
  like metrics (`resolve_dimensions` takes an `extensions` layer) and slice
  an extension's entity metrics over the same view. They are stored as
  `scope = 'global'` plus the `extension` column (V89).
  - `promote` is refused unless the extension is the project's own or
    bundled: toggling an installed extension would rebuild the metric cube
    each time.
  - A disabled or removed extension's dimensions are deleted
    (`delete_extension_dimensions_not_in`, never a promoted one). Facts
    don't reference dimension rows, so nothing else is lost.
- Fact collectors run `starlark` / `jaq` only, with no `env` /
  `credentials` / `network`. `exec` is refused unless the extension is the project's own: an own
  extension's exec program needs `process.run` like any extension
  collector and runs from its deployment.
  `entry` must exist in the extension.
- Precedence is global (the person's) < each extension, and the project's
  `use:` layers thresholds
  (`oxplow_config::resolve_{metrics,measures,dimensions}` take an
  `extensions` layer; scope `global` or `extension:<name>`; there is no
  built-in or project scope). `MetricsService::fact_collectors()` is the
  enabled extensions' fact collectors. An enabled extension's
  metrics are **on** unless the project mentions the key (a `use:`
  override or disable marker, or its own definition); a `default: off`
  one is listed inactive until the project `use:`s it. A metric's
  `collector:` names the collector in its extension that computes it: a
  collector metrics name runs only while one of them is active, one no
  metric names whenever its extension is enabled. A fact collector with
  `whole_tree: true` reads the whole tree as of the snapshot on every run.
  `oxplow.*` measure, dimension, metric and collector keys are the
  `oxplow` namespace's (`oxplow_config::reserved_key`).
- `MetricsService::extension_catalog` loads the live extensions
  on each resolve; fact-collector scripts are read through
  `extensions::read_extension_file` (bundled or disk), and a collector's
  `report` path resolves against the project root (the checkout).
- Storage: the scope CHECK allows only a fixed set, and
  rebuilding `measure` / `metric_spec` would cascade-delete facts (V54),
  so the store writes `extension:<name>` as `scope = 'global'` plus the
  `extension` column (V84) and reads it back. `v_metric_spec` shows
  `scope = 'extension'` and the `extension` name.
- A generation prunes extension specs no longer declared (a disabled or
  removed extension) with `delete_extension_specs_not_in`; its measures
  stay, since deleting one would take its facts. A new generation follows when the live set changes (a deploy).

## Health, disable and repair (P7.C1–C3)

Every contribution that runs — an external provider instance, a collector, an effect
(P8.D11, kind `effect`, V150) — shares one failure policy
(`contribution_health.rs`, the `contribution_health` table read as
`v_contribution_health`):

- **Counting.** A failed run or call counts; a good one starts the count
  over. A refused input, a denied permission, or a cancel isn't a failure.
- **Disabling.** The third failure in a row disables it on this machine
  and logs `contribution.disabled@1` (the row and the event commit together). A
  provider instance stops. A disabled collector doesn't run: the
  scheduler, the `collector.triggers` consumer and the snapshot sweep skip
  it, and `oxplow.collector.sync` refuses it with its reason (`Invalid` at
  `/id`). A disabled effect stops reacting (`effect.triggers` skips it).
  For an effect only a `failed` reaction counts — an interrupted one
  included, a lost race to another delivery not; `skipped` and `proposed`
  never do, and a reaction whose commands ran starts the count over. Nothing cascades: a page over its view still runs.
- **Repairing.** Core detects and reports (the health row,
  `contribution.disabled@1`, Settings → Extensions) with or without a work
  list. Filing the repair item is the bundled effect
  `oxplow-bundled/repair-disabled` (`effects/repair_disabled.star`, on
  `contribution.disabled`, `needs: [sql.read]`): it composes
  `oxplow.work_item.create` on the active work list — whichever provider
  that is, never a task table — titled `Repair <extension> <contribution>:
  <reason>`. Its body is what failed (the reason), where it is declared
  (`v_contribution` file:line), its recent failures (a collector's failed
  syncs, an effect's failed reactions, else the last error) and what to do.
  It no longer carries the declaration YAML, `intent.purpose`, the
  `extension check` output or the `engine:` line — none is readable through
  `sql.read`; the item says to run `oxplow extension check`/`test`. A later
  disable while the item is open comments on it (`oxplow.work_item.comment`);
  once it is done or canceled the next disable files a new one. **The
  open item is found by its link**: it is created with `links: [{ target:
  <contribution URI>, link_type: about }]`, and the open one is the not-done
  `v_work_item` with a `v_work_item_link` `about` the contribution's URI.
  The title is for people and never matched.
  `v_contribution_health.repair_item` is that same open item (the table
  keeps no `repair_item`). With no running work list
  (`none`, or the chosen tracker down) the effect skips; it is a bundled
  effect like `verify-unchecked`, so **repair items appear once its
  extension is enabled**, and disabling the effect stops new ones. It runs as an anchorless effect (the event has no thread), so the
  thread gate doesn't apply. oxplow never sends the item to an agent.
- **The events** are `contribution.disabled@1 { contribution, reason }` and
  `contribution.enabled@1 { contribution }`, `contribution` the URI
  (`collector:triage/todos`, `effect:…`, `provider:…`).
- **Enabling.** `oxplow.contribution.enable { extension, kind,
  contribution }`. It names the kind — a provider and a
  collector may share an id; there must be a failed one of that kind (or
  a provider instance the registry knows).
- **In the app (C3).** Settings → Extensions shows one health line per
  contribution under its extension (`contributionHealth.ts`, live over
  `v_contribution_health`): `OK` with its average time, a missed schedule and
  undelivered events; `Failing (N in a row): <error>`; or `Disabled:
  <reason>` with **Enable Again** (`oxplow.contribution.enable` through
  `personCommands`) and, while its repair item is open, **Repair with
  the Agent** — one line in the agent's input, `Repair the extension
  described in [oxplow uri <repair item>] — read it first.`, never sent
  (the repair brief is the item's body; `no-agent-input-automation.test.ts`
  pins the button as its only caller). Settings → Data → **Undelivered events**
  (the `alerts-deliveries` page, mounted in `oxplow-foundation/settings-data.sections`) lists the pending dead letters with Retry and Discard; the
  rail's Alerts shows "N events couldn't be delivered" while any wait.
  **Enable All Disabled** (a page action, atop the Extensions list and Data → Collectors)
  runs `oxplow.contribution.enable_all { extension? }` as the person: every
  failure-disabled contribution, or one extension's, each enabled as the
  single command does (one `contribution.enabled@1` apiece; an `External`
  command like it, so, like it, not undoable).
- **For agents.** `questions/extensions.yaml` and the extension skill's
  "Health and repair" section: read `v_contribution_health`,
  `v_collector_run` and `v_event_dead_letter`, read the repair item
  first, fix, check, test, `oxplow.collector.sync` — and run
  `oxplow.contribution.enable` (or `enable_all`) once the fix holds.

## Implementations

An extension declares capability implementations (stable,
`implementations:`; `extensions/implementations.rs`):

```yaml
implementations:
  - { capability: work_items, id: oxplow, entry: "oxplow:tasks" }
```

- `capability` is a choosable one, or one many implementations serve
  (`oxplow_domain::capability`: `agent_harness`,
  `ai_provider` — every one declared is active); `id` is what
  `activeProviders` names, or a many-capability's key (an agent session's
  `harness`) — lowercase, never `none`.
- **`icon:`** (any capability) is a path to an SVG inside the extension
  (`icons/claude.svg`): the implementation's logo, shown wherever it is
  (an agent harness's, on a session's tab). The checker (`load_icon`)
  refuses a missing file, a path outside the extension or not ending
  `.svg`, a file over 16 KB, one that doesn't start `<svg`, and one
  carrying `<script`. The declaration keeps it as a `data:image/svg+xml;base64,…`
  URI (`ImplementationDecl.icon`, `Implementation.icon`); the UI draws it
  only as an image (the mask of an empty element, `AgentIcon`), never as
  markup, so nothing in it runs. The foundation extension ships a
  monochrome mark each for `claude`, `codex` and `opencode`
  (`extensions/oxplow-foundation/icons/`); an external `agent_harness`
  provider declares its own the same way.
- `config:` configures the built-in, checked at load against its schema
  (`BuiltIn.config_schema`, `capability::check_config`; a built-in with
  none refuses any config); no built-in takes one now.
- `entry` names a built-in in core's standard library
  (`capabilities::BUILT_INS`: `oxplow:tasks`, `oxplow:commit-or-switch`,
  `oxplow:snapshots`, `oxplow:snapshot-hashes`, `oxplow:wiki`), the way a collector names
  `oxplow:junit`. A
  built-in's features are core's table's — it's core's code — never the
  manifest's. A built-in whose items' URIs carry a provider id
  (`BuiltIn.provider`: `oxplow:tasks` is `oxplow`) is declared under that
  id and no other.
- **A process implements a capability through `providers:`**, not
  here: a capability whose spec carries a provider contract (the work
  list, the effort policy, an agent harness) may be an external
  provider's instance — the work list and the effort policy chosen by
  `activeProviders` like a built-in, a harness registered beside the
  built-ins under its instance id, as many as run
  ([providers.md](./providers.md) "What a provider may implement").
- **An `ai_provider` names its Rust kind** with `provider:` instead of
  `entry:` (`implementations::provider_decl`, `Source::ProviderKind`):
  one of `oxplow_ai_rig::KINDS` (`anthropic`, `openai`,
  `openai_compatible`, `openrouter`, `gemini`, `mistral`, `groq`,
  `deepseek`, `xai`, `ollama`, `llamacpp`, `typesafe`), an unknown one
  refused naming them. It takes `config: { baseUrl? }` (any other key is
  refused), no `needs`, and needs `ai.compute` ("Permissions"): it
  points a person's key at that URL. Its `decide_native` feature is the
  kind's. The kinds and why they're Rust over Rig are
  [ai-providers.md](./ai-providers.md) "Rust kinds over Rig".
- **An `effort_policy` may be a script** instead: an `entry` that isn't
  `oxplow:<name>` names a `.star` file of the extension
  (`implementations::script_decl`, `Source::Script`). Nothing else is a
  script.
  - An effort policy's is effects-shaped — `transform({event})` →
    `{ commands: [{ name, input }] } | { skip: "why" }` — takes no config,
    and names the scopes it calls in the entry's `needs:` (only what a
    script can, `scope_calls::CALLABLE`; checked at load, with the script
    defining `transform`). `needs` is refused anywhere else. It runs
    under its extension's permissions, as an effect's does; what runs is
    what was deployed.
- `oxplow-foundation` declares the agent harnesses (`oxplow:claude-code`,
  `oxplow:codex-cli`, `oxplow:opencode`) and the AI
  providers, one per Rust kind under the kind's name, each at its API's
  default URL; foundation is required, so
  `CapabilityRegistry::new` registers nothing for them. The harnesses are
  registered by declaration (`harnesses::prepare_built_ins`, published by the
  generation → `Services.harnesses`, an `oxplow_domain::agent::registry::HarnessRegistry`).
- A required capability's default is core's own: `CapabilityRegistry::new`
  registers it (snapshots' `oxplow`, `oxplow:snapshots`), so it's there
  whatever is disabled, and a manifest declaring that id is an error.
- `oxplow-foundation` declares the default implementations (the work list,
  the effort policy and knowledge's wiki, `oxplow:wiki`), so they run with
  `oxplow-bundled` off; turning the list off is choosing another provider
  (or none) in Settings → Capabilities, not disabling an extension
  (`capabilities::CapabilityRegistry::resolve`, `.context/work-tracking.md`).
  It also offers snapshots' `hashes`
  (`oxplow:snapshot-hashes`, "Track changes only": no `contents`); a
  chosen `hashes` with bundled disabled falls back to core's `oxplow`.

**Declared needs.** A page or an advisory may declare `needs:
[work_items, snapshots.contents]` — capabilities, or one of a
capability's features (`oxplow_domain::capability::check_need`; an
unknown one is a load error). A page run checks them against what's
active (`ViewContext.active`, a `capabilities::Active` snapshot that
`view_context` fills): unmet, it runs no loader and `PageRun.inactive`
is set (`PageInactive { needs, message }`: "Needs: Work list (choose one
in Settings → Capabilities)."). The page view shows that instead of its
empty state. An advisory whose
needs aren't met doesn't run. A context built without `Services` (param
checks) checks nothing. A capability's none declares every feature (it
takes everything, keeping nothing), so a feature need is met by none and
only a need of the capability itself (`work_items`) isn't. `oxplow-foundation`
declares no `work_items` need: with no list chosen its task pages, hint and
skill just find nothing.

**What an implementation owns.** A provider instance owns its command
namespace (`<id>.*`), offered and run only while it's active
(`capabilities::Active::refusal`, `.context/commands.md` step 0). The
built-ins own nothing of the agent's surface: the work-item tools read
whichever list is active.

## Skills

An extension gives the coding agent skills and slash commands (stable,
`skills:`; `extensions/skills.rs`):

```yaml
skills:
  - { name: work-items, file: skills/work-items/SKILL.md, needs: [work_items] }
  - { name: work-next, kind: command, file: commands/work-next.md }
implementations:
  - { capability: work_items, id: oxplow, entry: "oxplow:tasks", skills: [work-next] }
```

- `name` is lowercase letters, digits and `-`, not one of core's own
  (`oxplow_agent_text::core_text`), declared once; a skill's
  frontmatter `name:` is its `name` and it has a `description:`, a
  command's frontmatter has a `description:`. `kind` is `skill` (the
  default: `<name>/SKILL.md`) or `command` (`/oxplow:<name>`).
- **Offered** (`capabilities::agent_text`): from enabled
  extensions (one with skills declares `agent`), when its `needs:` are met and, if an implementation lists
  it in its `skills:`, while that implementation is the active one. A name
  another extension already took is left out (logged).
- **Installed.** Each spawn writes what's offered into the runtime
  (each harness's `launch`, `crates/oxplow-harnesses`); boot, an
  extension change and a `capability.switched` rewrite the runtimes
  already on disk (`refresh_agent_text` → each harness's `refresh_text`). A skill folder oxplow wrote
  carries a `.oxplow` marker, so one no longer offered is removed and a
  person's own stay; the Claude `commands/` folder is oxplow's whole.
- `oxplow-foundation` gives the `work-items` skill (any work list) and the
  `work-next` command (oxplow's tasks' ready queue).

## Advisories

An extension gives the coding agent guidance — **hints** — with
**advisories** in `extension.yaml` (current). Core owns only the
mechanism; what to say, and when, is SQL in the extension.

```yaml
advisories:
  - id: low-coverage
    on: post-tool-use        # or prompt, or turn-end
    once_per: effort         # effort (default) | thread | session | day | row (needs a `key` column) | turn
    audience: agent          # agent (default) | person
    heading: "# Optional heading line"
    query: |                 # :thread_id, :stream_id, :turn_id, :effort_id
      SELECT '...' AS message FROM v_effort_observation WHERE effort_id = :effort_id ...
```

- **Triggers.** `post-tool-use` runs on each `agent.tool.finished`
  (`advisories.post_tool`), `prompt` on each prompt the person sends, and
  `turn-end` on each `thread.checkpoint` (`advisories.turn_end`, after
  `effort.observe`): evaluated when the turn ends, heard when the agent
  next hears from oxplow. A turn's end is the one point every harness
  reaches, blocked or not.
- **Params.** Every query may use `:thread_id`, `:stream_id`, `:turn_id`
  (NULL at a prompt) and `:effort_id` (NULL when the thread has no effort
  — for an event, the effort it was anchored to; at a turn's end, the one
  holding the turn's end; at a prompt, the open one).
- **Repeat rules.** `once_per: effort` fires once per effort and doesn't
  run without one; `thread` once per thread; `session` once per agent
  session on the thread (the latest turn's harness session; doesn't run
  without one); `day` once per thread per UTC day; `row` once per `key`
  on the thread (a query wanting once per effort per row puts
  `:effort_id` in its key, as `threshold-crossed` does); `turn` whenever
  it has rows. Marks live in `once_mark`, per effort or thread.
- **Audience.** `agent` hints go to the coding agent; `person` hints are
  raised in Alerts ("Hints for you", `apps/desktop/src/hints.ts`) until
  the person dismisses one (`oxplow.hint.dismiss`).
- **Delivery.** Every hit is recorded as a nudge (`v_agent_nudge`, kind
  `<extension>/<id>`, with its `audience`), undelivered. The thread's
  next prompt (`AgentContext::prompt_context`) or tool call
  (`post_tool_context`) takes the agent's undelivered nudges — oxplow's
  own first, oldest first, up to `NUDGE_BUDGET` (2,000) characters, at
  least one — and stamps `delivered_at`; what the budget held waits for
  the next hook. A person's nudge is stamped when they dismiss it. The
  ledger says what was raised and what actually reached its audience.
- **Effectiveness.** Each evaluation is counted (`hint_stat`), and
  `v_hint_stat` puts the counts together per hint and thread: evaluated,
  fired, delivered, held, muted. An agent hint that still fires after
  reaching the agent `MUTE_AFTER` (3) times in its effort (or its thread,
  with none) is muted there — not evaluated again — and the person is
  told once (a `hint-muted` nudge with what it said). A new effort starts
  it fresh.
- `crates/oxplow-app/src/advisories.rs`: `AdvisoryRunner` runs the enabled
  extensions' advisories for one trigger and scope and applies
  `once_per`. Marks are recorded only after every query ran. A failing
  query is logged and skipped.
- **Permission.** An extension's advisories speak into the agent's
  context, so the manifest must declare `agent.write`
  ("Permissions"; the loader refuses it otherwise), approved when the
  extension is deployed. A bundled extension's ship under
  `shipped_permissions`.
- `for_thread(svc, thread, on, cause)` runs them for a thread, reading
  the live extensions (whatever the thread's stream) and
  binding the thread's stream into the scope.
- `oxplow_validate_extension` dry-runs each advisory with every param NULL and
  checks it returns `message` (and `key` for `once_per: row`).
- oxplow-bundled ships hints on outcomes: `large-uncommitted` (turn-end,
  once per effort: an open effort holding 15 or more files — a commit
  landing its work would have closed it) and `landed-in-progress`
  (turn-end, to the person, once per item: an item still in progress
  though a commit landed its work and nothing is open on it); the two
  shared-files hints (turn-end: two open efforts on different threads
  changed one file within `sharedFiles.windowMinutes`; the Alert to the
  person once per pair, and a note to each agent, once per thread; see
  work-tracking.md "Cross-thread ownership"); and two
  metric ones, `metric-deltas` and `threshold-crossed` (see
  [metrics.md](./metrics.md)). **Bundled advisories set no coding
  requirements** (decided 2026-10-07): they say what oxplow observed, never
  how a project should code — no coverage target, no "add tests" — and
  a project that wants one writes its own advisory (the `coverage-target`
  nudge was removed for this).
  Each bundled hint has a fixture in `advisories.rs` that makes it fire
  and cases that keep it quiet (`every_bundled_hint_has_a_fixture_that_fires_it`
  fails when a hint ships without one). A count of `fired = 0` in
  `v_hint_stat` is therefore the project's state, not a broken query:
  `threshold-crossed` needs a metric with a `warnAt`/`failAt` that an effort
  moves past (only the coverage metric ships thresholds, and efforts
  rarely fall under them), `landed-in-progress` needs a commit-closed effort
  whose item is still in progress, and nothing is evaluated while
  oxplow-bundled is disabled, since its policy and hints are the
  extension's. A `coverage-target` row in `v_hint_stat` is residue from
  before that hint was removed. Advisories read
  stored views (`v_effort_metric_delta`, `v_effort_observation`), never the
  engine directly.
- The report-less-run nudge stays in core: it's about collection hygiene,
  not an instrument.

## Agents: the MCP surface

Everything a human can see or build, an agent can see or build too, over
MCP. That includes whatever **extensions** add: an extension's sources,
entities, dimensions, metrics and pages show up in the same generic tools
as core's, and its commands through `run_command`.

**Tools are declarations.** Core builds only the command bus's two tools,
`list_commands` and `run_command`. Every other tool is a command or a page
that asks to be one with `tool: <name>` on its `commands:` entry or its
page file, offered as `<namespace>_<name>` (the extension's `namespace:`):

```yaml
commands:
  - name: sql.query
    summary: Run ONE read-only SQL statement over the published `v_*` datasets. …
    scope: sql.read
    op: query
    tool: query_sql          # offered as oxplow_query_sql
# pages/open-work.yaml
tool: open_work              # offered as <namespace>_open_work
```

- **Typed results.** An operation that declares its result
  (`Op::returns::<T>()`, `T: JsonSchema`) gives its commands an
  `output_schema` (`CommandSpec.output_schema`); its tool lists that as
  MCP `outputSchema` and answers with the result as `structuredContent`
  beside the JSON text, so a client reads it typed. `sql.query` declares
  `SqlQueryResult` (columns, rows, `truncated`, `reads`, `freshness`,
  `generation`, `coverage`). A page tool declares none: what it returns
  depends on its `format`.
- oxplow's own tools are `oxplow-foundation`'s declarations over core's
  read operations (`oxplow_read`, `oxplow_query_sql`, `oxplow_show_page`,
  `oxplow_vcs_log`, …); no tool is special to core.
- A **command tool**'s description, schema and example are its command's,
  and a call runs it as the caller through the bus (validation, policy,
  proposals, audit). It's offered while its extension is enabled and the
  command is offered to the caller (`needs`), so the
  list follows the active implementations as commands do. Its read-only
  hint is its command's access (`View`/`Read`), unless the command
  needs a scope that spends the person's money (`Scope::spends`; today
  `ai.compute`, a model call on their key) or is an operation of one: it
  changes no record but isn't hinted, so the client asks before it runs.
- A **page tool**'s input is one property per param (its label, choices
  and default) plus `format`; a call runs `oxplow.page.run` and returns the
  page's text (`format: "json"`: its loaders' rows). Always read-only.
- A name is `[a-z][a-z0-9_]*`, at most 48 characters; two entries offering
  one name — in one extension, or two under one namespace (oxplow's own
  share `oxplow`) — are refused at load, on each.
- **Typed page tools.** `oxplow-foundation/pages/` holds the pages an
  agent asks for most, each `hidden: true` with a `tool:`: `catalog
  { dataset? }` (every dataset with its columns), `thread_work { thread }` (the
  thread's items by state, with their efforts), `work_next { thread }` (what
  to pick up next; the bundled `/work-next` command calls it) and
  `effort_summary { effort }` (title and summary, item, files, test runs,
  claims, decisions as sections). Params take bare ids (`thr3`, `eff9`).
  Each reads published `v_*` datasets only, so it works with the bundled
  extension off. For history, `oxplow_vcs_log` and `oxplow_diff` are
  commands, not pages.
- Every agent-invokable command stays reachable through `run_command`
  whether or not it's a tool: a tool is a named entry for the ones an
  agent reaches for often, kept few so the list stays readable for weaker
  datasets.
- Listing and dispatch: `oxplow_app::agent_tools` (`offered`, `tools_of`);
  the MCP server (`crates/oxplow-mcp`) lists and resolves them per caller.

**Understanding the semantic layer**

- `oxplow_query_sql`: read-only SQL over the published datasets. `v_dataset` /
  `v_field` / `v_dataset_lineage` describe every view (core `v_*` and
  extension `v_<ext>_<entity>`) with its column docs and inputs;
  `v_dimension`, `v_metric_spec` and `v_measure` list the dimensions,
  metrics and measures, extension-declared ones included, each with its
  scope.
- A metric's numbers are SQL too: `SELECT bucket, MEASURE('<key>') FROM
  metric_grid('day'[, '<dim>'])` — sliced by a dimension, in any bucket.
  Changing metrics is the `metric.*` commands (`run_command`).

**Working with pages**

- `v_page`: every page (`uri`, `extension`, `id`, `title`, `description`,
  `hidden`), with `v_page_param` (its params) and `v_page_answer` (the
  questions it answers); `v_extension` and `v_contribution` give the
  extensions and what they contribute.
- `oxplow_find_page { question?, about? }` (`oxplow.page.find`): the pages
  that answer a question, each with its params. Look here before writing SQL
  for something a person will look at. With `about`, a resource page is
  found when `pages/renderers.rs` offers it for that uri.
- `oxplow.page.renderers { uri, stream_id? }` (through `run_command`): the
  pages that can draw a uri, in Open With's order (`page`, `title`,
  `default`), as the window's `renderers_for` lists them.
- `oxplow_run_page { page | spec, params?, format?, loader?, draft? }`
  (`oxplow.page.run`): **what the UI shows** for those params, as the
  page's text by default, or with `format: "json"` each loader's columns
  and rows (up to 200) and the tree. `spec` is an inline page of the
  agent's own (YAML); it only reads. `draft: true` reads the stream's
  draft of the page's extension.
- `oxplow.page.describe { uri }` (through `run_command`): what a page file
  is made of, its source file, params, loaders and node tree, and which
  page and file each node comes from. `uri` is the file on
  `oxplow:extensions` (the running page; an older `rev`, that deployment's
  copy), `oxplow:temp` or the worktree (`text` is the draft); a file that is
  no page gives no rows.
- `oxplow_show_page` (`oxplow.page.show`): shows the person a page beside
  the conversation, a saved one with params or the agent's own `spec`.
- **Text rendering** (design rule 17, `crates/oxplow-app/src/viz_text.rs`
  and `pages/text.rs`): one renderer per view node kind, and each node
  appears under `[node <id>]` (an embedded page's under `[node
  <embed>/<id>]`), so the agent can point the person at one with
  `oxplow_navigate` and `#node=<id>`. A table or list is a markdown
  table of the displayed columns; a number or markdown node is its first
  cell; a bar chart is its labels and values with the total; a line chart
  pivots to one column per series with each series' total; a treemap
  lists labels by size, largest first; a tree is an indented list; a
  timeline is `at — label (URI)` lines oldest first; a detail is
  `**label**: value` lines; steps are a numbered `[x]`/`[>]`/`[!]`/`[ ]`
  checklist; hunks are each file's unified diff between its two revisions
  (up to 20 files, 8 KB each). What a node needs beyond its rows — the
  hunks' diffs, a `code` node's files, an `approvals` node's proposals —
  is `viz_text::resolve`d first (`text_of`). Tables stop at 50 rows and
  say how many more; an empty result says the node's `empty` text. It is
  what `oxplow_run_page`, `oxplow_get_open_page` and `copy` return, so an
  agent reads a page the way the person sees it.
- `oxplow_get_open_page(thread_id)`: what the human has open in that
  thread, for any page kind (a work item's, file's or a custom page's
  URI): `pageId` is its identity (the URI without its `#`) and the view
  the `#`, decoded. For a page it adds `pageRun`, the window's own run of
  it as text, with `focus`, the node the person is on. `others` lists the
  page each other window has open there; `tab` reads one of the thread's
  tabs instead (an id `oxplow_list_tabs` listed).
  - The UI reports the active page (`report_open_page`, UI-only) together
    with whatever that page published to `tabs/openPageDetail.ts`.
  - The state is held in memory on the thread runtime
    (`ThreadRuntimeRegistry::open_page`).
  - A hidden-but-mounted page can't overwrite the report, because only
    the active page's detail is sent.
- **Row actions:** right-click a page row → "Ask the Assistant" puts the
  row in the agent's input.
- **What a person can ask:** what pages say they answer
  (`answers:` in a page file; `about` makes it a question on pages for that
  kind of ref; a question is **one line** — a pasted line break is Enter in
  the terminal, so a multi-line one would send itself — checked by
  `answerability::check_pages`; see "Answerability"). It is read as
  **`v_page_answer`** (with `v_page`), beside **`v_config_key`**
  (`config_key`: project.yaml's keys with their doc and value schema), with
  `oxplow_query_sql`; there is no catalog RPC and no separate prompt list
  (`intent.prompts`, `v_prompt` and `prompt_catalog` are gone).
  - The UI: the **Catalog** page (oxplow-foundation's `catalog`, with
    `catalog-prompts` — grouped by the page's category (else its
    extension), with an Ask row action on `oxplow.agent.draft` — and
    `catalog-config-keys`, with Open Settings as a page action on
    `oxplow.window.navigate`); the nav bar ☰'s Ask (suggested prompts for
    the page's resource, `usePromptCatalog` over `v_page_answer`); and
    `EmptyState`'s prompts (usability.md → "Empty states").
- **No launcher entries.** The manifest has no `launcher:` list. A page is
  a file in `pages/`; a command — one that runs another with fixed input,
  or puts a prompt in the agent's input (over `agent_input.write`
  `draft`) — is a command the extension declares with a `ui`, offered like
  oxplow's own (`commandOffers`). The launcher lists the pages that have
  a `category`.
- **Page actions:** `actions:` are commands. An agent presses one with
  `oxplow_run_page_action` as the page acting for it (see "Actions are
  commands"). A command the agent may not run, or one that needs a
  person's confirmation, is refused.

**Building**

- Agents author extensions and pages **by editing files** under
  `.oxplow/extensions/<name>/` with their normal Edit tool, under the usual
  write guard. Editing a source changes nothing until
  `oxplow.extension.deploy { name }` ("Sources and deployments"); the
  authoring tools read the source, so the agent checks its work first.
- **No scaffold tools (decided).** A scaffold tool would write project
  files outside the write guard and the caller's worktree. The skill carries the templates instead, and humans have
  Save as Page in Explore Data. The same rule holds for metrics:
  the `oxplow.metric.scaffold` command returns the metric extension's files for
  the agent to write itself.
- `oxplow_validate_extension(name)` returns load errors, schema errors and a dry
  run of every page (`pages::checks`: each SQL loader runs once, a `columns`
  key the query does not return is an error, `layout_warnings` flags a
  layout that is likely wrong), so the agent can check its work without
  the UI. `oxplow extension check` is the same report.
- `v_extension`, `v_collector_state`, `v_collector_credential`
  (set or not, never the value), the `oxplow.collector.sync` command (never grants).
- **Worktree streams: preview, don't run.** Collected data is
  project-wide (one `ext__<ext>__<entity>` table), so a run (the
  `oxplow.collector.sync` command over `CollectorRunner::sync`, from any caller)
  always runs the **live** copy. An agent
  writing a collector tests it with `$OXPLOW_BIN extension test <name>` (a
  throwaway, through its examples); MCP
  `oxplow_preview_collector(owner, id, stream_id)` dry-runs what is
  deployed against that stream's files and data: `collector_runner::
  preview_collector` runs the deployed entity collector through the same
  `produce` step (same permissions: an exec collector needs
  `process.run`, and so does a Starlark one that calls a
  dataset need `ai`; other derived collectors need neither) and returns the coerced
  rows per entity (first 50, plus totals), storing nothing and recording
  no run. A **fact collector** (`facts:` declared) previews through the fact
  engine instead (`MetricsService::preview_fact_collector_in`, the same
  runner `oxplow extension test` uses): it runs that worktree's script over
  the stream's latest snapshot and returns `{ owner, id, facts }`, the
  facts it would record, recording nothing. One that reads `input` rows is
  previewed through an example, and a program (`exec`) is never previewed.

**Teaching the agent**

An `oxplow-extension` skill (shipped in `crates/oxplow-agent-text/assets`)
teaches the format, the `v_*` contract and the loop "read `v_dataset` →
oxplow_query_sql → write files → oxplow_validate_extension → oxplow_run_page".
Its page references (`page-format.md`, `loaders.md`, `view-nodes.md`,
`recipes.md`, …) describe the page file. Add to Context on a page puts
the page's URI in the thread's basket.

**Getting newcomers there.** "New Page with Your Agent…"
(`oxplow.page.draft_with_agent`, a Tasks command in the launcher; needs a
thread) puts a starter prompt into the agent's input, unsent. It never
sends; the person finishes the sentence.

## Example extensions (`examples/extensions/`)

Each loads clean (`documented_examples_load_without_errors`):

- **`github`** — an `exec` collector bringing pull requests in as the
  entity `v_github_pr`, with pages joining them to streams and tasks
  (`prs-by-task`, `pr-lifetimes`); and (P10) a pull request as a
  **resource**, `github/pr` — dataset `pull_request` (`uri`, `title`,
  `body`; it resolves and is searchable), the page `pr` (`for: github/pr`,
  given `uri=<uri>`), `[[pr:12]]`. `crates/oxplow-
  sdk/tests/just_works.rs` checks and tests it; its `a-pull-request-resolves-and-opens-on-its-page` fixture opens a pull request.
- **`session-history`** — a read-only page of the thread's earlier prompts
  from `v_agent_turn`, mounted at `new-session.sections` after `start`.
  `the_session_history_example_adds_a_section_to_the_new_session_page`
  checks the example and proves the mounted loader reads a recorded prompt.
- **`schema-review`** — a review policy of its own: a fact collector
  counting DDL per migration file (its own measure and dimension), a
  dataset joining those facts with core's evidence (`v_effort_file`,
  `v_test_run` and its provenance, `v_effort_observation`'s diff
  coverage, `v_change_function`) into a verdict, and a page
  (`effort-verdict`) replacing `effort.verdict` with it — no core change.
  `the_schema_review_example_joins_its_facts_with_core_evidence` runs it
  over a real effort.

## The `oxplow-bundled` extension

**Two shipped extensions, on purpose** (decided 2026-10-28). oxplow ships
`oxplow-foundation` (required) and `oxplow-bundled` (optional, on by
default), both in the `oxplow` namespace so a contribution can move
between them — or the two merge — without a rename. Foundation is kept
as small as it can be while still useful: the host's pages, the default
capability implementations (work items, effort policy, snapshots,
knowledge) and the measures core's change analysis records. Everything
optional — the review policy, analytics, the code metrics and their
collectors — is bundled's, and turns off with it. Don't add a third
shipped extension; bundled's contents are the next thing to sort out.

Everything oxplow ships that measures, visualizes or reviews is this one
extension (`extensions/oxplow-bundled/`): its namespace is
`oxplow_bundled` (commands, event types, measures, collectors) and its
datasets publish as `v_oxplow_bundled_*`.

What moved out of core, and what it is now. All of it is pages (files in
`pages/`, read by `v_page`); oxplow-foundation's pages draw the app's own
screens and oxplow-bundled's add the review and analysis:

- **Dashboards and review.** `planning`, `review`, `quality` and `usage`;
  the code-quality `findings`; the `change-review` page (treemap,
  look-here-first, functions, co-change, zones) mounted in the
  `oxplow-foundation/diff-view.sections`, `commit.sections` and
  `uncommitted-changes.sections` slots. The duplicate scan runs in core's
  change analysis; the compare page is foundation's `duplicate-block`.
- **Effort blocks.** `effort-tests` (coverage, untested files, test runs,
  failed tests, analysis findings), `effort-metric-deltas`, `effort-nudges`
  and `effort-plan`, mounted at `oxplow-foundation/diff-view.sections`.
  `task-tokens` mounts at `work-item.sections`; `thread-tokens` at
  `tasks.sections`.
- **Metric nudges.** The `threshold-crossed` advisory (with
  `metric-deltas`).
- **Git History's commit graph.** The `history-graph` component, replacing
  core's plain commit list (`vcs.history.graph`, a `ui.mounts`
  entry).
- **The app's screens.** foundation's `alerts`, `backlog`, `board`,
  `catalog`, `changes-effort`, `comments`, `commit`, `dashboards`,
  `diff`, `directory`, `file-viewer`, `files`, `git-dashboard`,
  `settings` (an `area` param: overview, agents, lsp, extensions, data,
  integrations, ai), `tasks`, `uncommitted-changes`, `wiki-page`,
  `work-item`, and more. Dashboards are in [dashboards.md](./dashboards.md) and
  metrics in [metrics.md](./metrics.md).
- **Rail panels** (the `panels:` list of oxplow-bundled): Go To
  (`go-to-panel`, collapsed `go-to-bookmarks`), Waiting on You
  (`waiting-on-me`, a badge), Comments (`comments-panel`), Uncommitted
  (`uncommitted`, collapsed `uncommitted-line`, count `uncommitted-count`)
  and Work (`work`, collapsed `work-line`, count `work-count`). Only the UI
  moved: stores, commands and views stay core. Each panel's `body` is a
  page.
- **Effort reasoning.** The AI profile `reasoning` (fallback `main`), the
  job `reasoning`, the command `oxplow.effort.infer_reasoning` and the
  effects `infer-reasoning` / `store-reasoning`, writing through core's
  `oxplow.effort.replace_inferred_decisions` and
  `oxplow.effort.replace_inferred_claims`.

**Owns what reads tables (P7.B5).** Derived data computed only from
core's rows is the extension's, as SQL: the "look here first" score is
its dataset `change_interest` (`v_oxplow_bundled_change_interest`, over
`ref('change_file')` and `ref('change_function')`), which the
`change-review` page reads; co-change surprises are its datasets
`co_change_pair` (`materialize: { every: 1h }` over the commit index —
"the last 180 days" moves with the clock) and
`change_co_change`, which `change-review` reads — and the
`file-co-change` page ("Usually Changes With", mounted at the diff
page's `file-header` slot, `oxplow-foundation/diff.file-header`) reads `co_change_pair` for the file a diff
shows; an
effort's churn is its fact collector `oxplow_bundled.effort_churn`
(`on: [effort.finished]`, `after: [change.analyze]`, reading the
effort's change files) recording `oxplow_bundled.effort_churn_lines`.
Full-tree duplicated lines read the tree, not tables, so they
are oxplow-foundation's collector, `oxplow.duplicate_lines`, over the
public `duplicate_blocks(min_lines)` builtin — what any extension could
write. What needs two revisions' trees — files, functions, imports, test
signals, per-function churn, the scoped duplicate scan — stays in core's
change analysis.

**Deliberately simple.** A basic **metrics explorer** and **simple
dashboards**. The base version lets people see what metrics exist and
pin a few to a dashboard as a starting point, a bit like Metabase. It does
not need to be fancy. Explore Data (oxplow-foundation's `explore-data`
page) browses the semantic-layer catalog (entities, measures, metrics),
shows a table or simple chart sliced by a dimension, and saves the result
as a page. A dashboard is a page with `category: Dashboards` and a grid
of nodes ([dashboards.md](./dashboards.md)). Metrics is a catalog (search,
enabled/all, range, branch, sparkline + latest value); the metric page
shows trend, recordings, stats and Enable.

**Also stays in core** (it is substrate other features need): snapshots,
collection ingest, attribution, token ingest, page visits (the rail and
launcher use them), the fact store and engine, the Go To page and
bookmarks (`v_bookmark`; the rail's Go To pane is the extension's).

New in the extension: the **effort review packet**, all live exception
pages:

- Waiting on me
- What deviated (files outside the task's stated area)
- Unverified claims
- Decisions made
- Tests weakened (deleted/skipped tests, removed assertions)
- Missing co-change (files that historically change with these but didn't)
- Read this first (risk-ranked hunks with evidence badges)
- Struggled here
- Copy review prompt (for reviewing with a second harness; the human
  pastes it)

**Done when** core boots and is usable with `oxplow-bundled` disabled,
and enabling it restores the review and analysis pages.

## Added for extraction

Features added to core because the extraction needed them,
available to every extension:

- `bar`, `line`, `treemap` and `grid` viz, with `chart` and `children`.
- `commit` and `metric` link kinds; `file` links with `line`. (A `task`
  kind came and went: a work item links as `page`, by its URI.)
- the Work Item and Tasks pages' section slots, with slot params checked at load.
- Page `category` and `hidden`.
- Page `actions:` as commands.
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see "Advisories").
- Page tabs carry params: `oxplow:oxplow/page?page=<ext>/<slug>&k=v#view`,
  so a slot page's heading opens its page with the slot's values, and
  history and bookmarks keep them.
- Panel `collapsed:` — a page shown as a collapsed panel's summary
  (for the Work panel).
- Panel `count:` — a page whose row count (or `number`) is the panel's
  header count without raising an alert.
- Column `icon:` naming any page kind draws that page's icon.
- Page param `options:` — a choice, shown as a toggle in a panel's
  header and a select on a page (for Go To's Recent / Most
  visited).
- Node `group: { by, link? }` on `list` / `table` — rows under linked
  sub-headings, also in compact panels — and actions with `group:` in a
  group's heading.
- Column `icon:` / `tone:` from fixed vocabularies, and node `emphasis:` /
  `depth:` — row highlight and indent.
- List, table and tree rows that link somewhere drag into the agent's
  context; tree labels honour their column's `link`.
- Collector pacing: an `on:` trigger's `settle` / `at_most` / `idle` /
  `force` (metrics.md "Pacing"), so an expensive collector runs
  once a burst settles or the project is idle — never losing a skipped
  snapshot's files — with `v_pending_run` saying it's updating.

Extension disk readers refuse absolute/traversing paths and symlinked folders
or path segments for source, bytes, directory listings and component bundle
metadata. Project fact inputs resolve under their checkout; symlinks outside
that checkout are refused before reading content. These host checks do not
provide OS confinement for an external program a deploy approved.

## Command candidate publication

Extension commands now stage as one enabled-source batch from a CatalogSnapshot.
Enabled non-required declarations with load errors reject the candidate, even
when their loader retained some valid commands. Building handlers does not
publish or execute them. Registry preparation validates all namespaces, IDs and
shortcuts; publication checks the original registry allocation identity and the
current catalog fingerprint. Any intervening registration/removal/replacement
requires restaging. Reconciliation yields and retries publication races up to
three times; persistent contention records diagnostics.

Rejected candidates retain previous command handlers for the remaining owners.
Disabled or removed sources are withdrawn independently, and captured command
RunContexts recheck that their owner is still enabled before execution. The
standalone command boundary remains separately usable; boot publishes its
candidate with dataset views, vocabulary, providers and UI metadata through the
generation coordinator. Runtime lookup uses the active catalog rather than
promoting current filesystem declarations. State hosts still enforce their
namespace and revision checks.

## Database generation candidates

`VocabularyService::prepare` freezes enabled vocabulary declarations and the
host work item recognizer from a CatalogSnapshot without writing or swapping.
Enabled load errors reject preparation. `PreparedVocabulary::replace_tx` uses
a savepoint and rejects the complete set on any event/ref/subscription or
recognizer diagnostic, restoring event contracts, retention and resource rows.
A changed recorded event schema and a shared wikilink prefix are candidate
errors; a candidate never publishes a partial result.

`PreparedExtensionDatabase` captures datasets, vocabulary and enabled contribution
descriptors from the same snapshot, and every entity the enabled collectors
declare: its transaction first creates each entity that has no view yet, empty
(`collector_store::ensure_entities_tx`; one that exists keeps its rows — they
are the collector's), so a dataset over an entity compiles before the collector
first runs. Without it an extension whose dataset reads its own collector's
entity is held back forever, its collector never active to fill the entity
(`the_documented_examples_activate`: the github example). Its transaction helper calls strict dataset
and vocabulary replacement and contribution restatement inside a surrounding
savepoint: any diagnostic or storage failure restores the whole candidate.
`prepare_with_capabilities` also includes prepared capability rows and switch
events, rejecting mismatched catalog identities. Vocabulary then uses the
candidate's planned work list recognizer rather than the old live selection.
It returns a vocabulary for publication only after the outer transaction commits.
Storage or rollback errors require the caller to abort; logical diagnostics stay
separate from retryable contention. This helper executes no external work.

The boot activation coordinator now uses these APIs with command, provider,
capability, adapter, collection-feed and component-bundle candidates. The old
capability, command, provider, dataset and vocabulary boot writers have been
retired from this path; config/entity-triggered dataset work is routed through the
coordinator. CapabilityRegistry holds core, declarations and external-instance metadata
under one lock. Each read resolves selection, features, hidden command namespaces,
recognition and database rows from one captured state. Candidate preparation
copies that state, replaces declarations without registering adapters, and derives
all candidate selections/rows from it. Publication checks an allocation identity
and the supplied host config's resolved choices/provenance before a single swap;
intervening mutations, another registry or changed choices require restaging.
Candidate publication fences config and catalog identity. Capability reads use
the active catalog's immediate disable/removal overlay and the config choices
captured with the last committed capability candidate; staging remains based on
the frozen candidate state. External instance metadata is captured; process and
adapter lifetimes are staged by the separate provider/runtime-adapter owners.

Provider resolution captures config/provider choice with project/global credential
scope in one merged mapping, and uses one catalog declaration snapshot per
generation. A disabled bundled owner cannot be shadowed by a project namesake.
These checks preserve owner/scope identity during preparation; they do not
coordinate provider process/adaptor publication with the other registries.

**Capability candidates.** `PreparedCapabilities`
(`crates/oxplow-app/src/capabilities.rs`) freezes candidate declarations,
resolved choices, features, work-item recognition and the
`v_capability_provider` rows from one catalog plus the registry's captured
state; each snapshot, selection, recognizer and row computation uses that
one captured implementation list, so feature resolution never rereads a
newer registry halfway through, and the selection and fallback rules are
the registry's own. Preparation starts no adapters or providers. Its
transaction helper restates the capability rows and switch events, rolling
back on failure. Publication checks the original allocation identity and
the supplied config's resolved choices and provenance; a changed registry
or config requires restaging. A prepared candidate conveys no authority.
`PreparedExtensionDatabase` can include those rows and switch events with
the datasets, vocabulary and contribution descriptors in one savepoint
(`prepare_with_capabilities`, above).

## Prepared native runtime adapters

`PreparedRuntimeAdapters` captures one CatalogSnapshot and constructs declared
work-item, harness, effort-policy, knowledge, snapshot and model-provider
maps without changing running registries. It rejects enabled load errors and
strict AI factory failures before any map publication. Script/gate objects
are constructed but no process, handshake, network call or paid probe runs.
External adapters/processes remain the provider lifecycle owner's work.

Each adapter subsystem exposes prepare/publish only (no separate register
helper); partial factory skipping, where it applies, is logged.
The boot coordinator publishes the prepared aggregate under the shared activation
fence, with compatible SQL and command readers. Capability and command
reservations are held across the commit and map swaps. A required-bundled core
baseline is active before strict full activation: baseline failure prevents the
daemon from serving; full-candidate rejection retains the baseline or last good
generation.

**One broken extension doesn't hold back the rest.** Before staging,
`CatalogSnapshot::admitted` builds the candidate from the raw load with each
enabled extension that can't load (its own errors, a blocked contract, or a
required import whose owner is out) replaced by its version in the active
generation, or left out when it has none; everything else activates together
as one coherent generation. Required bundled extensions are never replaced:
their errors still reject the candidate. The held extensions and why are the
activation's `problems` (Settings → Extensions), so the reason is visible
while the last working version keeps running. A held extension's declarations
are its last working ones; its scripts are read from disk as they are now.
A failure later in staging that names one extension — a dataset whose SQL
doesn't compile (checked inside the publication transaction, which rolls
back), a provider or capability that can't stage — holds that extension the
same way (`extension_catalog::Failed`) and the candidate is staged again; if
its last working version fails too, it is left out. A failure that names no
held-able extension (the host) rejects the candidate. A command namespace
collision names only the source refused and the one it collided with
(`commands::NamespaceRefusal`). The active catalog is separate from raw
candidate loads; its `source_generation` preserves the raw fingerprint. Runtime catalog reads use
the committed snapshot and apply immediate explicit-disable and missing-folder
revocation without loading edited declarations.

`ExtensionActivation::activate` now joins the prepared database, commands,
capabilities, native and external adapters, vocabulary, collection feeds and
component bundles under one owned reader/writer boundary. External processes
start and validate before SQL publication; the running-map reservation is acquired
before the DB permit, then held through commit and adapter swaps. Retired processes
shut down, successful startup health is recorded and initial sync is queued after
the publication transaction and reader boundary are released. Command owner metadata, command handlers, host config
and capability state are reserved on the blocking worker before commit and held
through all map swaps. Wiki/snapshot follow policies use the candidate's captured
selection, avoiding recursive capability/config reads while those locks are held.
Precommit rejection retains the previous active snapshot. A panic after the
commit (the in-memory maps part swapped, the boundary refusing readers) is
recovered at once: the database is the truth, so the activation stages the
generation again from it and publishes that, which clears `unavailable`; if
that fails too, the boundary stays unavailable and the failure is a `<host>`
problem in Settings → Extensions. Cancelling the async caller does not release
the worker's writer while it can still commit. Test hooks (`panic_at`) inject
the panic right after the commit or after the maps and providers are published,
any number of times, so both points and the failed restage are pinned
(`a_panic_after_the_maps_are_published_recovers_the_generation`,
`a_panic_on_the_restage_too_leaves_the_boundary_unavailable`).
Activations serialize on one lock: a second `activate` waits and both return the
same generation, and an edit landing while a candidate is staged rejects that
publication ("catalog changed while activation was staged"); the next one
carries the edit.

**Who a publication waits for.** Every pin is listed while it's held, with
what holds it (a run's contribution or actor and run id, or the reading
future's type for a bare `read`) and since when: `ExtensionActivation::readers`.
A publication waiting on readers logs that list every 10 s, keeping its one
write future (its place in the fair lock's queue). Pinned by
`a_waiting_publication_names_the_readers_it_waits_for` and
`a_panic_after_commit_recovers_the_generation`.

`GenerationPin` is host-only and conveys no authority. RunContext can retain an
opaque pin across spawned host work; child contexts inherit it and fresh jobs do
not. Nested reader scopes reuse their pin so a waiting fair-lock writer cannot
deadlock a parent query. SQL and command entrypoints pin before query materialize
or command lookup; page/effect/event-consumer paths use the same reader scope, and
host workers carry the concrete pin in RunContext. Task-local scopes alone do not
propagate into spawned tasks. Provider host-call registration is cancellation-safe
across candidate publication. Native/background and synchronous metadata
entrypoints still need a complete audit. Metric declaration rows now join the
publication transaction; state-value recapture remains a post-publication reader. The renderer receives the published metadata change signal;
a deploy, a rollback or a config change triggers candidate preparation (file changes never do). Until the first
generation is activated (boot's `initialize`, before ingress opens), no reader is
served: the generation is the only publisher of commands, datasets, vocabulary,
providers and capabilities.

Custom component preparation hashes each bundle's files in its deployment and
keeps their names, never their bytes. Publication swaps the whole active bundle
map and adds its bundles to the process's loaded versions, which a previous
frame is still served from while its deployment is on disk. A retained frame
from a different activated generation must reload before invoking a query or
command; a bundle of another root is built from that root's files. This check
requires the owning runtime reader scope for atomicity with command or query
execution.

## Runtime ingress and entity refresh

Extension listings enter the same generation scope as commands and SQL. A failed
candidate adds owner diagnostics to the committed metadata; newly discovered
packages remain visible for repair with all runtime contributions cleared.
Clearing rejection diagnostics emits a runtime change signal. Candidate bundle
and owner catalog publication precede external-provider admission under the
writer, so callback live checks cannot mistake a just-admitted owner for the
previous generation. Callback registrations use a drop guard: cancellation
releases retained run contexts and their generation pins. Explicit stop, disable,
removal and changed credential scope deny admitted callback authority immediately.

**Reading the catalog.** Every read of the activated declarations is one of
four, and a test build checks the rule: `runtime_snapshot` (and `get`, which
wraps it) panics when no generation pin is in scope and a generation is
active, so a new unpinned reader fails the suite where it first runs.

- *Pinned* (`ExtensionActivation::read`, or the pin a `RunContext` carries):
  a reader that joins declarations to another generation-owned map —
  commands, capabilities, providers, datasets. The agent-text refresh and the
  effort policy's owner lookup pin for exactly this. A task-local does not
  cross into `spawn_blocking`; use `ExtensionActivation::spawn_blocking`,
  which carries the caller's pins (the command router does).
- `ExtensionCatalog::unpinned(root, reason)`: a reader whose whole answer
  comes from the one snapshot it takes (an effect or collector matched on its
  declaration, a lookup by name, a listing). The `reason` says why that is
  enough — typically that what it starts pins for itself.
- `live_snapshot(root)`: a revocation gate — is this owner still enabled and
  current now — for callbacks, job authorization and publication, a provider's
  start check, secret ownership, and the generation stamp of a new run. It
  must answer while a publication waits on readers, so it holds no pin.
- `ExtensionCatalog::staged`: the private catalog one candidate is prepared
  in (metric declarations); it is no generation a reader could straddle.

Collector scheduling and fact-collector lookup retain a reader before selecting
declarations. A carried RunContext pin can re-enter a reader on another task
without acquiring a second fair-lock reader behind a waiting writer. Entity-run
notifications compare the current entity view/version/SQL/column/key contracts
with the signature captured in the publication transaction. Data-only runs do
not restart providers; changed entity schemas request one coherent candidate.
Provider initial sync and entity-state metric recapture do not themselves emit
collector-run completion events.

## Jobs

A **job** is what an extension declares (`job:<extension>/<id>`); a **job
run** is one execution of it (`job_run:<id>`, a row of `v_job_run`). A job
is model work an extension asks for that runs outside any
transaction and outlives a restart (`operation: ai.complete_typed`). It
streams its reply as it comes, may call tools the extension declares,
can be cancelled mid-reply, and says how it ended with an event. The
declaration is `extensions/jobs.rs`; intent, snapshot, cancel and the
end event `extension_jobs.rs`; what runs `extension_job_runner.rs`;
claiming, supervising and publishing `extension_job_dispatch.rs`,
`extension_job_scheduler.rs` and `extension_job_authority.rs`.

```yaml
jobs:
  - id: review
    description: Review the change's TODOs.
    version: 1                        # the declaration's own contract version
    operation: ai.complete_typed
    needs: [ai.compute, sql.read]     # ai.compute; sql.read adds the query_sql tool
    tools: [evidence.lookup]          # this extension's commands the model may call
    max_tool_rounds: 4                # model rounds at most, 1–16 (default 4)
    timeout: 300                      # seconds, 1–600 (default 120)
    replay: never                     # or idempotent: run again when cut off
commands:
  - { name: review.ask, summary: …, scope: jobs.write, op: enqueue }
  - { name: review.cancel, summary: …, scope: jobs.write, op: cancel }
  - { name: review.result, summary: …, scope: jobs.read, op: snapshot }
  - name: evidence.lookup             # a tool: reads, admits a job, says its input
    summary: The TODOs in one file.
    entry: commands/evidence_lookup.star
    needs: [sql.read]
    input_schema: { type: object, required: [path], properties: { path: { type: string } } }
    access: read
    job: true
```

- **A tool** is one of the extension's own commands that admits a job
  (`job: true`), only reads (`access: read`) and has an
  `input_schema`; anything else is a load error. The model calls it by
  its local name with `.` as `_` (`evidence_lookup`: dataset APIs allow no
  dots); its description is the command's `summary`, its parameters the
  schema. `query_sql` is reserved: with `sql.read` the dataset gets it, a
  read-only query over the published datasets (as MCP `oxplow_query_sql`; 200
  rows). Core's commands admit no job, so a dataset reads oxplow through
  `query_sql` and never writes: a job's writes come from an effect on
  its end.
- **Permissions.** A job runs under its extension's `permissions:`
  ("Permissions"): the commands its tools name outside the extension's
  namespace need `commands:<pattern>`, and its model calls need `ai`;
  the loader refuses the extension otherwise. A person approved them when
  the extension was deployed, and a deployment is immutable, so a job
  runs the declaration that was deployed — a new tool arrives with a new
  deploy. Every model call also needs its AI provider's approval.
- **Enqueue** (`jobs.write/enqueue`): `{ job, intentKey, request }`,
  `request` `{ schemaVersion: 1, role, messages, tools? }` — never a
  dataset, endpoint or credential: the role's dataset is bound when it runs.
  A job that declares `tools` takes none in its request. One intent key
  is one job (asking again returns it). At most 1024 queued or running
  in a project, 256 per extension. The request stays private (it isn't
  in the audit or any dataset).
- **Running.** The scheduler (four at a time, polling each second)
  claims a queued job, resolves its authority afresh — the declaration
  and permissions as they are now, the thread's open-thread gate (a Write-scope need must be bound to an open thread: `JobExecution.needs_thread`) — and runs it
  as `Actor::Job` (`job:<id>:<attempt>:<fence>`) on a run of its own,
  linked to the run that asked. Each model round streams on the role
  (`AiCompute::stream_typed`, a recorded computation: the same request
  reads its recorded answer, replayed at once). What comes is the
  attempt's **output** (`extension_job_output`, `v_job_run_output`): text
  coalesced into a row every 100 ms (or 4 KiB), each tool call and what
  it answered, each round's usage. Each write is its own short
  transaction — none is held while the dataset writes — and the table
  refuses rows for an attempt that has closed, so a worker that lost
  its attempt adds nothing.
- **Tool rounds.** A reply with tool calls (from a job that declares
  tools) runs them, at most 16 a round: `query_sql`, or the command
  through `commands.run_in` on the job's run — the bus admits the job
  as a job, validates the input against the schema and re-checks every
  child a script composes (none of core's admits a job). A refusal, an
  invalid input, an unknown tool or a failure answers the model with
  `{ error }`, and the job goes on; a script tool answers its script's
  `result`; more than 64 KiB is refused. The calls and their answers go
  back to the model as its next round. Past `max_tool_rounds` the job
  fails (`tool rounds exhausted`). A job without tools keeps any tool
  calls in its reply as data.
- **Its result**: `{ parts, finishReason, rounds, usage: { inputTokens,
  outputTokens }, aiCalls }` — `parts` is the last round's reply; a
  failure adds `error`; a cancelled job keeps the parts it had, with
  `stopped: cancelled`. Enqueue returns `{ jobRun, state, created }`;
  `jobs.read/snapshot { job_run }` returns `{ jobRun, job, intentKey,
  version, state, attempt, cancellationRequested, text, toolCalls, result
  }`, `text` being what the current attempt has streamed so far (`jobRun`
  the run's URI, `job` the job's). The intent key is the run's subject (one
  run per key and job), so an effect on `contribution.job.completed` knows
  what the answer is about; `v_job_run.intent_key` publishes it.
- **Cancel.** `jobs.write/cancel` (the extension's own jobs) or
  `oxplow.job_run.cancel { job_run }` (any run).
  A queued job ends at once. A running one has its run cancelled: the
  dataset's connection is dropped within 100 ms
  (`oxplow_ai::client::unless_cancelled`), the attempt closes
  `cancelled` with what had come, and the call's `v_ai_call` row is
  `error: cancelled` with that partial reply. Disabling its
  extension stops it the same way.
- **Limits.** Past its `timeout` a job fails (`timed out after Ns`),
  keeping its output; the dispatcher stops a worker still running 15 s
  later. The lease is 30 s, renewed every 10 s: an attempt whose lease
  ran out (oxplow stopped) is fenced when recovery finds it. A `never`
  job then ends `interrupted`. An `idempotent` one is queued again in
  the same transaction (`extension_job_dispatch::requeue_tx`,
  `JobEvent::RetryInterrupted`), so it is never seen `interrupted`, and
  runs as its next attempt, up to `MAX_ATTEMPTS` (3) in all; the third
  cut-off is final. On the new attempt a round that had finished reads
  its recorded answer (a recorded computation is keyed by its request,
  not its attempt); the cut-off round is asked again, and its tools run
  again (they only read). Cancelling a job queued for its retry ends it
  `interrupted`, not `cancelled`: the attempt before may still have an
  effect in flight. A queued job survives a restart and runs after it;
  one that was running is fenced once its lease is out, then replayed or
  ended as above. A running job whose
  declaration changes or whose extension is disabled is cancelled with
  what it had, never completed under the new declaration.
- **A job belongs to its extension's deployment, not to a catalog
  generation.** Its owner is the extension, its package version, the
  job's contract `version` and the deployment's hash
  (`JobOwner.deployment`, `extension_job.deployment_hash`); nothing records
  the generation it was queued under, so another extension's change
  neither strands a queued job nor stops a running one. A redeploy of its
  own extension is another deployment whatever its version says (a
  private extension may have none; a bundled one carries oxplow's): a
  running job's `live` gate fails and it ends `cancelled`; a queued one —
  nothing of its attempt ran — is rebound to the new deployment of the
  same contribution and contract (`JobEvent::Rebind`, the only change the
  job row's owner may take, queued to queued) and runs there. A changed
  contract leaves it queued with no operation to run. Pinned by
  `a_queued_job_is_rebound_across_a_redeploy_of_its_extension` and
  `a_running_job_stops_across_a_same_version_redeploy`. A job holds no generation pin while it runs (it
  can run for minutes, and an activation would wait on it): authorizing
  its claim, each tool call (the bus pins per command) and its
  publication each pin briefly, and each checks through the job's `live`
  gate that its own extension is still that deployment and enabled. Its run's `ContributionRef.generation` is `None`. The dataset
  call holds none either: `AiService::bind` resolves the role under a
  pin and returns a `Bound` (provider, model, the declaring profile), the
  `*_on` calls stream on it unpinned, and a recorded computation is keyed
  by and computed on that one binding. Pinned by
  `a_queued_job_runs_after_another_extension_changes` and
  `an_activation_does_not_wait_for_a_running_job`.
- **Collector and effect runs hold no generation either.** The same
  pattern: a run's `ContributionRef.generation` is `None`
  (`run_context::unpinned_contribution`) and `run_context::live_owner` is a
  `guarded` check that its own extension is still enabled, the deployment
  the run started under (`ContributionRef.deployment`; a same-version or
  unversioned redeploy is another one) and still declaring the contribution, run at each
  host call and at publication (a collector's rows commit inside
  `read_generation` after `check_authority`; a fact collector's capture
  likewise). A change to or the revocation of the run's extension stops it
  publishing under the old declaration; another extension's change doesn't
  touch it. The state host decides a run with no generation by the version,
  enabled and declaration checks. `oxplow.collector.sync` is an `External`
  command, which the bus runs after releasing its pin (`.context/commands.md`),
  and an effect's `handle` no longer pins the whole event: its script runs
  unpinned and the commands it composes pin per command. Pinned by
  `an_activation_does_not_wait_for_a_running_collector`,
  `a_collector_does_not_publish_across_a_change_to_its_extension`,
  `a_collector_does_not_publish_after_its_extension_is_disabled` and
  `an_effects_run_holds_no_generation_and_ends_with_its_extension`.
- An extension has at most 256 queued or running jobs (a
  repeat of an intent it already has is not a new one). No database
  transaction is held while a model streams (`Database::write_transactions_open`
  is zero where a model is streaming, asserted in the tests). These are
  pinned in `extension_job_hardening_tests.rs` and
  `extension_job_crash_tests.rs`.
- **The replay decision.** An `idempotent` job cut off by a crash or a lost
  lease runs again, up to three attempts in all; a `never` job (the default)
  ends `interrupted` and is never run again on its own. Either way the
  end is announced once, in the transaction that ends it. A crash at any
  persistence boundary leaves the job `running` with its lease, and
  recovery (after the lease) decides as above: after the claim (nothing
  started), before the publish transaction (the worker's answer is lost,
  and a replay reads it back from its recorded computation), and inside it
  (the transaction rolls back whole, so the end and its event are both
  unwritten). A worker that returns after recovery fenced its attempt
  changes nothing: its result is kept once as an observation
  (`v_job_run_observation`: `attempt`, `fence`, `outcome`, `evidence`).
  Extension disabled, or the declaration changed, between the worker
  returning and the publish transaction cancels the job at once, with the
  result it had as a `stopped: cancelled` one and the same late
  observation: the worker has returned, so there is no lease to wait out.
  Two dispatchers racing for a queued job: the revision check lets one
  claim it. A repeat of an intent is the same job; the same intent with
  other input is refused as a conflict, even when the queue is full.
- **Its end** is the event `contribution.job.completed@2 { job_run, job,
  version, attempt, fence, outcome: succeeded | failed | cancelled |
  interrupted }` (`job_run` and `job` URIs; an effect keeps to one job with
  `where: { job: "job:<extension>/<id>" }`), logged once, at the real end, in the
  transaction that ended it (however it ended: completion, cancel,
  recovery; an attempt queued again logs nothing) with the anchors and
  cause the job was asked with. It carries no request or result: an
  effect reads the result with `scope("jobs.read", { job_run })` and writes
  what it says (the triage example's `publish-summary`).
- **Seeing job runs.** `v_job_run` (state, attempt, rounds, tool calls,
  tokens so far, its `uri`) and `v_job_run_output` are published datasets, so
  a page or component over them follows a run live. oxplow-bundled's **Job
  Runs** page (Activity) lists the running and the last day's, with Cancel;
  a row opens its **Job Run Output**. A job run acts as `Actor::Job`, its
  source the run's URI (`job_run:<id>`); attempt and fence are on the row.
- **What a job can't do**: drive an agent session, ask a person (a
  command that needs confirmation is refused, never proposed), write
  (its tools read), run core's commands, or call a model its extension's
  permissions and the AI provider's don't allow.

### AI profiles (`ai_profiles:`)

`extensions/ai_profiles.rs` parses `ai_profiles:` (`id` `[a-z][a-z0-9_]*`
unique in the extension, `description` ≤ 4096 bytes, `fallback` one of
the built-in roles `main | summarize | decide` (required), `features` of
`text | tool_calling | streaming | image`) into `Extension.ai_profiles`
(`AiProfileDecl`), each a contribution `<ext>/ai_profile/<id>`
(`ContributionKind::AiProfile`, `RunsOn::Declared`). It is a declaration
only: the person binds `<extension>/<id>` to a dataset, else the fallback
role's dataset serves it; `features` are what a bound dataset must support.
The extension names it by its bare id — a job request's `role`
(`extension_jobs::enqueue` qualifies it and stores `<ext>/<id>`, refusing
a name the extension doesn't declare), a collector's `role=` (qualified
with the run's extension by `CollectorOracle`). `ai_profiles::profiles_of`
feeds `AiService::set_profiles` from the catalog; resolution, features
and Settings are [ai-providers.md](./ai-providers.md) "Extension AI
profiles". `extension check` and `extension test` print a line per
profile (`oxplow_sdk::profile_lines`: bound, or falling back), and
`check` warns when the binding serving a profile forbids a feature it
declares (`profile_warnings`, a person's `restrict:`).

## Host-owned secret references

`oxplow-domain/src/extension_secret.rs` defines a strict version-1 identity:
`{schemaVersion:1,id:<canonical UUIDv7>}`. It contains no owner, path, keychain
account, secret bytes or authority. References may appear in state/config;
values remain in the machine keychain under the primary project and current
owning extension. Copying team configuration cannot approve use or read another
namespace. Disabled/uninstalled owners cannot save or inspect a reference;
explicit human revocation can delete a retained keychain value after uninstall.

`ExtensionSecrets` keeps the values; whoever sets one does it with a command,
`oxplow.extension.set_secret { extension, reference?, value }` (a fresh
reference when none is given) and `oxplow.extension.revoke_secret { extension,
reference }`, scope `credentials.write`. A secret is a kind of value, not a
kind of actor (commands.md "Secrets"): `value` is `writeOnly`, so a person, an
agent and a page for either may set one, and no record, event, result or
dataset carries it. Presence is the live dataset `v_extension_secret` (`extension`,
`provider`, `name`, `reference` as JSON, `is_set`), read from the catalog's
provider credentials and the keychain; it follows `CredentialChanged`. Secret
bytes and keychain error details are never returned or audited. Values are
bounded to 16 KiB. Settings → Extensions lists them in the foundation's
`extensions-secrets` page (Set… / Rotate…, a form over the command whose
`value` field is masked by the schema, and Revoke). The old
`save_extension_secret`, `extension_secret_is_set` and
`revoke_extension_secret` IPCs are gone.

The first trusted consumer is a remote MCP HTTP provider using the shipped
adapter. Its bearer credential must be named `OXPLOW_MCP_BEARER` and carry
`secretRef`; references are refused for package executables, stdio servers,
OAuth credentials and other environment variable names. The fixed name avoids
process environment control variables. The reference identity, endpoint and
declaration are part of the deployment. The host checks exact current
owner and permissions before and after keychain resolution and gives bytes only to the
shipped HTTP transport; mapping scripts get no secret reader. Redirects are
disabled in that transport. References remain project-local even for globally
configured provider instances.

Processes capture a host reference revision at admission. Save/revoke invalidates
that revision synchronously, refusing further sensitive callbacks and causing
the existing process revocation watcher to stop old execution. Consumer cleanup
is detached from the settings RPC's lifetime, then requests coordinated
reconciliation. Keychain mutation failure also invalidates old consumers; it
does not claim deletion succeeded. Runtime isolation/revocation checks remain
pending.
