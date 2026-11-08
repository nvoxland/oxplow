# Extensions: The manifest, contributions and configuration

`extension.yaml` (manifest v2), the contribution descriptors, namespaced configuration, private state, and the design rules. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## Contributions (descriptors)

Every contribution has one `oxplow_domain::contribution::Descriptor`,
built at load from the kind's typed declaration
(`extensions::descriptors::describe`, `Extension.contributions`): its
identity (`ContributionId { extension, kind, local }`; one grammar for
`local`, dots only for the kinds whose ids are dotted), its owner
(extension, origin and the extension's `version:`),
how it runs (`RunsOn`), its needs and `after`, and the **global name** it
takes in a space extensions share (a metric key, a fact collector's id).
Its `source` (`DeclaredAt`, published as `v_contribution.file` and
`line`) is a file inside the extension — `extension.yaml` with the
entry's line (`manifest_v2::entry_line` by its id, `item_lines` for
`ui.mounts`), a page's `pages/<slug>.yaml`, a dataset's SQL — never the
folder it loaded from (a deployment is a machine path). A page joins it
to the extension for a link (`extension || '/' || file`).
The kinds' table (`contribution::KINDS`: manifest key, title, whether two
extensions may share a name) is the one place those facts live.

**A contribution's only public name is its URI** (`ContributionId::reference`,
its `Display`; [resources.md](./resources.md) "Using URIs"): the resource
`oxplow/<kind>` with the kind's own name as the key param,
`oxplow:oxplow/<kind>?<kind>=<namespace>/<id>`
— `oxplow:oxplow/job?job=effort_review/review`,
`oxplow:oxplow/collector?collector=triage/todos`,
`oxplow:oxplow/page?page=github/pr`,
`oxplow:oxplow/dataset?dataset=effort_review/reviews` — or,
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
`v_<namespace>_<name>` view), event types (global: the type string; one
descriptor per type, its highest `v` the contract version), resources
(global: `<namespace>/<name>`), pages,
panels, slot mounts (`<host>.<slot>-<n>`), decorators (`decorator-<n>`),
replacements and custom components (all mounted, no global name).

**Every name comes from the namespace, never the name.** A dataset's view
(`v_<namespace>_<name>`), an entity's table and view (`ext__<namespace>__<entity>`,
`v_<namespace>_<entity>`), an event type (`<namespace>.<…>`), a command id,
and every contribution's id — `ContributionId { namespace, kind, local }`,
`<namespace>/<local>`: pages, panels, effects, collectors, jobs, AI
profiles (`<namespace>/<profile>` in `ai.yaml`), advisories, custom
components, providers and their instances (`extensionInstances:` keys),
resources — are named by the manifest's `namespace:` (default: the name,
`-` → `_`). So is what is stored under them: `contribution_health`
(`namespace` column), `collector_run.owner` (a collector's owner is its
namespace), `effect_state`/`effect_run`, jobs, `panel_layout`, settings
(`extension_config.namespace`), private state (`extension_state.namespace`),
collector and instance credentials and extension secrets (keychain
accounts). Everything oxplow ships is in `oxplow`, so a user sees
`v_oxplow_verdict`, `oxplow.review.accepted` and `oxplow/collectors`
whichever shipped extension declares them, and a piece moves between
foundation, bundled and the holder without a stored or shown name
changing. `oxplow` is the only namespace extensions share
(`refuse_shared_namespaces` refuses any other, `check_namespace` keeps
`oxplow` to what ships). Sharing one shares its names, so its extensions
coordinate: a second one declaring an id or a global name the first has is
refused (`Registry::add`'s id and global-name checks, `descriptor.collision`;
the "already belongs to" check in `datasets::compile_extensions`), and
their `configuration:` declarations merge into one document
(`share_configuration`, `config_schema::merge`: one `version`, no key two
declare; the later one is refused, `configuration.shared`). Settings and
state are forgotten only with a namespace's last extension.

What says *who declares* a thing stays the extension: `Descriptor::owner`,
`v_contribution.extension` (beside `v_contribution.namespace`),
`run::ContributionRef::extension` (whose grants, files and deployment a run
has; `RunContext::extension()` vs `namespace()`), `dataset.owner`, an event
type's registrant (only it emits the type), a retention window
(`extension_event_retention` is keyed `(namespace, extension)`), an entity
drop (`collector_store::drop_extension` drops the tables behind the owner's
own `dataset` rows, never by the shared `ext__<namespace>__` prefix), a
metric capture's scope, and decorator ids (positional, never stored). To
find the extension behind an id, look in its namespace for the one that
declares it (`extensions::declarer`, `ExtensionCatalog::find_page`); a
health row's extension is `v_contribution`'s for its `uri`.

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
`thread_answer` and dashboard tables.) A page's URI is `oxplow:oxplow/page?page=<namespace>/<slug>`
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

- Targets are contribution URIs (`oxplow:oxplow/dataset?dataset=effort_review/reviews`). An
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
Both layers can be replaced atomically when the schema version changes. An
untouched layer must already use the current manifest version; reads of old
versions fail closed. `inspect` supplies the old stamps the replacing write needs.
Version decrease is refused. Failure rolls back both layers; storage errors
abort the outer audited command transaction. The write runs no script and waits
on nothing external. Writes return stamps and omit both layer inputs from audit;
private compositions/proposals follow the state rules. `inspect` and `snapshot`
(and the stamps a write returns) carry `needs_migration: { project, personal }`:
whether a stored layer is at a schema version other than the declared one.
Reading such a layer fails closed with a `/schema_version` error, distinct
from a stale-stamp conflict (`/expected`).

**Changing the schema version.** There is no migration runner. When an
extension changes its configuration schema `version`, activation leaves stored
layers alone. A layer stored at the old version fails closed on every read
(`/schema_version`) until the extension's own command writes it at the new
version: a compare-and-swap with the old stamp as `expected`, both layers in
one audited transaction. Values are never discarded or rewritten by the host.

Reset uses an empty override object at a newer revision, retaining stale-token
rejection. Disable/uninstall retain local rows for explicit recovery/reinstall;
access is refused while the owner is disabled, missing or stale.
Both scopes live in the local project's database, with no global identity lookup.
Raw secrets do not belong in defaults, schema or overrides. Secrets are host-owned
references (extensions-permissions.md "Host-owned secret references"); this service resolves non-secret
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

oxplow-bundled uses it for its review-policy constants;
`oxplow.review_policy.read` / `.write` are its commands over the scope above.
The holder (`extensions-bundled.md`) keeps the advisories' settings and
`oxplow.advisory_policy.read` / `.write`.
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
State has no migration runner, and neither has configuration, and lifecycle cleanup
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
>     `.oxplow/extensions/` (see extensions-page-host.md "What works today" below);
>   - running and validating pages over IPC and MCP;
>   - the `oxplow-extension` agent skill;
>   - the launcher's page sections (a page's `category`);
>   - sharing: `oxplow.extension.share` / `unshare` move a source in or out
>     of git, and a teammate deploys it in their checkout ("Sources and
>     deployments": boot deploys nothing);
>   - Explore Data (oxplow-bundled's custom page, with Save as Page);
>   - **bundled extensions** (compiled in, read-only, reserved names).
>     `oxplow-bundled` holds the effort review packet and the analytics
>     pages; core works with it disabled;
>   - `exec` **sources** that bring external records in as entities. The
>     mechanics and decisions are in
>     [semantic-layer.md](./semantic-layer.md) → "User and extension
>     sources", and a tested example is in `examples/extensions/github/`;
>   - **slots** on pages (`oxplow/diff-view.sections`,
>     `commit.sections`, `uncommitted-changes.sections`,
>     `new-session.sections`, `diff.file-header`; oxplow-bundled's
>     `oxplow/work-item.sections` and `tasks.sections`), **advisories**, per-project **disabling**;
>   - page alerts, panels, and extension-declared measures, metrics,
>     dimensions and fact collectors (see extensions-jobs.md "Contributing metrics");
>   - whole-page and row actions;
>   - manifest v2 with `intent` and the stable/experimental split, the
>     catalog cache and the SDK: `oxplow extension new|check|test`
>     (extensions-sdk.md "The SDK").
>
> When a piece ships, move it from "target" to "current" here, in the
> same commit.

## What works today: Phases P6b and P8

- **P6b (2026-10-01).** An extension can also *act* and
  *extend the shell*: `commands:` (Starlark composing core commands, see
  extensions-commands.md "Commands"), `ui:` — `slots` in one dotted namespace (extensions-slots.md "Slots"),
  `commands` in core menus (extensions-slots.md "Commands in core menus"), `decorators`
  (stable since P10) — and `custom_components:` (stable since P11)
  rendered sandboxed by a page's `custom` viz or `component` node
  (extensions-components.md "Custom components"). Installs, updates and
  deploys are reviewed by what they change ("Reviewing by
  effect"). An agent's run that needs a person waits as a proposal
  ([commands.md](./commands.md), "Proposals").
- **P8 (2026-10-02).** The experimental kinds run, in a private
  extension: `event_types:` (extensions-events.md "Event types"), `ref_kinds:` (now `resources:`, extensions-events.md "Resources")
  and `effects:` (extensions-events.md "Effects"), through one swappable vocabulary. A change
  is reviewed by its impact — rows, collector outputs, effects'
  reactions — at any revision, on the CLI (`extension check --impact`)
  and in an effort's review (extensions-deploy.md "Reviewing by impact"). Datasets declare keys and may materialize on a
  clock or incrementally ([semantic-layer.md](./semantic-layer.md)).
  Changes go live with `oxplow.extension.deploy` (extensions-deploy.md "Sources and deployments").

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
permissions: [ai.compute] # optional: what it reaches beyond the baseline (extensions-permissions.md "Permissions")
requires: [tracker]      # optional: other extensions it needs on (extensions-deploy.md "Sources and deployments" → Requires)
build: { run: bun run build, output: dist, exclude: [node_modules] } # optional: built at deploy (extensions-deploy.md "Builds")
intent:                  # required
  purpose: What question it answers, or what job it does
  origin: oxplow:oxplow/effort?id=eff42   # the thread/effort URI that created it, or null
  examples:              # inputs → expected outputs; fixtures for `extension test`
    - { name: …, input: {…}, expect: … }
# kinds
measures:    [...]   # also what the project's own extension declares; each may carry a one-sentence `description:` (`v_measure.description`)
metrics:     [...]   # `key:` definitions; `measure:` must be declared here or oxplow.*
dimensions:  [...]   # each may carry a one-sentence `description:` (`v_dimension.description`)
collectors:  [...]   # entity collectors (exec / starlark / jaq / read → entities) and
                     # fact collectors (`facts:`; starlark / jaq only; facts must be declared here or oxplow.*)
                     # see semantic-layer.md "Collectors"
ui:                  # what it adds to the core UI (a command's own `ui:` puts it in core menus: see extensions-commands.md "Commands")
  slots:             # pages mounted at other pages' slots; see extensions-slots.md "Slots"
    - { page: change-review, at: oxplow/diff-view.sections }
  decorators: …      # labels from its datasets on core URIs (stable since P10; see extensions-slots.md "Decorators")
  replacements: …    # a page in place of a named core component (see extensions-slots.md "Replacements")
advisories:  [...]   # see extensions-commands.md "Advisories"
datasets:   [...]   # SQL datasets: DatasetDecl entries (`fields:`, optional `relationships: [{ from, to: <dataset>.<field> }]`) + datasets/<name>.sql → v_<namespace>_<name>; `key: [fields]`; `materialize: on_change | { every: 1h } | { incremental: <field> }` stores one (semantic-layer.md "Datasets" vocabulary, "Extension datasets", "Materialized datasets"); the old `datasets:` / `columns:` are unknown keys with a hint
panels: …           # left-nav panels, each a page compact (see "Panels")
commands:  [...]   # Starlark scripts composing core commands, a scope's or its provider's operations (see extensions-commands.md "Commands")
resources: …        # kinds of thing a URI can name (see extensions-events.md "Resources"); each has an optional one-sentence `description:`
event_types: …      # its own namespace's event types (stable; see extensions-events.md "Event types", P8)
implementations: …  # capability implementations it declares (oxplow-foundation declares the defaults); each has an optional one-sentence `description:`
skills: …           # skills and slash commands for the coding agent
providers: [...]    # external providers over the provider protocol — a program, or an MCP server behind oxplow's adapter (providers.md)
effects: …          # scripts reacting to logged events by composing commands (see extensions-events.md "Effects")
custom_components: …   # sandboxed components a page's `component` node or `custom` viz draws (see extensions-components.md "Custom components"); each has an optional one-sentence `description:`
```

Pages aren't listed here: every `pages/*.yaml`, `parts/*.yaml` and
`queries/*.yaml` file in the folder is loaded, its kind by its folder
(extensions-pages.md "Three kinds, by folder"). A page gets into the
launcher through its own `category` (a page with no `category` is not listed). An old-format UI folder (`lenses/`) is a
load error that says to rewrite each file as a page.

To move an old extension over: each file in `lenses/` becomes a page
(a document's sections go into `view.stack`, its queries into named
`loaders:` loaders); fixtures and `intent.examples` name `page:` instead of
`lens:`, with table expectations under `expect.loaders.<loader>`. Then
delete the YAML files in `lenses/`: they are what the loader refuses.
`oxplow extension check/test <name>` says when it loads again.

**What the loader checks** (`Extension.errors` / `Extension.warnings`):
- shape: unknown keys, `manifest` version, `intent` present and
  `intent.origin` a canonical URI; each `fixtures/<name>.yaml` no
  intent example names joins `intent.examples` (`fixtures_are_examples`:
  a fixture file is an example, so a test is one file), and an extension
  with no example after that is a warning;
- page files: each `pages/*.yaml`, `parts/*.yaml` and `queries/*.yaml`
  parses (`pages::schema::parse_page`: unknown keys are errors; a key its
  kind doesn't take is an error; a `replaceable` names a replaceable
  component and is placed once); a slug in two folders is neither; an
  `icon` oxplow draws; the pages that a param's
  `options_from`, a column's `expand` and a form field's `from` name are in
  `pages/`; a page's renderer declarations (`for`, `accepts`, `default`,
  `pinned`) fit the resources it names; a `component` node or `custom`
  viz names a component the extension declares in `custom_components:`.
  Layout problems are **warnings** (`pages::checks::layout_warnings`:
  more than 6 filters, a `choice` of more than 12 options, a description
  over 200 characters, a table of more than 8 columns, a param no loader reads);
- lifecycle: `engine`, when given, is `>=MAJOR.MINOR[.PATCH]`;
- cross-references: a `ui.mounts` page is in `pages/` and its slot name
  has the shape `<namespace>/<page>.<name>`; a slot mount whose host page
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
