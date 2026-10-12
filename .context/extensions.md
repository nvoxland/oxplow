# Extensions

This doc covers how anything that **measures or visualizes** is added to
oxplow: the `extension.yaml` format, lenses, slots, actions and alerts, and
`oxplow-bundled`, the one extension that ships with oxplow.

**Terms.** oxplow's one vocabulary for what it's built from. Code, docs,
skills and UI copy all use these words, with these meanings
([usability.md](./usability.md) "Copy names things as a person knows
them"):

| Term | Means |
|---|---|
| extension | the package: a folder with an `extension.yaml` |
| contribution | one thing an extension declares (a lens, collector, command, effect, provider …) |
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
(`extensions::descriptors::describe`, `Extension.contributions`): a
stable id `<extension>/<kind>/<local>` (`ContributionId`; one grammar for
`local`, dots only for the kinds whose ids are dotted), its owner
(extension, origin and the extension's `version:`),
how it runs (`RunsOn`), its needs and `after`, the **global name** it
takes in a space extensions share (a metric key, a fact collector's id)
and the public forms it already had (`aliases`). The kinds' table
(`contribution::KINDS`: manifest key, title, whether two
extensions may share a name) is the one place those facts live.

At load, `check_ids` turns an id that breaks the grammar into a load
error, and the folder name must be a valid extension name
(`valid_extension_name`; bundled names are core's). Across the catalog,
`check_catalog` (end of `load_extensions_in`, after disables) adds every
enabled extension's descriptors to a `Registry` in catalog order: a global
name a kind doesn't share that another already holds is refused — a load
error of the later extension naming both — and the contribution is taken
out of that extension, so nothing downstream sees two (this is what keeps
the first extension's metric when two declare one key). `version:` is
required for `sharing: shared` (`manifest_v2::valid_version`); a bundled
extension's version is oxplow's own. Every kind is described: collectors
(global: a fact collector's id), effects, commands (global: the bus id),
advisories, skills (global: the name), implementations (global:
`<capability>/<id>`), providers (global: the id, which is also the default
instance's id, its refs' segment and its commands' namespace), measures,
metrics and dimensions (global: the key), models (global: the
`v_<ext>_<name>` view), event types (global: the type string; one
descriptor per type, its highest `v` the contract version), ref kinds
(global: the kind; registered first, most specific namespace first, so
`acme-pr`'s `acme_pr_` wins inside `acme`'s `acme_`), lenses, pages,
panels, slot mounts (`<slot>-<n>`), decorators (`decorator-<n>`),
replacements and custom components (all mounted, no global name).

The id rules each kind used to restate are the domain's: `valid_local`
(effects, advisories, pages, panels, skills, custom components),
`valid_extension_name`, `extension_namespace` (the `-`→`_` rule, used by
command namespaces, model and entity views, entity tables, event types
and the SDK). Rules that stay their own on purpose:
`refuse_shared_namespaces` refuses *both* extensions whose commands map
to one namespace (a namespace is one extension's);
`capability::instance_id_problem` keeps provider ids to snake_case (the
id is also a command namespace); a model can't take a core model's name
or be declared twice in one extension; the event-type "two extensions
make one namespace" check (unreachable with valid names, kept for an
invalid-named folder, which still loads).

**Discovery.** Each generation (boot and every catalog change)
restates the `contribution` table (V47, `oxplow_db::contribution_store`)
from every enabled extension's descriptors, read as `v_contribution`.
The reference's "What it can declare" table is generated from
`contribution::KINDS` (each kind's `summary`) between markers in
`docs/reference/extensions.md`;
`descriptors::catalog_tests::the_reference_lists_every_kind` fails when
they drift (`OXPLOW_BLESS=1` rewrites it). The inventory and plan behind
all this are the foundation roadmap's "Normalize contribution
descriptors".

**Public imports and exports.** The pure validator is `extensions/imports.rs`;
`extensions/public_contracts.rs` normalizes manifest declarations, binds public
model references and withholds contributions before catalog publication.
Exports are explicit opt-in. A stable contribution kind or a legacy alias
does not make a contribution public.

- Targets use canonical `ContributionId` (`<extension>/<kind>/<local>`).
  An import has a consumer-local binding name, a positive integer contract
  version and required/optional status. Binding names are lowercase letters,
  digits, dashes and underscores, starting with a letter and ending with a
  letter or digit. Aliases and internal names never resolve a target.
- Package `version` is release SemVer; `engine` is host compatibility;
  an export's positive integer version is its individual contract. Imports
  require exact equality. A model/event export cannot relabel its native
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
`imports: [{id, contribution, version, optional}]` and
`import_guards: [{contribution, imports: [binding]}]`. Guards name this
extension's models, lenses, collectors, commands, effects, advisories, skills
or providers. A guard's bindings must be declared and nonempty; unknown or
duplicate names are errors. Models and lenses can read a public model with
`import_ref('binding')`; tokenized calls infer a guard automatically. Strings,
comments and qualified function names are untouched. A model's binding becomes
an ordinary `ref('<producer>/<model>')`, retaining lineage and native contract
checks; a lens's becomes its quoted published view. SQL grants still apply.

The loader validates declarations and optional guards before legacy global-name
collision pruning can hide duplicate identities or let a withheld contribution
reserve a global name. Host manifest checks (including engine compatibility)
also block candidates with explicit public contracts and their required
consumers; an invalid candidate never reserves global names. Resolve imports,
withhold guarded contributions, remove mounts that
refer to absent lenses/models/pages/providers, withdraw their exports, and
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
supplies all enabled models and entity stubs; it does not reuse active views or
publish registries. Previously active consumers blocked by the candidate report
errors on that candidate, while unrelated existing failures are not attributed
to it. Model compilation also reports errors from declared transitive consumers.
Disabled packages can be inspected without changing enablement or authority.

Both impact sides resolve against current project dependencies, rather than a
complete historical dependency catalog. The installed side reads its own raw
source even if its active contributions were cleared. Review and installation
reject contract regressions before replacing files; source lookup allows a
contract-blocked package to review a repairing update. Explicit configuration
disabling still applies to update operations. Rejected project folders using
reserved bundled names remain diagnostics only and cannot block the bundled
package's public identity. Runtime generation activation is integrated; import
withdrawal and activation behavior are covered end to end by
`extension_imports_tests.rs` (compatible, mismatched, missing, optional,
cyclic, private/experimental, producer disabled, and an update that would break
a consumer). A contract-blocked extension is cleared to `enabled: false` by the
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
abort the outer audited command transaction. No migration script or external
wait executes inside that transaction. Writes return stamps and omit both
layer inputs from audit; private compositions/proposals follow the state rules.

Reset uses an empty override object at a newer revision, retaining stale-token
rejection. Disable/uninstall retain local rows for explicit recovery/reinstall;
access is refused while the owner is disabled, missing, stale or revoked.
Both scopes live in the local project's database, with no global identity lookup.
Raw secrets do not belong in defaults, schema or overrides. Host-owned secret
slots/references are still pending; this service resolves non-secret settings
only.

## Private state host boundary

`extension_state.rs` supplies `StateHost`, bound by the host to the primary
project, over the caller's already-owned connection. Requests select only
`scope: project|personal` and `area: state|cache`; they cannot select an extension,
project path, SQL, run identity or grants. The host requires a valid contribution,
an explicit `state.read` or `state.write` declaration, allowed live authority,
the current catalog generation/package version and an enabled descriptor. Core's
unrestricted context without a declaration is refused. Imports do not select
another namespace. Revoked, disabled, removed, stale or undeclared owners fail
with fixed diagnostics before payload access.

Native commands may declare `scope: state.read`, `op: snapshot` to read their own
namespace. This read returns null when absent or `{stamp, values}` when present;
it has read access and its result is not copied into command audit. State remains absent from
SQL models. Scripts may declare `needs: [state.read]` and call
`scope("state.read", {"scope": "project", "area": "state"})`; collectors, fact
collectors, effects and policies use a separate owned read for each call, with
no transaction held while their worker waits. Command composers use the routing
connection. The shared `scope` builtin accepts declared callable services
and checks the actual service on every call; declaring state access does not
imply SQL access. Service listings report declared callable services as potential
uses when a script names or aliases `scope`, rather than inferring every dynamic
call argument.

Provider `host/call` serves the same state read using the provider contribution
identity captured from an exact enabled catalog declaration at instance creation,
with the catalog generation unchanged across approved-copy preparation,
not the invoking command's extension or a protocol-supplied owner. Calls require
a registered in-flight invocation and recheck its live consent gate. Candidate
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
Migrations, lifecycle cleanup and host secret services remain unfinished. Raw
secrets do not belong in state payloads.

Current-generation checks use the filesystem catalog, including its stat walk;
they are conservative on any catalog change. This does not pin DB/runtime
registries or keep state access usable under a retained last-good handler after
a rejected candidate. The coherent activation coordinator must adopt the host
boundary with its active-generation identity. Working-path tests cover the
stores, resolution and schema adapters, the secret reference and host, and the
command-bus path end to end (`extension_private_state_tests.rs`: write/read,
owner isolation, disable keeps rows, precedence, value-free refusals, restart
over a database file); adversarial and concurrency coverage is still to do.

> **Status: built (epic tsk275; host finished in tsk278).**
> - **Current:**
>   - loading project extensions and their lenses from
>     `oxplow/extensions/` (see "What works today" below);
>   - running and validating lenses over IPC and MCP;
>   - the `oxplow-extension` agent skill;
>   - the lens page and the launcher's "Lenses" section;
>   - sharing: team via the repo, world via the `oxplow.extension.install` /
>     `oxplow.extension.update` commands and Settings → Extensions;
>   - the core explorer: the Explore Data page (with Save as Lens) and
>     lens tiles on dashboards;
>   - **bundled extensions** (compiled in, read-only, reserved names) and
>     the `effort.review.details` **slot**; the bundled `oxplow-bundled` extension
>     holds the effort review packet;
>   - `exec` **sources** that bring external records in as entities. The
>     mechanics and decisions are in
>     [semantic-layer.md](./semantic-layer.md) → "User and extension
>     sources", and a tested example is in `examples/extensions/github/`.
>   - **slots** (now `effort.review.details`, `vcs.commit.details`,
>     `vcs.status.details`, `work_item.detail.body`, `thread.plan.header`),
>     **advisories**, per-project **disabling**, and the
>     **analytics extraction** (tsk280): every analytics page and
>     widget is now a lens in `oxplow-bundled`, and core works with
>     it disabled (checked headless, 2026-09-27).
>   - lens alerts (tsk316; the `rail` slot they mounted in became panels
>     in P6.G1), and extension-declared
>     measures, metrics and fact collectors (tsk311, gauges until P7.B3;
>     see "Contributing metrics").
> - **Current:** extension-declared dimensions (tsk328).
> - **Current:** lens action buttons (tsk329) and the `settings.section` slot (then `settings`)
>   (tsk330).
> - **Current (P1, 2026-09-28/29):** manifest v2 with `intent`,
>   `sharing` and the stable/experimental split (tsk413), the textual
>   v1→v2 migrator (tsk414, removed with the v1 reader in tsk865), the
>   per-root catalog cache (tsk415, tsk390) and the SDK: `oxplow extension
>   new|check|test` (tsk416; "The SDK").
>
> When a piece ships, move it from "target" to "current" here, in the
> same commit.

## What works today

Code: `crates/oxplow-app/src/extensions.rs` (the loader, lens runs and
validation), `crates/oxplow-rpc/src/commands/extensions.rs` (IPC), and the
lens tools in `crates/oxplow-mcp/src/lib.rs`.

- **Files.**
  - `oxplow/extensions/<name>/extension.yaml` is a **manifest v2**
    (`manifest: 2`, `name` = the folder, `sharing`, `intent`, the
    contribution kinds; see "`extension.yaml`" below).
  - `lenses/<slug>.yaml` contains `title`, `description`, `query`, `viz`
    (`table` | `list` | `number` | `markdown`), `params` (`name`, `label`,
    `default`), `columns` (`key`, `label`, `link: {kind: page | file |
    wiki | effort-diff | …, from}` — a work item links as `page` by its
    ref) and `empty`.
  - Unknown keys are errors, so typos surface instead of being ignored.
- **Ids.** A lens id is `<extension>/<slug>`.
- **Reading.** What the app shows is read from the **main worktree**
  (the daemon's project dir), whichever stream is open: listings, lens
  pages, slots and panels, custom components, prompts, advisories,
  `oxplow.lens.show` / thread answers and `get_open_page`. A stream's `stream_id`
  only scopes the data a lens reads (`:stream_id` / `:thread_id`). See
  "Per stream" below. Reads go through
  `Services.extension_catalog` (`crates/oxplow-app/src/extension_catalog.rs`):
  a per-root cache behind a stat-only fingerprint of `oxplow/extensions/**`
  (every file in the folder — a custom component's bundle too, so its
  file cap keeps that walk small) and `.oxplow/project.yaml`, so a hit costs ~60 µs instead of the ~3 ms
  parse and an edit still shows up on the very next call (see
  [performance.md](./performance.md)). `find_lens`, `run_lens`,
  `validate_extension`, `list_data_entities` and the advisory/metric/source
  readers all take the catalog; `load_extensions` itself is the cache's
  loader and the write paths' direct read. Hidden folders directly under
  `oxplow/extensions/` are excluded from both loading and fingerprinting, so
  installation staging does not identify a new catalog. Hidden files within
  a declared extension remain fingerprinted. `ExtensionCatalog::snapshot`
  returns immutable declarations with their load fingerprint identity. Retaining
  it preserves that identity across cache reloads; `generation(root)` still
  observes current files for candidate staleness checks. The raw `snapshot`
  remains an authoring/candidate view; runtime readers use `runtime_snapshot`
  for the last committed catalog plus immediate disable/removal revocation.
  Neither API pins the filesystem itself.
- **Params.** Values are bound as `:name` (`SemanticLayer::query_sql_named`).
  Supplied values override defaults. An unknown param name is an `Invalid`
  error, so a typo can't silently fall back to a default. A param with no
  default and no value binds NULL.
- **Errors never cascade.** A missing or bad `extension.yaml`, or a folder
  name mismatch, marks that extension's `errors` and skips its lenses. A
  bad lens file is listed in `errors` and skipped; the other lenses still
  load.
- **`validate_extension`** also dry-runs every lens with its defaults. It
  reports SQL errors, and `columns` / `link.from` keys that the query
  doesn't return.
- **Surfaces.**
  - IPC and MCP: `list_extensions`, `get_lens`, `run_lens`,
    `validate_extension`.
  - MCP only: `list_lenses`. MCP's `list_extensions` is a summary
    (each extension's own fields, errors and warnings, and a count of
    each list it contributes); `name` gives one extension's whole
    listing — the full list ran to 124k characters with the bundled
    extension in it.
  - **Over MCP these are the authoring tools** and read the caller's
    **working copy**: an omitted `stream_id` is the caller's own stream
    (its header, else its thread's; the primary only for an anonymous
    caller), so an agent in a worktree can check the extension it just
    wrote before it's merged — the same for `list_lenses`,
    `preview_collector`, `review_extension`, `oxplow.extension.install`,
    `oxplow.extension.update`, `run_lens_action` and `ensure_change`.
    `site_search` is the exception by design: omitted, it searches every
    stream.
  - **Over IPC (the UI)** they show the main worktree's extensions:
    `list_extensions`, `get_lens`, `load_component`,
    `invoke_component_command` and `prompt_catalog` take no stream;
    `run_lens`, `lens_text`, `lens_form`, `submit_lens_form`,
    `run_component_query` and `run_lens_action` take one only for the
    lens's data context. `validate_extension` and `review_extension` are
    authoring calls and check the named stream's working copy.
  - `get_open_page` (MCP) plus `report_open_page` (UI) are current; see
    "Agents: the MCP surface".
    Agents write lens files with their normal Edit tool, under the filing
    guard, taught by the `oxplow-extension` skill.
- **UI.**
  - `lens:<extension>/<slug>` pages (`LensPage.tsx`) render `table`,
    `list`, `number` (through `formatMetricValue`) and `markdown`.
    Linked cells go through `RouteLink`, so plain-click navigates in the
    tab.
  - Params sit in the right rail: Enter or blur applies, Escape reverts.
    A tab id may carry starting values (`lens:<ext>/<slug>?effort_id=12`).
  - A run failure shows the error inline, with a nudge to use "Improve
    with Agent".
  - The page re-runs when a model it read changed (`modelsChanged`), when
    facts landed for a measure its `metric_grid()` read
    (`metricSamplesChanged`), on edits under `oxplow/extensions/` —
    the run's `result.reads` is what it subscribes to (P4.6) — and after a
    reconnect (`onRemoteReconnect`): events sent while the daemon was
    unreachable never arrive, so anything may have changed (tsk1050). Every
    lens host uses `useRerunOnChange` in `src/lens/lensRerun.ts`.
  - "Improve with Agent" inserts `[oxplow lens <id> k=v…]`, with only the
    changed params, through the standard add-to-context path.
  - The Cmd+P launcher re-reads lenses (the main worktree's) every
    time it opens and lists them under their `launcher.category`
    (default **Lenses**; `hidden` ones not at all), merged into the
    static directory by category order (`mergeDirectory`).
  - `alert:` marks when a lens needs attention: `{ min_rows: N }` (the
    run returned at least N rows) or `{ column: c, above: X }` /
    `below: X` (the first row's value), with an optional `label`. The
    loader refuses one with both or neither condition. Every `LensRun`
    (IPC and MCP `run_lens`) carries `alert: {firing, count, value,
    message}`; the message is `label: count`, `N rows`, or `label: value`.
  - **Pages** (P6.G2, target §11.3): `pages: [{ id, title, icon?,
    category, lens }]` — a lens shown full-page at
    `page:ext.<extension>.<id>` (`Extension.pages`, `ExtensionPage`;
    checked at load in `parse_pages`: a kebab-case, unique id, a launcher
    category, a lens that exists), listed in the launcher under its
    category (`components/extensionLauncher.ts`).
  - **Panels** (P6.G1, target §11.3): `panels: [{ id, title, icon?,
    scope: project | stream | thread, body, badge?, open?, collapsed?,
    count? }]` put a lens in the
    left nav (`open`, a `page:<kind>` ref, is the page its header opens —
    a core page it summarizes, as Comments opens the inbox; absent, the
    body lens). `collapsed` is a lens shown compact while the
    panel is collapsed, as its summary (absent, a collapsed panel is just
    its header). `count` is a lens whose row count — a `number` lens's
    value — is the header's count, raising no alert; with a `badge` too,
    the count lens's number shows and the badge still feeds Alerts
    (`panelCount`). (`Extension.panels`, `ExtensionPanel`; checked at load in
    `parse_panels`: a kebab-case id, lenses that exist — body, badge,
    collapsed, count — a badge with an
    `alert`, and a `stream` / `thread` scope's lenses declaring
    `stream_id` / `thread_id`). The nav binds those itself
    (`panelParams`: the stream's and thread's row ids for the stream and
    thread it's shown for, re-run when they change) rather than leaving
    the backend to infer the thread from the selection, and runs every
    panel's lenses from one owner (`useExtensionPanelRuns`), each distinct
    lens once per refresh whatever roles it plays. The body renders compact; the badge's
    alert count shows on the panel, and the **Alerts** page (status-bar bell) lists
    every firing badge. oxplow-bundled's Waiting on You is a panel
    whose badge is its own lens.
  - **Actions are commands** (P6.B1, target §11.4): `actions:` declares
    `{ id, label, command, input?, row? }` — a button above the result, or,
    with `row: true`, an item in each row's menu — every row of every row
    component (table, list, tree, timeline, detail, steps, hunks) is
    focusable and opens it from a right-click or the Menu key / Shift+F10. A command
    that asks shows `CommandConfirm` from `LensResultView` itself, above
    the body, so it asks wherever the row is (an answer in the strip, a
    grid child, a dashboard tile), not only where the toolbar is shown.
    - `input` is the command's input. A string that is exactly
      `{{param.<name>}}` or `{{row.<column>}}` becomes that value, typed
      (a number stays a number); a string containing them has them spliced
      in as text. The values go into the command's input, never into SQL.
      One tokenizer reads them (`oxplow_domain::template` — every
      template's, a command's `ui.input` and `open_after` too), for
      load-time validation and run-time binding alike, so the two can't
      disagree.
      At load, a `{{param.x}}` must name a declared param and `{{row.x}}`
      needs `row: true`; `validate_extension` checks `{{row.x}}` against
      the result's columns; the command name must be well-formed. Any
      other shape is a field error.
    - **A lens grants no power.** `lens_actions::run_lens_action` runs the
      command through the bus as `Actor::Lens { lens_id, on_behalf_of }`
      — `Human` from the UI (`run_lens_action` RPC), the calling agent from
      MCP `run_lens_action` (which never confirms). The bus applies the
      command's `invokers.lens`, and the agent policy through
      `on_behalf_of`, so an agent can't reach a human-only command or
      confirm one through a lens. The audit records `lens:<id>`.
    - A command that asks (`NEEDS_CONFIRMATION`, now kept on the thrown
      `IpcCallError`'s `code`) shows a confirm strip in the toolbar: Run
      (focused, Enter) or Cancel (Escape).
    - **Every lens has Copy** (its text rendering, the `lens_text` RPC)
      **and Add to Agent Context** (`[oxplow lens <id> params…]` into the
      agent's input, never sent) — kit affordances, not declared. A
      compact strip and a grid's children (`toolbar={false}`) show no
      toolbar.
    - Running a collector is the command **`oxplow.collector.sync { owner, id }`**
      (External, `Invokers::ALL`; `collector_runner::CollectorRunner::sync`,
      which the scheduler uses too). It **never approves**: approving is
      `collector_runner::approve_reviewed` behind the UI-only
      `approve_collector` RPC (Settings → Data's Approve & Run approves,
      then runs the command). MCP `run_collector` runs the command as the
      agent.
    - UI: `LensToolbar` and the row menu in `LensResultView.tsx`; logic in
      `lens/lensActions.ts` (`performLensAction`, `copyLens`,
      `addLensToContext`, `rowRecord`).
    - The GitHub example's PR lens has a Sync PRs action
      (`command: oxplow.collector.sync`).
  - **Viz** (`LensResultView.tsx`): `table`, `list`, `number`, `markdown`,
    plus `bar` (`DailyBarChart`), `line` (`components/charts/TrendChart`,
    one chart per `chart.series`), `treemap` (two-level
    `components/charts/squarify`) and `grid` (children run with the
    params they declare), and the structure components (P6.A2): `tree`
    (collapsible nesting by `tree.parent`; orphans are roots, a cycle is
    cut), `timeline` (oldest first, linked through `timeline.ref`),
    `detail` (the first row as label/value pairs), `steps` (a checklist
    by `steps.status`: done, active, failed, else pending) and `hunks`
    (each row's file diffed between `hunks.from` and `hunks.to` in the
    diff viewer, `DiffPane`, one expanded at a time), and **`form`**
    (P6.B2): `form: { command, defaults? }` renders the command's
    `input_schema` as a `SchemaForm`, its fields starting from `defaults`
    (placeholders bound like an action's) under the query's first row —
    a form needs no `query` (RPC `lens_form` returns the spec and those
    values). Submitting (RPC `submit_lens_form`,
    `lens_actions::submit_form`) runs the command as the lens acting for
    the person, the defaults under what they entered; a command that asks
    shows `CommandConfirm`. A form's text rendering names its command
    (an agent runs the command itself). Only a form and a grid (which renders its children, never rows of its own) omit `query`. The loader drops a
    lens missing the column roles its viz needs, naming the block
    (`chart`, `tree`, `timeline`, `steps`, `hunks`), and
    `validate_extension` checks every named column exists in the result
    (`Lens::role_columns`). Pure data shaping lives in `lensModel.ts`
    (`barRows`, `lineSeries`, `treemapItems`, `treeNodes`,
    `timelineEntries`, `stepItems`, `hunkRows`, `childParams`).
- **Sharing.** There are three levels:
  - **Yourself.** An extension works in your worktree as soon as it's
    written.
  - **Your team.** Extensions are committed files under
    `oxplow/extensions/`.
  - **The world.** Publish an extension as a git repo with
    `extension.yaml` at its root. Others run `review_extension(git_url,
    git_ref?)` then the `oxplow.extension.install { git_url, git_ref?,
    reviewed_sha, stream? }` command (`commands/extension_install.rs`,
    P8.A9: `External`, `Confirm::Always` — an agent's run becomes a
    proposal a person approves; Settings → Extensions' install box runs it
    confirmed, the review being the confirmation).
    - **Review first (tsk378).** `review_extension` (or `(name)` for an
      update) clones into `.oxplow/tmp/` and returns `ExtensionReview`:
      the extension as it would load (`errors` = load errors), its commit
      `sha`, and `problems` from a dry run of its lenses/advisories
      (reported, not blocking: a lens over an unsynced source can't run
      yet). Settings shows it as an inline panel spelling out each exec
      source's program, hosts and credentials, derived sources,
      advisories, fact collectors and slots (`reviewModel`); Install/Update
      confirms. `oxplow.extension.install` / `oxplow.extension.update` take the
      `reviewed_sha` and refuse a clone at any other commit, or one with
      load errors.
    - The repo is cloned inside `.oxplow/tmp/`, because workspace
      isolation forbids writing outside the project.
    - It's validated, then copied without `.git` into a hidden staging
      directory under `oxplow/extensions/`, on the destination filesystem.
      `source.yaml` is written there and the copied candidate is revalidated
      against the project catalog before replacing anything. Symlinks are
      skipped, since they could point outside the extension.
    - The origin is recorded in `source.yaml` (`git`, `gitRef`, `sha`).
      The loader surfaces it as `Extension.source`.
    - Installing never overwrites an existing folder, and a name must be
      lowercase letters, digits and single dashes.
    - `oxplow.extension.update { name, reviewed_sha }` re-clones from the recorded source. The old
      folder is replaced only after the staged copy validates, and only for
      git-installed extensions. Candidate check and publication are serialized
      within the process. Publication renames the installed folder into the
      staging directory, then renames the complete candidate into place. A
      failed second rename restores the old folder; if restoration also fails,
      retain the backup and report its path rather than deleting it. Cleanup
      failure after successful publication is a warning, not a failed update.
      The two renames still have a visibility gap and no startup crash recovery;
      they do not coordinate DB models or runtime registry activation.
    - Installing is a write tool on MCP. The skill says to do it only when
      the user asks, and to offer to commit the result.
    - Installing runs nothing. An exec source, a Starlark one that calls
      a model (`ai_*`) and a shared extension's
      advisories run only after a person approves them in Settings →
      Data. Approvals are stored per machine outside the repo, MACed under
      a keychain key, and bound to a hash of that version of the program,
      so a changed script needs approving again (`exec_consent.rs`,
      `collector_runner.rs`; see [architecture.md](./architecture.md) → "A
      repo's config never runs a program without consent").
- **Core explorer (stays in core, deliberately simple).**
  - **Explore Data** (`explore-data` page, `ExploreDataPage.tsx`):
    - Lists every model from `v_model`, with its columns from
      `v_model_column` (SQL, like everything else; live as extensions
      compile).
    - A picked model shows its **lineage** (`ModelLineage.tsx`, from
      `v_model_lineage`): the models it reads (links that open them), the
      tables it reads (plain text), and the models that read it.
    - Picking one runs `SELECT * … LIMIT 50`; the SQL box then accepts
      any read-only query (Cmd/Ctrl+Enter runs it), metrics included —
      `SELECT bucket, MEASURE('<key>') FROM metric_grid('week')` — and
      re-runs when what it read changes.
    - "Show as" switches the viz: `table`, `list`, `number`, `markdown`,
      and the charts `bar`, `line`, `treemap` (P6.F1), whose columns are
      picked from the result (`chartDefaults`, then a select per role;
      a chart keeps its columns while a re-run still returns them).
    - **Chart a metric** seeds the explorer's own metric query
      (`metricTemplate`: `metricSeriesSql` as a line). While the SQL is
      still that template, **Slice By** picks a dimension (`v_dimension`)
      and regenerates it with `metric_grid`'s second argument
      (`sliceTemplate`); edited SQL is free SQL and is never rewritten
      (the line's series picker slices it instead).
    - Pin to Dashboard and Save as Lens keep the chart (the tile's
      `chart` option, the lens's `chart:`).
    - **Raw tables** (a checkbox, in warning color) reads physical tables
      too — the person's debugging switch, IPC `query_sql { raw }` only,
      never an agent's. A raw result carries a banner, and **Save as
      Lens** and **Pin to Dashboard** are disabled with the reason
      (`keepBlockedReason`): a lens or a tile reads only models.
    - **Pin to Dashboard** adds the query as a `query` tile shown with
      the chosen viz (`PinToDashboard.tsx`, shared with the lens page).
    - **Save as Lens** runs `oxplow.lens.keep` with the query as a `spec` (P11,
      tsk943 — on the bus like Keep This, in the stream's worktree; its
      shape checked as `oxplow.lens.show` checks one, its query through the SQL
      gateway as the explorer runs it, so a `metric_grid()` chart is kept,
      tsk987), then opens the new lens — or, kept in a non-main stream's
      worktree (`live: false`), toasts that it shows once the stream is
      merged (`KEPT_IN_STREAM`; Keep This does the same).
      It creates the extension if missing
      (with the same v2 manifest `oxplow extension new` writes —
      `extensions::scaffold_manifest`, so a saved lens starts as a
      checkable extension with an `intent` to fill in), refuses to
      overwrite a lens, and refuses git-installed extensions.
  - **Thread answers** (P6.C1, `commands/lens.rs`,
    `oxplow-db/src/thread_answer_store.rs`). An agent answers with a lens
    rather than pasting rows: MCP **`show_lens { lens | spec, params? }`**
    runs `oxplow.lens.show`, which stores a `thread_answer` (V124, model
    `v_thread_answer`, ref `answer:<id>`) and returns the text rendering.
    - **One lens shape, `LensSpec`**: what a lens file holds, what an
      answer stores and what `oxplow.lens.keep` writes (`Lens::from_spec`,
      `Lens::spec`). `extensions::save_lens(root, ext, slug, &spec,
      &LensOrigin)` refuses a spec with a `spec_problem` (no title, a
      `grid` or `custom`, or a `form`: a form's submit runs its command
      as the person, and an answer an agent chose never does — so no
      answer, and no kept spec, is a form), writes the YAML
      pruned of nulls and empties, and — when the file doesn't then load —
      removes it and reports the loader's errors. Serialize a spec's JSON
      values with `plain_json` (serde_json's `arbitrary_precision` makes
      numbers maps under `serde_yaml`).
    - An answer's query is agent SQL: `oxplow.lens.show` checks it with the same
      read-only authorizer as `query_sql` before storing anything.
      `run_answer` (RPC and MCP) re-runs it.
    - **Keep This** is `oxplow.lens.keep`: the answer becomes a private lens in
      `my-lenses` (params → defaults, `intent.origin` the thread).
      **`oxplow.lens.share`** (a person's only) moves a lens into a shared
      extension, created with `sharing: shared` and `engine`
      (`ManifestScaffold.shared`), and refuses one that doesn't load or
      reads beyond models. The target goes through
      `extensions::writable_extension_dir` first — the one check every
      writer of an extension's files runs (a valid name, not bundled, not
      installed), so a path or reserved name never reaches the
      filesystem — and a refused share removes only the files it wrote
      (never a directory that was there before).
    - **UI** (P6.C2, `components/Answers/`): a terminal thread's
      `AgentPage` shows the **Answers strip** above the terminal — the
      thread's answers from `v_thread_answer` (`threadAnswers.ts`,
      `useThreadAnswers`), newest first, hidden while there are none,
      collapsible (Escape inside it collapses; remembered per thread in
      `oxplow.answers.collapsed`). An ACP thread renders each answer
      inline under the `show_lens` call that made it (`answerOfTool`
      finds `answer:<id>` in the call's result). Each `ThreadAnswer`
      re-runs through `run_answer` with `useRerunOnChange`, and offers
      **Keep This** (an inline name — empty takes it from the title —
      Enter keeps via `oxplow.lens.keep`, Escape cancels) or, once it is a lens
      (kept, or an existing lens shown), a link to it. A **kept** answer
      is one line — its title and lens link — with Show / Hide, so kept
      answers don't crowd the terminal; the strip is capped at 35% of the
      column, and the terminal below it sits in a flex column so it refits
      when the strip grows (tsk1042: a `height: 100%` under a flex item
      doesn't resolve, so the terminal was clipped and never refit). The
      agent tab has no route context, so `AgentPage` takes `onOpenPage`.
    - Explore Data's **Save as Lens** (`oxplow.lens.keep { spec, stream }`) uses
      the lens's title as a new extension's `intent.purpose`, and the
      caller's thread, when it has one, as its `intent.origin`.
  - **Lens tiles.** A lens can be pinned to a dashboard: "Pin to
    Dashboard" on a lens page, or `oxplow.dashboard.add_item { kind: "lens",
    lens_id }`.
    - The tile is kind `lens`, with `lensId` stored in `options_json`.
    - `LensTile` renders it with the shared `LensResultView`, capped at 8
      rows, against the primary stream, since dashboards are
      project-global.
    - The launcher's new **Data** category holds Explore Data, Metrics
      and Dashboards.
- **Sources (collectors).** Declared under `collectors:` in `extension.yaml` (v1: `sources:`) and parsed
  into `Extension.sources`. A bad source is reported in `errors` without
  hiding the extension's lenses. Settings → Data shows each source
  (runtime, schedule, row counts or failure, Approve & Run / Sync Now);
  Settings → Extensions keeps each extension's source credentials.
- **Settings shows what `extension check` finds** (tsk1039). Loading can't
  catch a lens whose SQL fails (it never runs the SQL), so Settings →
  Extensions runs `validate_extension` — the same dry run `oxplow extension
  check` prints — for each enabled project extension, on open and on
  `extensionsChanged`, and lists its errors with the load errors, once
  each. Bundled extensions are checked where they're built.
- **Bundled extensions.** There is one, `oxplow-bundled` (tsk1084): the
  review packet, the analytics lenses and the agent's advisories in one
  extension, because the point is building oxplow on its own extension
  platform, not letting people pick between its features. It can still be
  disabled like any other.
  - Their sources live in the repo at `extensions/<name>/`. They're
    compiled into the binary by `crates/oxplow-app/src/bundled_extensions.rs`
    (`include_str!`). A test fails if a file in the folder isn't listed.
  - The loader reads them through the same `load_one` as project
    extensions, over an `ExtensionFiles` abstraction (`Disk` /
    `Embedded`). Nothing is written to disk.
  - `origin` is `bundled` and their path is `bundled:<name>`.
  - Their names are reserved:
    - a project folder with that name is listed with an error and never
      shadows the bundled one;
    - `oxplow.extension.install` refuses the name;
    - `extensions::save_lens` (the writer behind `oxplow.lens.keep`) refuses to
      write into a bundled extension.
- **Slots (current: see "Slots" below).**
  - `extension.yaml` declares `ui: { slots: [{slot, lens}] }` (v1:
    top-level `slots`). `SLOTS` in
    `extensions.rs` names each slot and the params it **offers**; a
    mounted lens gets the ones it declares and must declare at least one,
    or the mount is an error. Loaded as `Extension.ui.slots`.
  - `src/lens/LensSlots.tsx` renders a slot (each mount marked
    `data-slot="<slot>"`, which the "every enhancement off" smoke test
    looks for): every mounted lens, run with
    the slot params it declares (`slotRuns`), re-run on data events.
    DiffViewPage offers `effort_id` and `change_id`; WorkItemPage `ref`
    (body and sidebar; the item's ref, whichever list); PlanPane
    `thread_id` (the compact `strip` variant, which hides lenses with no
    rows); GitCommitPage and UncommittedChangesPage `change_id`, plus
    UncommittedChangesPage's strip and GitHistoryPage's side column
    `stream_id` (a side column only when something mounts there —
    `useSlotMounted`).
    Numeric ids come from `numericRowId` (`tsk42` → 42).
  - **Latest request wins** (`src/request-guard.ts`, tsk370). LensSlots,
    LensPage and ExploreDataPage `begin()` each fetch and apply its result
    only while it's still the newest; new params (another thread, lens or
    stream) clear the old results first, so thread A's rows never show
    under thread B.
  - `change_id` comes from `src/lens/useChange.ts`: it calls
    `ensure_change` on mount and again whenever `v_change` changes
    (`shouldReensure`): the analysis landed, or the `change.analyze`
    consumer recomputed a working tree or open effort.
- **Change links.** `diff-at` (`from` path, optional `line`, `base` and
  `head` columns holding the change's labels; join `v_change`) opens that
  file's diff between the two sides (commit shas / `HEAD` → git refs,
  `working tree` → disk, `snapshot N` → that snapshot, so an effort's
  change links too). `compare` takes
  a `path:start-end|peer:start-end` value (build it in SQL) plus an
  optional `head` version column and opens the side-by-side
  `duplicate-block` page, which is now the general core compare page.
  Both refs carry their whole spec, so App's `handleOpenPage` and
  in-tab navigation register them directly.
- **Disabling.** `extensions: { disabled: [name] }` in
  `.oxplow/project.yaml` (bundled extensions included). `load_extensions`
  reads the list with `oxplow_config::disabled_extensions` (just that key,
  from the worktree being read) and returns a disabled extension with
  `enabled: false` and no lenses, slots or sources, so every consumer
  ignores it; `find_lens` and `run_lens` say it's disabled. Settings →
  Extensions toggles it through the UI-only `set_extension_enabled`.
- **A page of sections leads with what matters** (tsk1036). Slot sections
  (`LensSlots`) and a `grid`'s children with no rows (no error, not a grid)
  fold into one closing line, "Nothing found: Duplication, Co-change
  Surprises, …", each a link to its lens (`lensModel::foldEmpty`). A
  section's Copy / Add to Agent Context show on hover or focus (the
  `.lens-section` rule in `index.html`). The effort page opens with a core
  verdict strip above the slots (`pages/effortVerdict.ts`, core models
  only: the last test run, diff coverage, unverified claims, decisions to
  confirm), and the task page links to it as **Review**: on each effort's
  activity card, and in the details rail for the newest effort.
- **The review packet** (in `oxplow-bundled`). Its lenses, mounted in
  `effort.review.details`:
  - Decisions Made (`v_decision`, `provenance = 'recorded'`)
  - Decisions Oxplow Noticed (`v_decision`, `provenance = 'inferred'`)
  - Unverified Claims (`v_claim` where `verified = 0`), each row with
    **Mark Verified** (`oxplow.effort.verify_claim`)
  - Decisions Oxplow Noticed's rows carry **Confirm** / **Dismiss**
    (`oxplow.effort.confirm_decision` / `oxplow.effort.dismiss_decision`)
  - What Deviated (its own model `v_oxplow_bundled_deviation`: each
    effort's files against its work item's title and body — a file is in
    the area when the text names it or one of its directories at least
    two levels deep; none when the item names no area. Live, not
    materialized: 0.2 s over all 5.5k effort files of this repo's
    database)
  - Tests Weakened (deleted test functions from `v_change_function`,
    fewer assertions and new skip markers from `v_change_test_file`;
    also mounted in the `vcs.commit.details` and `vcs.status.details` slots)
  - Struggled Here (`v_struggle`)
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
  `oxplow.review.accept { ref, force? }` comments the review on the
  effort's work item then transitions it to `done`, refusing (`{ refuse }`)
  while a claim is unverified or an inferred decision unreviewed unless
  `force` (then it lists them in the comment);
  `oxplow.review.request_changes { ref, note? }` comments a checklist —
  each unverified claim, inferred decision, file outside the area, and
  the note — then transitions to `todo`. A claim, decision or path is
  spliced in through `md_text` (one inert line: an agent's claim can't
  add headings, links or items to the reviewer's comment); the note is
  the reviewer's own markdown. Both read one `input` query
  (`v_effort` + `json_group_array`s over `v_claim`, `v_decision`,
  `v_oxplow_bundled_deviation`), compose `oxplow.work_item.comment` and
  `oxplow.work_item.transition` — as steps, every list's verbs running
  outside the transaction (not undoable; [commands.md](./commands.md) →
  "Composition") — refuse an
  effort without a work item by name, are `confirm: always`, and are a person's or
  a lens's — never an agent's. `questions.yaml` + `README.md` (its skill)
  say what an agent can read of the packet; a bundled extension's
  questions are checked against a running registry by
  `the_bundled_extensions_answer_their_questions` (oxplow-sdk).

  It also has a Waiting on Me lens, reachable from the launcher:
  what your agents are waiting on you for (`v_agent_status` with
  `awaiting_user`: a question or plan they put to you, a permission
  prompt, a final message ending in a question), blocked tasks, and open
  notes. Each empty state says what's *good*, e.g.
  "Every claim for this effort is backed by evidence."

  The slot lenses above need `effort_id` / `change_id`, so they're
  `hidden` from the launcher (opened there they'd show NULL-param empty
  states). The launcher gets stream-level starters instead, under Work:
  **Recent Decisions** and **Unbacked Claims**, scoped by the implicit
  `stream_id`. `launcher_lenses_need_no_slot_params` enforces the rule
  for every bundled lens (tsk374).
- **One skill set.** Every agent runtime writes its skills and commands
  from one `AgentText` (`oxplow_domain::agent::text`): core's
  (`OXPLOW_SKILLS` and `CORE_COMMANDS` in
  `crates/oxplow-agent-text/src/lib.rs`) plus what
  extensions offer now ("Skills"). ACP agents get the same set as an
  index in their system prompt plus the MCP `get_skill` tool (see
  agent-model.md → ACP).

- **P6b (epic tsk592, 2026-10-01).** An extension can also *act* and
  *extend the shell*: `commands:` (Starlark composing core commands, see
  "Commands"), `ui:` — `slots` in one dotted namespace ("Slots"),
  `commands` in core menus ("Commands in core menus"), `decorators`
  (stable since P10) — and `custom_components:` (stable since P11)
  rendered sandboxed by
  `viz: custom` lenses ("Custom components"). Installs, updates and
  provider approvals are reviewed by what they change ("Reviewing by
  effect"). An agent's run that needs a person waits as a proposal
  ([commands.md](./commands.md), "Proposals").
- **P8 (2026-10-02).** The experimental kinds run, in a private
  extension: `event_types:` ("Event types"), `ref_kinds:` ("Ref kinds")
  and `effects:` ("Effects"), through one swappable vocabulary. A change
  is reviewed by its impact — rows, collector outputs, effects'
  reactions — at any revision, on the CLI (`extension check --impact`)
  and in an effort's review ("Reviewing by impact"). Models declare keys and may materialize on a
  clock or incrementally ([semantic-layer.md](./semantic-layer.md)).
  Installs and updates are the `oxplow.extension.install` / `oxplow.extension.update`
  commands.

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
  token, and `/ipc` includes `forward_terminal_input`, so agent-written JS
  in the window could drive the agent. Custom components use the declared,
  scoped bridge instead; arbitrary app-window code remains outside that
  contract.
- **Presentations are policy.** Bundled lenses provide useful starting
  views, including exceptions at an effort boundary. Trends, reports and
  domain-specific dashboards are equally valid extension presentations;
  the semantic data and metrics are reusable independently of a view.

## Where extensions live

- `extensions/<name>/`: bundled with the app, read-only.
- `oxplow/extensions/<name>/`: project extensions, authored by the user or
  their agent and **committed with the repo**. This is how a team shares
  them.

**Why `oxplow/` and not `.oxplow/`:** `.oxplow/` is oxplow's *local
state*. Its `.gitignore` ignores everything except `project.yaml`, and the
fs watcher prunes its subdirectories wholesale, so files there are neither
committed nor snapshotted nor attributed to efforts. Project-authored
oxplow content already lives in `oxplow/` at the repo root (for example
`oxplow/collectors/*.star`). Extensions follow that convention, so they are
ordinary project files: visible in the file tree, diffed, reviewed and
merged like code.

**The main worktree is the project** (decided 2026-10-07). What the app
shows — and the `extensions.disabled` list in `.oxplow/project.yaml` —
comes from the main worktree, the same in every stream. Extensions are
mostly cross-stream concerns, so branching them per stream was the wrong
separation. Writes still go where code goes: an agent in a worktree stream
edits its own worktree's `oxplow/extensions/` (the write guard keeps it
out of the main checkout), and `oxplow.lens.keep` / `oxplow.lens.share` /
`oxplow.extension.install` / `oxplow.extension.update` write to the named stream's
worktree. The app shows that work once it's merged into the main
worktree, like any other code change. Until then the MCP authoring tools
(`validate_extension`, `run_lens`, `get_lens`, …) read the agent's own
working copy so it can check what it wrote — its files, under the main
worktree's config: the daemon's catalog is
`ExtensionCatalog::for_project(main)`, so every load (`load_extensions_in`,
the write paths' `load_fresh`, the SDK check's workspace filter) takes
`extensions.disabled` from the main worktree whatever root it reads.
Standalone (`ExtensionCatalog::new()`, the CLI) a root is its own
project. If per-stream config is ever
needed, it goes in a separate file, not a per-worktree read of
`project.yaml`.

Each extension is enabled or disabled per project in the main worktree's
`.oxplow/project.yaml`. Loading respects the workspace isolation rule
([architecture.md](./architecture.md)). A malformed extension shows an
error for that extension only; it never breaks the others or core.

## `extension.yaml`

Manifest **version 2** (P1.11, tsk413; `crates/oxplow-app/src/extensions/manifest_v2.rs`).
Every contribution is declared as data, unknown keys are errors, and the
loader resolves cross-references with `file:line` messages.

```yaml
manifest: 2
name: oxplow-bundled   # must equal the folder name
namespace: oxplow      # its command ids' namespace (default: the name, `-` → `_`); `oxplow` only for oxplow's own
description: …
sharing: shared          # private (default) | shared — see below
engine: ">=0.7"          # the oxplow it targets; required when shared
intent:                  # required
  purpose: What question it answers, or what job it does
  origin: effort:eff42   # the thread/effort ref that created it, or null
  examples:              # inputs → expected outputs; fixtures for `extension test`
    - { name: …, input: {…}, expect: … }
# kinds
measures:    [...]   # same schema as .oxplow/project.yaml
metrics:     [...]   # `key:` definitions; sourceMeasure must be declared here or oxplow.*
dimensions:  [...]
collectors:  [...]   # entity collectors (exec / starlark / jaq / read → entities) and
                     # fact collectors (`facts:`; starlark / jaq only; facts must be declared here or oxplow.*)
                     # see semantic-layer.md "Collectors"
ui:                  # what it adds to the core UI (a command's own `ui:` puts it in core menus: see "Commands")
  slots:             # lenses mounted into core pages; see "Slots"
    - { slot: effort.review.details, lens: change-review }
  decorators: …      # labels from its models on core refs (stable since P10; see "Decorators")
  replacements: …    # a lens in place of a named core component (see "Replacements")
advisories:  [...]   # see "Advisories"
models:     [...]   # SQL models: ModelDecl entries + models/<name>.sql → v_<ext>_<name>; `key: [cols]`; `materialize: on_change | { every: 1h } | { incremental: <col> }` stores one (semantic-layer.md "Extension models", "Materialized models")
pages: …  panels: …  # see "Panels" and "Pages"
commands:  [...]   # Starlark scripts composing core commands, a scope's or its provider's operations (see "Commands")
ref_kinds: …        # kinds of thing a ref can name (stable since P10; see "Ref kinds")
event_types: …      # its own namespace's event types (stable; see "Event types", P8)
implementations: …  # capability implementations it declares (oxplow-bundled declares the defaults)
skills: …           # skills and slash commands for the coding agent
providers: [...]    # external providers over the provider protocol — a program, or an MCP server behind oxplow's adapter (providers.md)
effects: …          # scripts reacting to logged events by composing commands (see "Effects")
custom_components: …   # sandboxed components for `viz: custom` lenses (see "Custom components")
```

Lenses aren't listed here: every `lenses/*.yaml` file in the folder is
loaded. A lens gets into the launcher through its own `launcher.category`
(or stays out with `hidden: true`).

**Private vs shared.** `sharing: private` is the usual case (this
project, this person, agent-built). `sharing: shared` (committed for a
team, installed from git, bundled — bundled *must* be shared) must
declare `engine` (satisfied by this oxplow) and its own `version`.

**What the loader checks** (`Extension.errors` / `Extension.warnings`):
- shape: unknown keys, `manifest` version, `intent` present and
  `intent.origin` a canonical ref; an empty `intent.examples` is a
  warning;
- lifecycle: sharing rules above; `engine` is `>=MAJOR.MINOR[.PATCH]`;
- cross-references: a `ui.slots` lens exists (and declares a param
  the slot binds); a grid's `children` exist — in this extension or,
  once everything is loaded, in another; a `launcher` target is a
  canonical ref. A metric's `sourceMeasure` or a fact collector's `facts` that
  is neither declared in the extension nor an `oxplow.*` built-in is a
  **warning** (it may come from the project's or another extension's
  `measures:`, which resolves when the catalog is assembled).

**Each entry's error is at its own line.** Every list block — `commands`,
`effects`, `collectors`, `pages`, `panels`, `implementations`, `skills`,
`ref_kinds`, `custom_components`, `event_types.types`,
`ui.decorators`, `ui.replacements` — and the manifest's own nested lists
(`intent.examples`, `intent.prompts`, `ui.slots`) type their entries one
at a time: a broken entry is left out with an error at its own line
(`manifest_v2::item_lines`, the line of each `- ` item, so a shape error
with no name to look the entry up by lands there too), and the rest of
the extension loads. `intent` and `ui.slots` are read raw
(`IntentFile`, `Vec<Value>`) and typed by `intent_of` / `slots_of`,
which `check` and the loader share.

**Schemas.** `extension.yaml` and `lenses/*.yaml` have JSON Schemas
generated from the loader's own serde types (`extensions/schema.rs`:
`ManifestV2` with `#[schemars(with = …)]` naming each kind's on-disk
struct, and `LensFile`), checked in at `docs/reference/schemas/` and
published with the docs. A test keeps them current (`OXPLOW_BLESS=1`
regenerates them), and every bundled manifest and lens must validate.
Scaffolds and kept lenses start with a `# yaml-language-server:
$schema=…` modeline. A schema holds the shape only; cross-references and
which keys go together stay the loader's to check. When you add a kind
or a key, derive `JsonSchema` on its file struct. The user-facing
reference is `docs/reference/extensions.md` and
`docs/reference/provider-protocol.md`.

**Decision (2026-09-28): `advisories` is a stable kind.** The plan sketch
had it experimental, but `oxplow-bundled` — bundled, so shared by
definition — ships on it, and a first-party extension depending on a
kind is exactly the evidence promotion requires (target §10.1, §12).

**Only v2 loads (tsk865).** A manifest without `manifest: 2` is a load
error (`extension.yaml:1`); so is any key v2 doesn't have (`sources:`, a
top-level `slots:`, `gauges:` — `deny_unknown_fields`). There is no
reader or migrator for v1, and no recogniser for an old shape: a slot
name that isn't one is an unknown slot, an action that isn't `{ id,
label, command, input?, row? }` a field error (tsk920).
`Extension.manifest_version` is the
version the file declares.

## The SDK

`crates/oxplow-sdk` (P1.14, tsk416; target §10.5) is the one
implementation behind three doors: the `oxplow extension new|check|test`
CLI, the RPC/MCP `validate_extension`, and `save_lens`'s manifest. Every
door gives an author the same report, so an agent editing from a terminal
and one calling MCP read identical `file:line: what — fix` lines.

- **`scaffold(root, kind, name, origin)`** (`extension new <kind> <name>`)
  writes `oxplow/extensions/<name>/extension.yaml` (v2, `sharing:
  private`, an `intent` whose `origin` is the ref passed with `--origin`,
  one example) and, but for a bare extension, that example's fixture
  `fixtures/basic.yaml` — then the kind's starter (P7.C6):
  - `lens` — `lenses/<name>.yaml`, open tasks in the viewer's stream, with
    a Start row action (`oxplow.work_item.transition`, a `ref` column it selects
    but doesn't show);
  - `collector` — a Starlark collector `items` over `v_work_item` declaring
    entity `item`, a model `open_items` over `ref('item')` and a lens over
    the model; its example runs the collector over fixture rows;
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
    the area) and a Code launcher lens over `metric_findings()` whose
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
  It refuses an existing folder, a bad name and a non-ref origin. **What
  it writes passes `check` with no warnings and `extension test` clean**
  (a provider once a program replaces its stub) —
  `crates/oxplow-sdk/tests/just_works.rs` holds each kind to it, through
  to loading in a real oxplow: the lens's row action names a registered
  command, the collector syncs and its model publishes, the command runs
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
  `catalog.named` (manifest shape, lifecycle, cross-refs, lens shape;
  a Starlark collector's script must parse and define `transform`, an
  error at the collector's line) plus `extensions::validate_extension`'s
  dry run — **always** (P7.C6): on `layer` (the project's database,
  read-only), else on the throwaway oxplow's empty in-memory one, which
  has the extension's models and metrics seeded so a lens may read
  `metric_grid()` / `metric_findings()` (`DryRun::{Project,
  EmptyDatabase}`; the CLI's project layer is `SqlGateway::with_metrics`,
  so those functions resolve there too — a metric not yet seeded in that
  database is `no metric … (see v_metric_spec)` until the app has loaded
  the extension). The dry run first compiles, in a rolled-back read,
  the extension's models beside the other enabled extensions' (P4.9 —
  resolution, lineage, and the contract, so a changed contract at a
  published version fails here before it publishes) over **empty
  stand-ins for every declared entity that hasn't synced**
  (`models::EntityStub`: a typed temp table under the entity's view);
  `models::check_extensions` returns those views (`CheckedModels.views`).
  Every later query of the check — command examples' reads,
  advisories, lenses — reads through them as an **overlay**
  (`SqlGateway::with_overlay`, `SqlQuery::temp_views`: recreated on the
  query's own connection and dropped after), so a fresh collector →
  model → lens checks clean before its first sync and a lens may read
  its own unpublished model. It returns a `CheckReport { ok, errors,
  warnings, dry_run, extension }`; `render_findings` prints it as text
  (`error: <file:line …>` lines then a one-line summary) or JSON.
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
  `{ ref: $any }` for a work list; `react` to a checkpoint expecting
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
  test <name> [--bless] [--json]`, P5.D5; MCP `preview_extension`, which
  runs it for an agent with an empty environment) runs on **a throwaway
  oxplow** (`throwaway::Host`) that is a real one: a temp project with a
  copy of the project's `oxplow/extensions/` and of which it turned off
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
  a host-registered kind no activation replaces): an example's `model: {
  replies }` is what it answers, in turn. No paid call, no provider.
  It runs `check`, then every intent example with a fixture
  (`fixtures/<name>.yaml`: `input`, `expect`, `$any` matching anything):
  a lens (`{ lens, params?, rows? }` — `rows` seeds the extension's
  entities, `collector_runner::publish_declared_rows`), an event (`{ event
  }`: appended to the throwaway's log and delivered; the extension's
  effects, approved there, react for real — `expect: { effects: [{ id,
  state }], commands }`, read from `v_effect_run` and `command.executed`),
  a job (`{ job, intentKey, request, cancel? }`: through the extension's
  commands over `jobs.write/enqueue` and `jobs.read/snapshot`, approved
  there; `cancel: true` cancels it through `jobs.write/cancel` once it
  has written — `expect: { state, result?, text?, toolCalls?, output? }`,
  `output` its rows `[{ kind, … }]` ("Jobs"); a model reply is a string
  or `{ text?, toolCalls?: [{ name, arguments }], chunkDelayMs? }`;
  naming `effects`, `commands` or `pages`
  also approves its effects and delivers the `contribution.job.completed`
  the job logs, asked in a thread: the effects that reacted, the commands
  they ran and which of the named wiki refs now exist), a component (`{ component, queries?,
  invokes? }`: `load_component`, `run_component_query`,
  `invoke_component_command` as its frame would — `expect: { queries:
  [{ rows }], invokes: [{ ok, refused? }] }`), and the dry runs that
  stay dry (`{ effect, event }`, `{ implementation, event }`, `{ command,
  input }`, a derived or fact collector, an event type, a wikilink). For
  the new forms only the keys `expect` names are compared
  (`shaped_like`). Then, per declared provider, the conformance kit
  ([providers.md](./providers.md) "The conformance kit"), the effort
  policy suite for a policy script, `questions.yaml` and incremental
  models. `TestReport { ok, errors, warnings, blessed, ran, generation,
  contributes, unsupported, left }`: `contributes` is the extension's
  descriptors by public id, `unsupported` what only the app shows
  (pages, panels, slots, decorators, replacements, skills); `render`
  prints it like `check`'s. Exit 0 / 1 / 2 as `check`. `check` with no
  running oxplow asks a throwaway (with no environment) for its command
  registry. `the_triage_example_reads_the_tree_the_layer_and_a_model`
  runs the triage example's job, event and component this way and holds
  its `contributes` equal to a real oxplow's over the same project.
- **The CLI** is `apps/desktop/src-tauri/src/extension_cli.rs`, dispatched
  by `main.rs` before Tauri boots exactly like `oxplow hook`. It takes a
  bare name (under `--root` or the cwd) or the extension folder's path
  (the project is read off `…/oxplow/extensions/<name>`). `check` opens
  `<root>/.oxplow/local.sqlite` **read-only** (`Database::open_read_only`:
  no migrations, no writes, refused unless its schema version matches
  this build — the CLI may not be the app's version) so lens SQL is
  dry-run against the real project; without a usable one it warns on
  stderr with the reason and checks everything else. Exit 0 clean, 1 with errors (or an SDK error such as an unknown
  extension), 2 usage.
- **`validate_extension`** (RPC and MCP) returns the `CheckReport`, not
  the bare `Extension`; the extension is inside it.

The `oxplow-extension` skill requires `check` after every edit.

## Answerability

`crates/oxplow-sdk/src/answerability.rs` checks that an agent
reading the right skill can answer the questions an area of oxplow, or
an extension, exists for. A questions file is a list of `{ question, skill, reaches: { sql } |
{ command, input }, shape: { columns } }`:

- **Core's areas**: `crates/oxplow-agent-text/assets/questions/<area>.yaml`
  (`vcs`, `work_items`, `knowledge`, `code_intel`, `extensions` — areas
  of oxplow, not capabilities; `oxplow_agent_text::ANSWERABILITY_QUESTIONS`),
  each naming a shipped skill
  (`oxplow-codebase` — history and code, added for this —
  `oxplow-runtime`, `oxplow-wiki-capture`). The in-tree test
  (`every_answerability_question_reaches_what_its_skill_names`) runs them
  against a throwaway `Services::in_memory` (async, activated as boot does).
- **Per question**: the skill's text names every model the SQL reads
  and the command it runs — as a whole word, in any case
  (`answerability::names`: `v_commit_file` doesn't name `v_commit`); a
  model is a table the query reads (`models_in`: after `FROM`, `JOIN` or
  a comma in a `FROM` list), never a `v_*` word in a string, a comment
  or an alias (tsk570); the SQL runs through the gateway and
  returns exactly `shape.columns`; a command's input validates against
  its schema. A question whose skill doesn't lead there is a finding,
  which is the point: when a capability grows, its questions say which
  skill must learn about it.
- **Live** (`OXPLOW_LIVE_ANSWERABILITY=1`): `answerability::live` asks
  this machine's `decide` model (`AiService::for_this_machine`: the
  global `ai.yaml`, keychain read only), given only the skill text and
  the catalog (every `v_model` view and every command an agent may run),
  what it would reach for first; it must pick the same model or command.
- **An extension's own** `questions.yaml` runs in `oxplow extension test`:
  `skill` is a markdown file in the extension, SQL runs on the test's
  throwaway oxplow, and a command is one on its bus — core's or the
  extension's own `commands:` — or one of its providers' `<id>.<name>`
  (schema from its declarations).

**`oxplow extension test <name>`** (P5.D5, P7.C6; `extension_test.rs`) runs on
**a throwaway oxplow** (`Host`, "The SDK" above: activated as boot
activates, its entities created empty, the fixture model answering every
model call). It never opens the project's database. It runs `check` there
(with that registry, so `commands:` examples dry-run), then — only if `check` is clean — each
`intent.examples[*]` by its fixture `fixtures/<name>.yaml` (the event,
job and component forms are in "The SDK"):

- `input: { lens: <slug>, params? }`, `expect: { columns?, rows: n |
  $any }` — the lens runs; its row count (and columns, when expected)
  must match;
- `input: { collector: <id>, rows?, event? }`, `expect: { entities: { <name>:
  n | $any } }` — a derived collector runs over `rows` (standing in for
  its `input` query, `preview_collector(…, event, rows)`) and `event`
  (the event an `on:` collector reacts to, as the envelope), storing nothing, its
  rows typed against the declaration; a collector that calls a model
  (`ai.compute`) is approved in the throwaway and runs against the fixture
  model (`model: { replies }`, in the form the role expects, e.g. `{ answers:
  { label: { type: choice, choice } } }` for `ai_classify`); an exec or
  provider-reading collector's example is a warning, not run (it needs a
  person's approval);
- `input: { collector: <id>, files: { <path>: <text> } }`, `expect: {
  facts: [...] }` — a **fact** collector (one with `facts:`) runs over
  `files` as its tree (`MetricsService::preview_fact_collector`, its other
  input keys — `report`, `rows`, `event` (made the envelope) — as its
  input), storing nothing;
  each fact is compared on the keys its expected fact names (tsk1047: it
  used to be run as an entity collector and fail);
- `input: { command, input }` — a provider's, run in its session
  (providers.md "The conformance kit");
Every example's `event:` is built by `EventView::fixture` through the
throwaway's vocabulary (core's plus the extensions' declared types,
synced at its start) — "The event envelope" above: the script gets the
envelope the runtime would hand over, and a fixture that couldn't happen
is an error at it (``fixtures/x.yaml:1: example `x`: event: …``). A
review reads each fixture through that version's own declared types
(`Vocabulary::with_declared`), so a version that adds v2 carries its v1
fixtures to it.

- `input: { effect: <id>, event: { type, payload, subject? }, answers? }`,
  `expect: { commands: [names] } | { skip: <part of the reason> } | {
  reacts: false }` (P8.D12) — whether the effect reacts (`on`/`where`),
  then `effects::dry_run`: its script, its scope calls answered
  from `answers` (per scope, in call order) or for real, the
  composed calls checked against the registry; nothing runs. An
  extension command's is `input: { command: <id>, input, answers? }`;
  `{ skip: $any }` matches any skip (`reaction_problem`, shared with the
  policy's);
- `input: { implementation: <id>, event: { type, payload, subject?,
  anchors? }, answers? }`, the same `expect` but `reacts` — an
  **effort policy script** (`implementations:` with `capability:
  effort_policy`): the event envelope, `effects::run_over` with the answers, the
  calls checked against the registry; appending events is an error (a
  policy emits nothing). Naming no policy script of the extension is an
  error naming the id. For each such script `test` also runs the
  effort-policy suite (`policy_suite`): a throwaway host over a copy of
  the extension (`suite_host`, shared with a provider's suite), the
  program approved there (`effort_policy_program`), the script chosen as `activeProviders.effort_policy`, then
  `effort_policy_conformance::suite`; a finding is an error at the
  manifest, `ran` shows `effort_policy suite`;
- `input: { event_type, v?, payload }`, `expect: { valid: true | false,
  upcast?: <payload> }` — the payload against the extension's declared
  schema (the newest version without `v`), and its upcast;
- `input: { wikilink }`, `expect: { ref: <canonical ref> | null }` — what
  `[[…]]` names with the extension's ref kinds beside core's;
- no fixture is a warning; an `input` naming none of these is an error.

Then `questions.yaml`, each provider's conformance kit, and (P8.B5) each
`materialize: { incremental: <column> }` model on its fixture
`fixtures/model-<name>.yaml` — `before:` and `after:`, each `{ <table>:
[{ column: value }] }`: in a rehearsal (rolled back), `before` is written
and the model built whole into a temp table keyed like its `m_<view>`,
`after` is written, the rows past its watermark are appended, and what
it holds is compared with its SELECT's rows now
(`models::incremental_matches_full`). A difference is an error at the
model's line in `extension.yaml` ("misses n row(s) a full refill holds"
— a watermark that doesn't grow as rows arrive drops a row below it
unseen, which nothing at run time can tell); an append that hits the key
passes (the runtime refills then); no fixture is a warning.

## Lenses

A **lens** is a user- or agent-built way of looking at your work: a
query over the semantic layer plus how to show it. (Not "view", which is
taken by the SQL `v_*` views; not "data app".)

A lens file (`LensFile`, `deny_unknown_fields`) takes `title`,
`description`, `query`, `viz`, `params`, `columns`, `empty`, `chart`,
`children`, `launcher`, `hidden`, `actions` and `alert`.

- `params`: `name`, `label`, `default`, `options`. Untyped: a value is
  bound as-is.
- **Choice params.** `options: [{ value, label }]` makes a
  param a choice: a lens page shows a select, and an extension panel
  whose body lens has one shows it as a toggle in the panel's header
  (one small button per option). The default must be one of the options
  (the loader refuses the lens otherwise, `param_problem`), and a run
  supplying another value is refused (`resolve_params`). A panel's pick
  binds its body lens only — the other lenses don't declare it — and is
  the viewer's, remembered in the browser (`oxplow.panelChoices.v1`,
  `PanelRunsProvider`); a pick the lens no longer offers falls back to
  the default (`panelChoices`).
- **Id params are pickers (tsk1043).** On a lens page, a param named
  `ref` (a work item, over `v_work_item`), `effort_id`, `thread_id` or
  `stream_id` — oxplow's own ids,
  the names it fills from context or a slot binds — is a picker over the
  matching model, by title (`lens/lensParams.ts`); any other param is
  typed. The details rail's header wraps, so its actions sit below its
  title when they don't fit beside it.
- **Implicit params (tsk375).** A param named `stream_id` or `thread_id`
  is bound to the viewer's context (`extensions::LensContext`, numeric ids)
  unless the caller supplies it; precedence is supplied → context →
  `default`. `lens_context(svc, stream?, thread?)` resolves it: the given
  stream (else the thread's, else the primary) and the given thread (else
  the stream's selected-or-active one). IPC `run_lens` / `run_lens_action`
  resolve it from their `stream_id`, so the UI needs no plumbing; MCP
  `run_lens` / `run_lens_action` also take the agent's `thread_id`;
  `get_open_page` uses the viewer's thread. Validation dry-runs use no
  context (defaults only).
- `query`: SQL over `v_*` and the extension's entities, with `:param`
  binding.
- `viz`: `table`, `list`, `number`, `markdown`, `bar`, `line`, `treemap`,
  `grid` (child lenses, `children`), `tree`, `timeline`, `detail`,
  `steps`, `hunks`, `form` (`form: { command, defaults? }`; no `query`
  needed) or `custom` (`custom: {
  component, props? }`, one of the extension's `custom_components`; see
  "Custom components"). Each component names the columns it draws from in
  its own block: `chart` (`x`, `y`, `series`, `label`, `size`, `group`),
  `tree` (`id`, `parent`, `label`), `timeline` (`at`, `label`, `ref`),
  `steps` (`label`, `status`), `hunks` (`path`, `from`, `to` — two
  revisions, `working` / `snap:<id>` / `git:<rev>`).
- `columns`: `key`, `label`, `link: {kind, from, line?, base?, head?}`
  to a core page, so rows are page-graph links (kinds: `task`, `file`,
  `wiki`, `effort-diff`, `commit`, `metric`, `page`, `diff-at`,
  `compare`), and `unit` — another column of the row holding this one's
  unit, so a number shows as a metric value (`16.4 s`, `42.5%`; tsk1038).
  A timestamp cell shows in local time, its full time on hover.
- `columns` also take `icon` and `tone`: each names another
  column of the row. `icon`'s value names an icon drawn before the cell
  from a fixed vocabulary (`lens/lensIcons.tsx`): the work glyphs
  `ready` / `todo` ☐, `in_progress` ◐, `blocked` ⚠, `done` ✓,
  `canceled` ✗, `archived` ▣ (each with its own tone); the kinds `epic`,
  `task`, `wiki`, `file`, `folder`, `commit`, `diff`, `lens`, `metric`,
  `dashboard`, `comment` (their pages' icons); and the ref-kind icon
  names (`bug`, `git-pull-request`, … — `REF_KIND_ICONS`); past those,
  any page kind (`git-dashboard`, `alerts`, `work_item`, …) draws its
  page's icon, so a row of pages — a bookmark, a visit — can
  pass its `page_kind`. `tone`'s value
  is `accent`, `success`, `warning`, `danger` or `muted`, drawn with theme
  variables (`--accent`, `--status-done`, `--status-waiting`,
  `--severity-critical`, `--text-muted`); it colours the cell and wins
  over an icon's own tone. Values come from the query, so the loader
  can't check them: an unknown name draws nothing, an unknown tone is
  plain.
- **Row styling**, each naming a result column:
  - `group: { by, link? }` (`list` / `table`): the rows under a heading
    per distinct `by` value, in the order each first appears — a group
    with no rows doesn't appear. The heading looks like the rail's
    sub-headings (also compact, in a panel) and links through `link`
    (any column link kind; `from` defaults to `by`), read from the
    group's first row. A table puts the heading in a full-width row.
  - `emphasis` (`list` / `table` / `tree`): a truthy value (true, a
    non-zero number, text other than empty / `0` / `false`) highlights
    the row (`--accent-soft-bg`, medium weight).
  - `depth` (`list` / `table`): a whole number (0–8) indents the row, for
    children under a parent in a flat list.
  - `group.by`, `emphasis` and `depth` are never shown as cells. The
    loader refuses them on other vizes; `validate_extension` checks the
    columns they name (and `icon` / `tone`) are in the result
    (`Lens::role_columns`); at render a missing one is ignored.
  - Copy (`lens_text`) renders a grouped lens as a section per group.
  - Pure shaping: `rowGroups`, `rowEmphasized`, `rowDepth` in
    `lensModel.ts`.
- **Rows drag into the agent's context**: a `list`, `table` or
  `tree` row whose first linked column resolves (`rowRef`) is draggable
  with the agent-context drag (`CONTEXT_REF_MIME`, `setContextRefDrag`) —
  a working-tree file or wiki page as its `@` mention, any other ref as
  `[oxplow ref …]` (`rowContextRef`). The agent terminal highlights and
  accepts it as for the core rows; the keyboard path is the row menu's
  Ask About This. A `tree`'s labels draw through their declared column
  (`columns` entry for `tree.label`), so its `link`, `icon` and `tone`
  apply.
- `actions`: commands the lens offers, `{ id, label, command, input?,
  row?, group? }`, with `{{param.x}}` / `{{row.x}}` placeholders; run as the lens
  for whoever pressed, so they grant no power. `group: <value>` puts the
  action in that group's heading (matched against the group's value as
  text) instead of the toolbar — also in a compact panel, where the
  toolbar is hidden. It needs the lens to have `group`, isn't a row
  action, and binds only `{{param.x}}`; a group with no rows shows no
  heading, so neither its action.
- `alert`: a row-count or threshold condition that shows a panel badge
  (nudging the agent is what advisories are for).

> **Not built.** From the original design: a column `format`; a
> `decision` link kind; `followup-comment` and `open-diff` actions.
> (`hunks` and `steps` are built, above; `copy-review-prompt` became
> `copy` on the Review Prompt lens.)

Lenses render through one core `LensPage` / `LensSlots` in oxplow's design
system, as a `lens:<ext>/<slug>` page kind. Bookmarks, backlinks, the
launcher and sibling navigation work unchanged. Every lens has an "Improve
with Agent" action that pastes `[oxplow lens <ext>/<slug>]` and its params
into the agent's context through the existing add-to-context path; oxplow
never types into the agent.

## Slots

Slots are the **only** way an extension reaches a core page. Core pages
declare them (`SLOTS` in `extensions.rs`) and render whatever is mounted,
in declaration order (there is no `order` field). With nothing mounted,
the page is plain. **Slot names are one dotted namespace,
`<capability>.<page>.<region>`** (P6b.C1): a mount naming anything else
is an unknown slot (listing the known ones), and a top-level
`slot_mounts:` / `decorators:` / `replacements:` is an unknown field —
they live under `ui:`.

| Slot | Core page | Params |
|---|---|---|
| `effort.review.details` | diff-view for an effort | `effort_id`, `change_id` |
| `work_item.detail.body` | WorkItemPage, below the body | `ref` |
| `work_item.detail.sidebar` | WorkItemPage, in the side rail | `ref` |
| `thread.plan.header` | PlanPane (compact strip) | `thread_id` |
| `vcs.commit.details` | GitCommitPage | `change_id` |
| `vcs.status.header` | UncommittedChangesPage, a strip above everything | `stream_id` |
| `vcs.status.details` | UncommittedChangesPage | `change_id` |
| `vcs.history.sidebar` | GitHistoryPage, a side column shown only when something mounts there (`useSlotMounted`) | `stream_id` |
| `diff.file.header` | a file diff (`DiffPane`), a strip under its header (P9.A2; `DiffFileHeaderSlot`) | `path`, `left_revision`, `right_revision` (each a revision in its wire form: `working`, `snap:<id>`, `git:<sha>`), `stream_id` |
| `settings.section` | Settings: a section per mounting extension, titled with its name, before AI (tsk330; `SettingsSlotSections`, `slotRuns(…, extension)`) | none |

The launcher isn't a slot: a lens lists itself with `launcher.category`.

## Replacements

`ui.replacements` (P9.A1, stable since oxplow-bundled's history graph;
`extensions/replacements.rs`) puts a **lens in place of a named core
sub-component** — never a whole page:

```yaml
ui:
  replacements:
    - { target: vcs.history.graph, lens: history-graph, version: 1 }
```

The targets are one table, `oxplow_domain::replaceable::REPLACEABLE`:
each a `target`, the `capability` it belongs to, whether it is **bound to
a provider**, its contract's **version**, and its **props contract** —
the params the replacement lens is given instead of any host state. The
table is published as `v_replaceable` (restated with the vocabulary,
`replaceable_store`) and generated into the reference
(`the_reference_lists_every_replaceable_view`), so an author reads the
contracts where everything else is. A declaration's optional `version:`
must be the contract's: a changed contract refuses it rather than handing
it props it doesn't know.

| Target | Core component | Capability | Bound | Props |
|---|---|---|---|---|
| `work_item.board` | the Board's columns of cards (`WorkBoard`, `BoardPage`) | `work_items` | no | `scope` (`thread` / `backlog` / `all`), `thread_id` (the thread when `scope` is `thread`, else null) |
| `work_item.list` | the Tasks page's list (`TasksList`, `TasksPage`): the scope picker, detail pane and backlog drawer stay oxplow's; the lens's `stream_id` is the scope's stream | `work_items` | no | `scope` (`thread` / `stream` / `all`), `thread_id` (the thread when `scope` is `thread`, else null) |
| `work_item.detail` | a work item's editor, its title and body (`TaskDetail`, `WorkItemPage`): the rail (state pill, Move To, fields, Delete), Comment / Link, slots and activity stay oxplow's | `work_items` | no | `ref` (the item) |
| `work_item.detail.state` | a work item's State (`WorkItemPage`'s rail; P10) — never its Move To, which stays oxplow's canonical states, so a replacement can't strand an item (tsk919) | `work_items` | yes | `ref` (the item) |
| `vcs.history.graph` | Git History's commits (`HistoryPanel`): oxplow's own is a plain list (`CommitList`) | `vcs` | no | `head_sha` (the stream's head), `reveal_sha` (a commit to scroll to and mark, or null), `limit` |

**Bound or not.** A bound target shows one provider's own thing — the
state control lists that tracker's states — so only the extension of the
provider whose item it shows replaces it (below). Every other target is a
**view**: the Board, the list and the editor show the capability as a
whole, reading `v_work_item` and changing items through the interface
commands (`oxplow.work_item.update`, `transition`, `reorder`, `move`), so
they work with whichever list is active and any extension may bring one —
a view needs no provider. Choosing a presentation is separate from
installing a work list.

**Whose shows** is the person's: `replacements: { <target>: <extension> |
oxplow }`, one person-only project key validated against the table. An
extension chosen shows (an unbound target only); `oxplow` keeps oxplow's
own (any target); unlisted is the default — the only extension replacing
it, several and no choice keeping oxplow's own, and for a bound target
its provider's extension's. Settings → Integrations lists each replaced
target with that choice (a `<select>`: Default, oxplow's own, each
extension), with or without provider instances.

Every target has a label a person reads — its `label` in the same table
("board", "state control", "history graph"), carried on each
`UiReplacement` so the desktop names it from there (tsk935; no second
list to keep in step). A state control replacement lists the tracker's
own states, each with **Move Here** (`oxplow.work_item.transition` with
the `native_state`); oxplow's Move To stays beside it: the canonical
states every provider takes.

**At load** (`parse_replacements`, errors at the entry's line): the
target is in the table; the lens is one of the extension's and declares
**every** prop (a slot's lens declares at least one; a replacement takes
the whole contract — a Board that ignores `scope` isn't the Board); for a
bound target, the extension brings a provider of the target's capability
(`providers:`); a target is replaced once per extension. The viewer's
stream isn't a prop: a lens that declares `stream_id` gets it, as
everywhere. Valid ones are `Extension.ui.replacements` (`UiReplacement {
id: <ext>/<target>, extension, target, capability, boundToProvider,
lensId, label }`).

**At render** (`lens/useReplacement.ts`, `components/Replaceable.tsx`).
A core page wraps the component: `<Replaceable target props streamId
fallback={<WorkBoard …/>}>` (`BoardPage`). `useReplacement` reads the
enabled extensions' replacements of the target and, only when one
exists, what decides it:
- **bound**: the host passes the provider of the thing it shows
  (`<Replaceable provider={item.provider}>` on `work_item.detail.state`):
  the **item's own provider** decides, whichever is active, so another
  provider's item never gets a lens that sends this one's states
  (tsk918). Which extension brings it is read from
  `v_capability_provider` here, not at load, so installing or switching
  one needs no reload;
- **unbound**: the person's choice (`replacements`), or the only
  candidate; no provider is read;
- no candidate, a provider no extension brings (oxplow's own), several
  unbound candidates and no choice, or `oxplow` chosen → `fallback`,
  untouched;
- chosen → its lens runs with the props it declares (`childParams`) and
  renders through `LensResultView` (the one render path: kit, or `viz:
  custom` with its sandbox and "custom" badge) under a **"replaced by
  <extension>"** badge (`replacement-<target>`, `replacement-badge`),
  re-running like any lens. The page's own chrome (the Board's scope
  picker) stays oxplow's;
- it can't load — the lens run fails, or its custom component doesn't
  start (`CustomComponentViz.onFailure` → `LensResultView.onCustomFailure`
  → `useReplacement`'s `fail`, which makes the replacement `failed`) →
  **the core component**, under a line saying whose it was and why
  ("beads's board couldn't load: …. Showing oxplow's.",
  `replacement-fallback`) — never the lens's table, and **outside** the
  replacement's frame: no badge, none of the lens's toolbar (tsk855);
- while any of that isn't known yet, nothing renders, so the wrong
  component never flashes. Reads of who is active overlap (one per
  change); the newest one asked decides, however they resolve
  (`useReplacement`'s sequence, as its lens run has, tsk857).
Showing one records `usage { kind: "replacement", key: <ext>/<target> }`
(the evidence a kind needs to be promoted) — a kit lens when it ran, a
custom one when its component says `ready` (`onCustomReady` → `shown`),
so one that never loads isn't counted. A replacement is a lens, so
its text rendering is the lens's; agents read work items from
`v_work_item` either way. The capability smoke test renders the Board
with every enhancement off (no `replacement-*`), and an extension's view
replacing it with no provider of its own; a work item's editor replaced
with the rail still oxplow's; a work item's state control, with two
extensions replacing it, shows the item's own provider's, not the active
one's; Git History shows the plain list, and oxplow-bundled's graph when
it replaces it.

**The first real one: the history graph.** oxplow-bundled's
`history-graph` component (`components/history-graph/`, classic scripts
on the client library and the kit) draws the stream's commits as `git log
--graph` does — a lane per line of descent, a dot per commit — from its
hidden lens `history-graph` (a recursive walk of `v_commit` from
`:head_sha`, with local branch and tag labels from `v_branch` / `v_tag`).
Its text, author and branch filters are its own and only dim or hide rows,
so typing never re-runs the lens; the host re-runs it when a commit lands
or a ref moves (its reads). A row opens `commit:<sha>`; a Cmd/Ctrl-click
opens it in a new tab (`navigate(ref, { newTab: true })`, bridge protocol
3). The commit page's prev/next siblings, which oxplow's list passes, a
component can't pass. It declares no commands, so it isn't a program to
approve. Core keeps no lane code: `CommitList` (the fallback, and the Git
dashboard's recent commits with their stats) is a plain list.
`lens/historyGraph.test.ts` runs the bundle's own script against its page
— lanes, merge lanes, child-before-parent order, filters, reveal, Cmd-click.

## Reviewing by impact

An extension is reviewed by its **impact** — what installing or updating
it would change — not only by what it declares (`extension_impact.rs`).
`ImpactReport` holds, each with a `Change` (`added`, `removed`,
`changed`, `unchanged`):

- `lenses` — each lens's rendered text before and after (`LensImpact`);
- `models` — each view, the parts that differ (`changed`: `query`,
  `columns`, `description`, `tests`, `version`, `deprecated`), its
  columns before and after, the first contract difference
  (`contract_change`; a SQL-only change keeps the contract) and its
  `downstream` readers (`models_diff`); the review names the parts
  ("its description and tests changed") when the contract held;
- `collectors` — each collector's `Grants` (entry, runtime, hosts,
  credentials, env) before and after and the views it fills
  (`collectors_diff`);
- `providers` — grants, each declared command by name (`CommandChange`)
  and the capability's features before and after, plus where spec and
  declarations first differ (`first_difference`, so a change no grant,
  command or feature shows still reads as something) (`providers_diff`,
  over each spec and its checked-in declarations);
- `effects` — each effect contribution's trigger and reactions
  (`EffectImpact`, below).

`extension_impact::json_difference` is the one "where do two JSON
values first differ" walk (the provider host's `first_difference` uses
it too). A provider's changes are worded once, in Rust
(`extension_impact::provider_phrases`): the review's `summary`
prefixes each phrase with `Provider <id>:`, and `approval_lines` — sent
as `ProviderImpact.lines` — capitalizes them for the Data section's
approval row.

The report is built from the two loaded versions: **it never runs a
program, an exec collector or an external provider** — consent forbids running a
version nobody approved. What it does run is read-only or sandboxed:
lens and model queries on each version's overlay, and derived
collectors' and effects' scripts (below).

`extension_impact::impact(layer, before, after)` builds it:
each version carries its lenses **already run once**
(`extensions::run_lenses` → `LensRuns`: default params, no viewer
context) — for the candidate, the very runs `check_extension` checked,
so a review runs each lens once — and each is rendered from its run
(`lens_text::render`, a grid's children rendering empty) against **its
own version's models** (each side's overlay — so a lens over a
model the candidate changes renders against the changed SQL). A lens
that fails the same way on both sides is `unchanged`. Each
model's `downstream` is its direct readers from `v_model_lineage`
(`downstream_of`) other than this extension's own views, by exact name;
a provider's declarations are read from each version's files.
`review_extension` fills `ExtensionReview.impact`, with the installed
version as `before` when it replaces one (`review_update`), else none
(everything is `added`); a candidate that doesn't load gets **no**
report (`impact: None` — its errors say why, and it can't be
installed). Settings → Extensions shows `ImpactReport.lines` —
`extension_impact::summary`, the one wording the install
review, `extension check --impact` and an effort's review share:
collectors' and providers' grants first — "now reaches x (was y)", a
provider command added (destructive) — then models with their readers,
lenses) and each changed lens's text before and after,
side by side (`LensDiff`; no line diff yet).

**An extension at any revision.** `extension_at(trees, ws, rev,
name)` loads project extension `name` as revision `rev` of the
workspace holds it — a commit (`git:HEAD`), a snapshot, the working
tree — through `Trees::corpus` into an in-memory `Tree` (the
`ExtensionFiles` the loader reads). A custom component's bundle is
looked up three ways (`BundleLook`): on disk it's `Found` or `Absent` (an
error); in a revision's tree it's checked from the tree's own paths when
they're there, and `Unknown` — taken as declared, not an error — when
they aren't, since a built bundle usually isn't committed. At `git:HEAD` of a clean worktree it equals the
disk load; `None` when that revision has no `extension.yaml`. It's what
lets a review compare two revisions of an extension, neither of which
need be on disk.

**Each side on its own overlay.** A check is `prepare` →
`Prepared { lenses, overlay }`: the extension's models (with the other
enabled extensions') compile to temp views, published nowhere, and its
lenses, advisories and commands read through that overlay. A review
prepares **both** sides — the candidate and the installed version — so a
lens over a model whose SQL changed renders against each version's own
SQL, whatever the database has published (a disabled extension, a model
that failed, another worktree's copy).

**A model's rows.** Each changed, added or removed model's
`ModelImpact.rows` is a `RowDiff { before, after, keyed?, note? }`: each
side's rows read through that side's own overlay (up to 10 000 — the read gateway's own cap, `MAX_ROW_LIMIT`;
`ROW_DIFF_LIMIT`). When both versions declare the same non-empty `key`
it's a merge-join by key — `KeyedDiff { key, added, removed,
changed, samples }`, up to 20 sample rows in key order, each with its key
and its row before and after (key order is SQLite's: numbers by value,
then text); otherwise, or past the limit, or when a side's key has a
NULL part or repeats (one row would stand for several — the key test
runs only at publish), counts with a note saying why (or that a
side's query failed). An unchanged model has
none. The rows are the review's only reads of model data; nothing is
written.

**A collector's outputs.** Each derived collector (Starlark,
jaq) whose script or declaration changed is dry-run on the same inputs
in both versions — each version's intent-example fixtures that name it
(`fixtures/<example>.yaml`, `input: { collector, rows }`), the latest
five events its `on:` trigger matches, else its `input:` query once — by
`collector_runner::dry_run_collector`: each version's own script text,
its `input:` read through that version's own overlay,
storing nothing, as a pure run — a script that names an `ai_*` builtin
is refused before it runs (a review never spends or sends) — under the command
scripts' `COMMAND_SCRIPT_BUDGET` (5 s, not a live collector's 120 s:
someone waits on a review). `CollectorImpact.outputs`
holds each input's `Ran { counts, rows (20 per entity), error? }` before
and after. An exec or read collector is never run — approved or not —
and says so in `not_run`. A collector whose entry's text changed though
its declaration didn't is `changed` with `script_changed`, and says "its
script changed" even when there's nothing to run it on.

**An effect's reactions.** `ImpactReport.effects` lists each
effect by id (`EffectImpact`) with its trigger before and after (`EffectTrigger { on,
filter, needs }`; a script-only change is `changed` too) and, when it
changed, `outputs`: each input — both versions' fixtures that name it
(`input: { effect, event, answers? }`) and the latest five events of its
`on` types — composed by each version that reacts to it
(`effects::dry_run` through that side's overlay, nothing runs):
`Composes { commands, skip?, error? }`. The lines read "Effect x: added — on t where k = v", "Effect x: on …
→ on …", "Effect x on fixture basic: runs [a] → skips (why)".

**A bounded review.** Script dry runs — collectors' and
effects' — share one deadline, `REVIEW_DEADLINE` (60 s, from the start
of `impact`; `impact_within` takes another): past it, no further input
runs, and each collector or effect that didn't get through its inputs
is in `ImpactReport.out_of_time` (`collector <id>`, `effect <id>`), with
the line "Out of time: collector a and effect b — the review stops
running scripts after 60s". Only the later side is *checked*
(`prepare`: commands and their examples, components, advisories, lens
shapes); the earlier side is only *read* (`read_side`: its models'
overlay and its lenses rendered on it) — what it was, not whether it
was right.

**`oxplow extension check <name> --impact [--against <rev>]`.**
The same review on the CLI: `oxplow_sdk::check(…, against)` — the check
and its impact with one throwaway oxplow's command registry — loads the
extension at git `HEAD` (or `--against`) — `extension_tree_at` through a
`Trees` over the VCS alone — as `before` and the working tree as `after`,
and runs `extensions::impact_between` (the install review's path too):
each side on its own overlay over the project's database read
read-only (else an empty one), so it **writes nothing** — the database
file is byte-identical after. Text output adds `impact against <rev>:`
and the report's lines; `--json` adds `impact` (the `ImpactReport`) and
`against` to the check's JSON.

**On an effort's review.** DiffViewPage shows an "Extension
Changes" section when the change's files include `oxplow/extensions/**`
(`changedExtensions`): RPC `extension_impact_between { streamId, start,
end }` → `extensions::extension_changes_between` names each extension
whose files differ between the two revisions (`Trees::diff`), loads both
versions as those revisions hold them (`extension_tree_at`) and reviews
them with `impact_between` — an `ExtensionChange { name, change,
impact?, errors }` each (a removed one has no report). The section
renders each with `ImpactReportView` (the server's lines, then each
changed lens before and after), the component the install review uses
too. While the RPC runs it says "Reviewing acme…"; when it fails, "Could
not review acme: <why>" — never a spinner that doesn't end.

## Commands in core menus (a command's `ui.about`)

A command an extension declares meets a person through its own `ui`
([commands.md](./commands.md) "Offering a command to a person"). One
**about a ref's kind** (`ui: { label, group, about: work_item, input: {
ref: "{{ref}}" } }`) is offered on that ref: its page's nav-bar
**Commands** menu (`RefCommandsMenu`, beside Ask) and a row's right-click
menu for the row's ref (lens rows, by the first ref the row links to —
`rowRef` — and Board cards), grouped by `ui.group`. Its input's strings
may be exactly `{{ref}}` / `{{ref.id}}` (or `{{stream}}` / `{{thread}}`),
and without an `input` it gets `{ ref: "{{ref}}" }`. The desktop builds
them from the person's command listing (`refOffers`,
`components/refCommands.ts`) and runs each as search does (App publishes
its offer host, `setRefOfferHost`). To put another command on a ref's
menu — oxplow's transition to done, say — an extension declares its own
command over the same operation (`scope:` / `provider:` + `op:`,
with its `ui.input`) or a script composing it. The old `ui.commands`
block is gone.

**The desktop reads the extensions from one store**
(`extensionsStore.ts`, `useExtensions()`): one `listExtensions` (the
main worktree's, the same in every stream) and one event subscription
however many readers are mounted
(slots, `useSlotMounted`, `SettingsSlotSections`,
decorators, rail panels, extension pages, the launcher), reloaded on
`extensionsChanged`; the listing lives while something reads it, so a
later mount loads afresh. Only Settings → Extensions lists them itself — it
reloads after its own installs and updates.

## Custom components

`custom_components:` (P6b.D1; **stable since P11**;
`extensions/custom_components.rs`) are web bundles a `viz: custom` lens
renders in a sandboxed frame. **The sandbox bounds what it reaches**:
the frame has an opaque origin (the iframe's `sandbox="allow-scripts"`
*and* the daemon's `sandbox allow-scripts` CSP directive), no daemon
token and no way to send data — no fetch/XHR/WebSocket (`connect-src
'none'`), no forms, no storage. The one way out is navigating itself:
the page's `frame-src` (`http://127.0.0.1:*`, `http://localhost:*` —
each loopback name the daemon serves a bundle to, tsk1003; not `[::1]`,
which no CSP host source can name, tsk1048) bounds that to this machine —
Tauri's CSP in the app, and a one-directive meta CSP in `index.html`
for a plain browser, where whatever serves `dist/` sends no header
(`lens/frameBound.test.ts` keeps them one value; only `frame-src`,
since a fuller policy would bound what the page connects to, and a
daemon over a tunnel isn't on loopback) — and the host ends the
component on its second `load`. So it reaches only the lenses it may
query (`assets`) and the commands it may invoke (`commands`).

**A component that acts is a program a person approves** (P11,
tsk960). `invoke` runs a command with the viewer's rights, so a
component that declares `commands` is listed on Settings → Data →
Programs (`ProgramKind::Component`, key `component:<ext>/<id>`,
`exec_consent::component_program`): the approval covers every file of
its bundle and the commands it may run, and either changing asks
again. Unapproved, it still renders and queries; its `invoke` is
`Denied` with the approval message, and the host shows that reason
under the frame (`custom-component-refused`, `componentRefusal`) — the
component's own code may not. A component that declares no commands
only shows and reads, and isn't a program.

**What runs is what was approved** (tsk984). Before it shows a frame the
host loads the component's bundle (`load_component`, from the main
worktree, as the lens itself is: the RPCs read the project's extensions
whatever the stream) into `Services::component_bundles`
(`component_bundles.rs`): every file of the folder, read once, keyed by
its **version** — the component's approval hash over exactly those files
and its commands (`ProjectProgram::component_hash`, the same function
Programs lists the version with). The daemon serves the frame from that
snapshot alone (`/components/v/<version>/…`), and the frame's `invoke`
names the version it loaded: it runs only when that version is
approved and declares the command. So a bundle edited on disk after the
frame loaded changes nothing that runs, and one loaded while edited
can't act on the approval of what the disk says later. The folder is
read as served — the extension's path joined with the manifest's
`bundle` — so a spelling that differs from the disk's only in case still
covers every file. The snapshots kept are the 16 newest; an older
frame's invoke is refused with "reload its lens".

**A hot update reaches a mounted frame.** Activation stages every
enabled extension's bundles into the generation it publishes
(`PreparedComponentBundles`), each stamped with that generation; a frame
whose snapshot belongs to a generation that is no longer active has its
`query` and `invoke` refused with "reload its lens". The host loads the
bundle again on every `extensionsChanged` (sent after an activation
publishes, `CustomComponentViz`): the same version keeps the frame it
has, a new one mounts a new frame — and that version acts only once a
person approves its code; the old approval doesn't carry over
(`an_activated_edit_needs_a_reload_and_its_own_approval`).

**Bundled and project components share one runtime.** A bundled
extension's files are embedded as bytes (`bundled_extensions.rs`,
`include_bytes!`), so its component's assets ship as they are; its
bundle is found from the embedded paths (`Embedded::bundle_stat`),
staged, hashed (the same `component_hash` over `bundled:<name>/…`),
served at `/components/v/<version>/` under the same CSP, and — when it
declares `commands` — listed on Programs and approved like a project's.
What still differs by origin, none of it about components: a bundled
advisory needs no approval, the `oxplow` command namespace is reserved
for bundled extensions, and a bundled extension's name wins a clash with
a project one's.

```yaml
custom_components:
  - id: burndown                       # [a-z0-9-]+
    title: Burndown
    bundle: components/burndown        # default components/<id>; holds index.html
    assets: [open-tasks, oxplow-bundled/visits]   # lens ids; a bare slug is this extension's
    commands: [oxplow.work_item.transition]
```
```yaml
# lenses/burn.yaml
title: Burndown
query: SELECT day, remaining FROM v_x_burndown WHERE stream_id = :stream_id
params: [{ name: stream_id }]
viz: custom
custom: { component: burndown, props: { color: accent } }
```

**Assets are lens ids, not models**: accepting a model would mean
accepting SQL from the frame; a component that needs a model writes a
`hidden: true` lens over it. Load checks, each at its line: the id, a
duplicate, `bundle` a relative folder without `..` that holds
`index.html`, no symlink anywhere in it, at most 256 files and 5 MiB
(`stat_bundle`; a bundled extension has none), each asset a lens id
alone (no `?params`, `@rev` or `#fragment` — `query(asset)` names the
lens) — this extension's must be among its lenses — and each command a
command name. A `custom` lens needs `custom.component` naming one of the
extension's components and a `query` (its rows are what the component
shows and what an agent reads); `spec_problem` refuses `custom` (an
answer can't carry one); an agent reads it through `lens_text` as its
table, prefixed ``(custom component `<ext>/<id>`; its table rendering)``.
`check_extension` checks that the declared commands are registered and
warns when a custom lens also fills a kit role block (`chart`, `tree`,
`timeline`, `steps`, `hunks`) — the kit may already render it; that is
the honest extent of a "this reimplements the kit" lint.

**The bridged calls** (P6b.D2, `lens_actions.rs`; the frame reaches them
only through the host): `run_component_query { id, asset, params,
stream_id }` runs `asset` — a lens id, a bare slug meaning the
component's extension — when it's one of the component's `assets`
(`Invalid` naming them otherwise), through `run_lens`: the lens's own
read-only, parameterised query, never SQL from the frame.
`invoke_component_command { id, command, input, stream_id, confirmed }`
runs `command` when it's one of the component's `commands` and a person
approved the component as it is now, as
`Actor::Lens { lens_id: id, on_behalf_of: Human }`, so every policy
applies as if the person ran it; the input is literal (no placeholders);
a command that asks comes back `NEEDS_CONFIRMATION` and the **host**
asks, never the frame. Both are UI RPCs (`ui` in surface parity).

**The host** (P6b.D4, `lens/CustomComponentViz.tsx` +
`lens/componentBridge.ts`): `<iframe sandbox="allow-scripts"
referrerPolicy="no-referrer">` at `componentBundleUrl(base, version)`
(the daemon's `/components/v/<version>/`, once `load_component` answered;
a bundle that can't be loaded shows the table and why; with no daemon
base — `remoteBaseUrl()` null — the lens shows its table), under a
**custom** badge. On the frame's first
`load` the host posts `init { protocol, run, props, tokens }`
(`initMessage`, the one builder) with one end
of a `MessageChannel` (to `"*"`: the frame's origin is opaque; the port
goes to that frame alone) and listens on the other end only. Frame
messages (`parseFrameMessage`): `ready`, `{ id, method: query, asset,
params }` (params SqlCell values only), `{ id, method: invoke, command,
input }`, `{ id, method: navigate, ref }` (an oxplow page only:
`componentNavigationTarget` refuses an external-url ref with `INVALID`,
since an outside tab would carry whatever the frame put in its URL out);
replies `{ id, ok, result |
error: { code, message } }`; a re-run sends `update { run }` — only
when its params or result differ from what the frame last got
(`createBridgeHost` takes the run `init` carried); a second `ready` is
ignored. The frame is keyed by its bundle URL (`ComponentFrame`): a
stream switch mounts a new frame whose first `load` is its own, and a
failure is remembered per URL. An invoke that asks shows the host's
`CommandConfirm`; confirming re-runs it confirmed, declining answers
`CANCELLED`. No `ready` within 3 s, or a second `load` (the frame
navigated itself), tears it down and shows the table. `ready` records
`usage { kind: "custom_component", key: <ext>/<id> }` — the evidence a
kind needs to be promoted. `tokens` are the root's CSS custom properties
(`tokensFromStyle`). No gesture check on invoke: the frame's
clicks don't reliably activate the host across webviews, and the real
bounds are the declared list, the person's policy and the host's
confirmation.

**The client library** (P9.A4). A bundle doesn't speak the protocol by
hand: the daemon serves `oxplow-component.js` (and its `.d.ts`) at
`/component-lib/<file>` — embedded in the daemon
(`crates/oxplow-daemon/assets/`, `components::component_lib`), served
like a bundle (loopback `Host` only, ungated, outside CORS, `nosniff`),
and named in every bundle's CSP `script-src` beside the bundle's own
folder (`bundle_csp(source, lib)`). There is no `'self'` in that CSP
(tsk983): even in a sandboxed frame it matches the daemon's whole origin
— the response URL's, checked in Chromium and WebKit — so it would let
an approved component load another bundle's code; and the check lint
reports a page path that leaves the bundle (`..` past its folder, an
absolute path other than `/component-lib/`, a `data:` script or sheet).
It is a **classic script that defines the
global `oxplow`**, not a module: module scripts are fetched with CORS,
which an opaque origin never passes and nothing served to a frame allows
— the same reason a bundle's own scripts are classic. `oxplow.connect()`
waits for `init`, answers `ready`, and resolves with `{ run (the latest),
props, tokens, protocol, onUpdate(fn) → unsubscribe, query(asset,
params), invoke(command, input), navigate(ref), applyTheme(doc?) }`; a
failed request rejects with the host's `{ code, message }`. `init`
carries `protocol` (`BRIDGE_PROTOCOL` in `componentBridge.ts`, `PROTOCOL`
in the library — one number, bumped when a message's shape changes), and
`connect` rejects a host speaking another. One app is one library
version. `componentClient.test.ts` drives the served file against the
real `createBridgeHost` and the real `init` (`initMessage`, which
`CustomComponentViz` posts), so the two can't drift (tsk856). `oxplow extension new
component <name>` scaffolds a private extension — the component, its
`viz: custom` lens and a bundle using the library — that checks and
tests clean (`just_works.rs`). The reference is in
`docs/guide/lenses.md`.

**`custom_components` is stable** (P11, tsk962), on the evidence rule:
a shared extension's component acts with it. The github example's **PR
Lifetimes** (`lenses/pr-lifetimes.yaml`, `components/pr-lifetimes/`)
draws each pull request as a bar from opened to merged — a range the
kit's charts don't draw; its filters re-run its own lens (`query`), a
bar opens `github_pr:<n>` (`navigate`), and Sync runs `oxplow.collector.sync`
(`invoke`) — refused until a person approves the component on
Programs. The host's own re-run (`onUpdate`, at the lens's params) is
asked again with a selected filter rather than drawn over it; a pull
request whose dates can't be read is left out and counted in a note,
never blanking the rest; a rejected `connect()` is said in the status
(tsk1008) — a component author's model for each. The loader no longer limits components to private extensions,
nor to project ones. The
browser suite's `specs/components/pr-lifetimes.spec.ts` runs it in
Chromium **and WebKit** (the engines of browser mode and the macOS
window; the suite's one WebKit project): the frame loads, the filters
and navigation work, approval gates `invoke`, and a self-navigation off
this machine sends no request in either engine — the blocked page's
load then ends the component and its table shows.

**The kit's stylesheet** (P11, tsk961). `/component-lib/oxplow-kit.css`
(`assets/oxplow-kit.css`, served beside the library as `text/css`, and
named in every bundle's CSP `style-src`) is a few `ox-` classes in
oxplow's look — text, card, badge, buttons, forms (`ox-form`, `ox-field`,
`ox-label`, `ox-input`/`ox-select`/`ox-textarea` with the accent focus
ring, `ox-error`), drag and drop (`ox-drag`, `ox-dragging`,
`ox-drop-target`, the Work board's dashed accent drop look), table,
states, chart series — that read only the theme's tokens (`kitSheet.test.ts`: every
`var(--x)` is one the app's root defines, and no literal color). A
bundle links it and calls `component.applyTheme()`, which sets each of
`init`'s `tokens` on the frame's root through the CSSOM — no `<style>`
text crosses the bridge. `init` used to carry the CSS as `kitCss`; that
went, and the protocol is 2.

**Check lints** (P11, tsk961, tsk994): what a bundle's CSP refuses
without a word, or what ends the component, is an error at check, at its
file (`bundle_problems` / `page_problems`, from `check_components`):

- an inline `<script>` that runs (no `type`, or a JavaScript one — a
  `type="application/json"` data block is fine) or an event handler on a
  standard element (a custom element's `on…` attribute is its own);
- a `type="module"` script;
- a load from outside the bundle (`src`, each `srcset` candidate, a
  `<link>`'s `href`) — a scheme other than `data:`, `//host`, an absolute
  path other than `/component-lib/` (matched before `?`/`#`), or `..`
  past the folder;
- a `<base>`, an `<iframe>`/`<frame>`, an `<object>`/`<embed>`, a
  `<form>`, a refresh `<meta>`;
- an `index.html` that never loads the client library.

Only each bundle's **`index.html`** is linted — the one page a frame
shows, since a nested frame is refused and navigating away ends the
component — and it is read from the **version under check**: the
candidate of a review (`ReviewSide::read`), the folder on disk for a
plain check. A version without the built page (a revision's tree) has
nothing to lint; loading reports a bundle with no `index.html`. A
bundled extension's pages are linted from its embedded files, the same
way. The scan is a
start-tag reader, comments skipped and raw-text elements (`script`,
`style`, `textarea`, `title`) read to their close, not a full HTML
parser.

## Decorators

`ui.decorators` (P6b.C5; **stable since P10**; `extensions/decorators.rs`)
put labels from one of the extension's **models** on core refs:

```yaml
ui:
  decorators:
    - { model: flags, kind: work_item, placement: ref-chip, label: label, color: color }
```

The model (`v_<extension>_<model>`) must declare a `ref` column and the
`label` (and optional `color`) columns named — checked at load against
the model's declared columns, which are its contract; column names must
be plain identifiers, since the desktop names them in its query. The
desktop (`components/decorators.ts`, `useDecorations`) runs each
decorator's queries over the refs it shows (`SELECT ref, "<label>" AS
label[, "<color>" AS color] FROM <view> WHERE ref IN (…)`), re-run when
the model changes. **Bounded** (tsk934): the refs go 200 to a query
(`REFS_PER_QUERY`), each query with a row limit that has room for all of
them, so a large table loses no badge; one extension adds at most 3
decorations to one ref (`MAX_DECORATIONS_PER_REF`), each label at most 40
characters (`MAX_LABEL`, then `…`). `ref-chip`: a chip in the header of a page whose ref the model
lists, after the page's own chips (`Page`, by the page's ref from its
navigation context). `row-badge`: a `RefBadge` (tone `label`) after a
lens cell that links to a listed ref. A color is used only when it's a
plain one (`#rgb…` or a CSS color name, `safeColor`). Decorations are
additive: a decorator whose query fails shows nothing.

**Promoted to stable (P10)** on its first-party use: oxplow-bundled
shows each effort's latest verdict on the effort — a `ref-chip` on its
page and a `row-badge` where a lens row links to it. Its model `verdict`
(`Accepted`, `Accepted (forced)` or `Changes requested`; green, orange,
red) reads `verdicts`, which keeps every verdict event
(`materialize: { incremental: seq }`, appended as each lands; a rewrite of
the log — retention's payload expiry, a restart's refill — refills it
whole). It reads only the events' **envelopes** (tsk886): the type says
the verdict (`oxplow_bundled.accepted` / `.changes_requested`) and the
subject what it was about — the effort first, then its work item, then
each claim and decision an acceptance took unchecked (any makes it
forced). Retention keeps envelopes and an extension's payloads go after 30
days at most, so a verdict a decorator shows must not live in its
payload. It shows; it never acts.

## The run context

Every run of extension code — and every bus run — carries one
`RunContext` (`oxplow_domain::run`; the app's constructors are
`run_context.rs`): its run id (a UUIDv7), its parent run, the
contribution it belongs to (`<ext>/<kind>/<local>`, the extension's
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
  model). What a mode may use is the services table's (`modes`), enforced
  by `RunContext::decide_service` ("Services" below): a pure run asking a
  model is refused before its script runs.
- **Stale is checkable.** `check(current_generation)` refuses a run
  built against a catalog generation no longer served, or cancelled.
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
  a registered run) and knowledge, snapshot and harness verbs.
- **Composed commands keep child ownership.** Routing captures each child's
  registered contribution factory with its retained handler. Nested composers,
  transactional leaves and external steps receive that child's context, not
  the parent extension's namespace or declaration. Transactional leaves narrow
  their mode; all children inherit cancellation and live guards. Native scope
  operations capture their implicit backing scope in declared authority.
  Routing and dispatch check run authority. This is handler attribution, not
  a coordinated DB/catalog activation or a new per-child command audit.
- **Every contribution's run is recorded once** (`extension_run`,
  `v_extension_run`; [data-model.md](./data-model.md) "extension_run"):
  its identity, actor, anchors, cause, outcome, cost (`cpu_ms`,
  `peak_bytes`, from each script's `Ran`), the scopes it called and its
  model calls — never what it read or wrote. Nested work of the same
  contribution adds to its parent's usage (`child()` shares it); another
  contribution's child is its own row, its `parent_run_id` the run that
  started it. A person's `oxplow.effect.retry` / `backfill` links the
  reaction's run to theirs (`under`) without making it theirs: the
  effect still runs as itself.
  **Reading it:** the bundled `runs` lens (`oxplow-bundled/runs`, in the
  launcher under Activity) lists, per extension and for the last 7 days, its
  runs, failures, denials, mean `cpu_ms` and model calls, the ones needing
  attention first. Settings → Extensions shows each extension's worst current
  run (`v_extension_run`, `latest = 1`).
- **Cancel reaches the worker.** Every script a collector, command,
  effect, policy or fact collector runs is passed its run's cancel flag;
  an exec collector's program is killed when it's set. Upcasts, report
  parsers, scripted AI mappings and the MCP adapter's mappings still run
  under their own fixed budgets without one.

## Services

A script reaches oxplow through **services**, each a scope it names in
its `needs` (`oxplow_domain::service::SERVICES`, one table every reader
uses):

| Service | Builtins | Approval | Modes |
|---|---|---|---|
| `sql.read` | `scope("sql.read", …)` | no | any |
| `tree.read` | `files`, `source_files`, `duplicate_blocks` | no | any |
| `ai.compute` | `ai_classify`, `ai_score`, `ai_summarize`, `ai_extract`, `ai_complete` | a person's | external only |
| `ai.discover` | `ai_capabilities`, `ai_models` | no | any, when the host supplies AI metadata |

- **Who may declare what** (checked at load): a Starlark collector's
  `needs:` any service (`check_declared`; another runtime has no
  builtins, so `needs` there is an error); a composing script — a
  command's `entry:`, an effect, an effort policy script — reads only
  (`scope_calls::check_script_need`: it runs pure, so `ai.compute` is
  refused, naming why). A **command's** script reads the semantic layer
  and not the tree: the bus composes it inside its transaction, and
  reading a whole tree there would hold the database while it reads
  (refused at load, naming an effect or a collector). A provider's
  `needs` stay `scope()` calls (`check_callable`: `host/call` answers
  `sql.read`).
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
  (declared / implied / undeclared, used or not, approval) —
  `CheckReport.services`, `oxplow extension check --services`; the MCP
  `list_services` tool serves `SERVICES`; `docs/reference/extensions.md`
  "Services" is generated from it (`the_reference_lists_every_service`,
  sharing `test_fixtures::generated_section` with the kinds table).
- **Where a person sees them.** Settings → Extensions shows a line per
  script under each project extension's row — "Collector triage — Read the
  captured tree · Ask a model (you approve it)" — worded from the service
  titles (`list_services`, one rpc over `SERVICES`; the surface-parity
  manifest has it as `both`), a service its script uses without naming it
  in `needs` flagged "(not in its needs)". The row's lines come from the
  extension's check (`validate_extension` → `CheckReport.services`); the
  install and update review (`ExtensionReview.services`, listed under what
  it declares and before the impact report) computes them from the
  candidate's own files (`of_extension` reads through the review's
  reader, never the installed copy of the same name). Bundled extensions show no
  line (they are not checked in the section). Row model:
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
  fixture) and the model (a `CollectorOracle` on the run, caller
  `collector:<owner>/<id>`). A `sql.read` call is decided as it's made
  (`Calls::serve`), not twice. A caller with nothing to give (a command;
  a dry run, which is pure) says the service isn't available there. The
  worker gets the result as its host set (`Given::onto`).
- **Approval** stays the host's: a script that names a model-calling builtin
  runs only once a person approved it — an entity collector
  (`check_approved`), a fact collector (`MetricsService::approved_to_ask`:
  an extension's in Settings → Data, the project's under Programs).
  Metadata-only discovery does not spend a key or require that approval;
  it still checks the declaration and live run authority. It never accesses
  credentials or probes a model. See [ai-providers.md](./ai-providers.md).
  Contained entity and fact collectors also pin their loaded script text to
  the current entry; an approval for changed code cannot authorize older
  cached text at a subsequent service call or final result.

## Grants

A **need** is what a contribution declares; a **grant** is what a person
approved; a **decision** is what a run was allowed (`oxplow_domain::grant`,
`grants.rs`).

- **The request.** `GrantRequest { scopes, program, hosts, credentials,
  env, commands }`, derived from the declaration: an exec collector's
  entry, `network`, `credentials` and `env`; a collector's services
  (`services::collector_needs`: declared ∪ implied, `ai.compute` among
  them for a model-calling script — a model is a scope, not a flag); a
  script's (effect, command, effort policy) scopes from its `needs`; a
  project program's from its listing (`program_request`).
  `widening(granted)` names what a new request asks beyond a grant; a
  narrower one asks nothing new.
- **The grant.** `Grant { request, granted_at, granted_by, revoked_at }`.
  What a run decides against is always the request derived from the
  declaration *now* — the approval's hash proves that declaration is the
  approved one — never a request stored in the approvals file, which a
  process writing that file could widen.
- **The authority** a run carries (`RunContext::authorized`,
  `run::Authority { declared, grant, enforcement }`): the grant when a
  person's approval is what lets it run (a program, a model-calling
  script, an effect), else its declaration alone (a script command, a
  policy's script, a provider's `host/call`s — decided against the
  provider's own `needs`). Core's own work has none and is allowed what
  it does.
- **Decisions.** `run.decide(Need)` → `Allowed`, `Denied(why)` or
  `Unenforced(why)`, recorded on the run (`v_extension_run.decisions`):
  each scope call (`Calls::serve`); a program collector's `program`,
  `files`, each `env` name, each `host` and each `credential` — a
  credential its grant doesn't name is never read from the keychain; a
  script's services (`scope` `tree.read` / `ai.compute`, by the router).
  A denied need stops what asked for it.
  `Enforcement { hosts }` is the one table of what this machine holds.
  A program's file and network decisions remain `unenforced`: macOS's
  proxy and direct-network sandbox do not prevent local brokers or file
  access. `grants::gaps` reports this in Programs and review grant lines.
- **Bounded reads.** `sql.read:v_work_item` names an exact published
  model; use multiple needs for multiple models. SQLite preparation checks
  the query's models and transitive registered `ref` inputs, including
  materialized models, before evaluating rows. Bare `sql.read` preserves
  the explicit whole-layer grant. Bounded queries cannot currently use
  `metric_grid` / `metric_findings` because their host precomputation has
  no model authority contract; query granted published models instead.
  `tree.read:src/main.rs` names a captured file; `tree.read:src/` names a
  directory. Absolute paths, traversal, backslashes and globs are refused.
  The host filters its captured tree before giving it to a script; these
  needs do not grant raw filesystem access. Bare `tree.read` grants the
  whole captured tree.
  Manifest validation, capability availability, command script checks and grant
  request construction recognize the validated service prefix while preserving
  the original qualifier. Transactional command composers still refuse bounded
  tree reads because their tree service is unavailable inside the transaction.
- **External execution.** Every external program (an exec collector, a
  project fact program, a report parser, an external provider, a harness
  or MCP stdio server) runs with the person's file, network and process
  authority. Nothing declares that, and the approval says so plainly: a
  program "runs with your file, network and process authority; oxplow
  cleans up its processes but doesn't confine it". Confining external
  processes is shelved (the owner's decision); a confinement tier is
  deferred until real OS confinement exists, and there is no `execution:`
  key (a manifest still declaring it is a load error). A program's file and
  network decisions stay `unenforced`. What oxplow does do is cleanup, not
  confinement: on Unix each program runs in an owned process group, killed
  on completion, timeout, cancellation or revocation. A process can escape
  its group. Platforms without owned-group cleanup just run the program.
  Pipe I/O also has a deadline, so escaped children cannot leave host
  drain/writer threads blocked indefinitely.
  Environment/credential delivery is deliberately process-readable;
  contained script services never receive keychain values.
  Entity collector programs run from a private per-run copy, verified against
  the approved declaration and code hash before launch and during execution.
  Their working directory and `OXPLOW_EXTENSION_DIR` name that copy, so helpers
  load the approved code. The complete copied folder is limited to 500 files
  and 16 MiB, including its manifest and lenses; oversized folders fail before
  launch. Entries under normally excluded metadata paths are hashed explicitly
  and need renewed approval. Copy preparation rejects symlinks and special
  files; the copy is removed when the run releases it. Original-tree edits or
  copy changes stop the run under the live authority policy.
  Disk extension reads also refuse special files and files larger than 16 MiB;
  Unix opens refuse a final symlink and avoid blocking on replacement FIFOs.
  Project fact programs and report parsers use the same bounded copy helper,
  with their project-relative code layout preserved. Their guard owns the copy;
  `OXPLOW_CODE_DIR` names its root and `OXPLOW_PROJECT_DIR` names the original
  checkout for their approved data access.
- **Collector previews.** A preview stores no rows, but receives the same
  contribution authority and pinned live consent as an ordinary entity
  collector run. Previewing cannot bypass declared model/path access or
  continue an AI/program call after its approval changes.
- **Revoking** removes the approval ([architecture.md](./architecture.md)
  "A repo's config never runs a program without consent"). Host-owned
  `RunContext::guarded` checks pin the approved version, compose through
  nested runs and recheck before sensitive calls/publication. Replacement
  approval cannot authorize an already loaded old script. External hosts
  poll live consent with 100 ms between checks; entity collector processes
  use 25 ms and report/fact programs 10 ms between checks. In-flight AI futures are dropped and their results discarded.
  This cannot undo an external side effect already performed.
- **Component authority.** Query messages carry the loaded bundle version.
  Asset and command allowlists come from that immutable bundle; adding
  manifest permissions cannot widen a running frame. Asset declarations
  are included in component approval hashes.
- **Not yet:** OS confinement and model call-count/monetary budgets.

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
| `subject` | the canonical refs it's about |
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
  refs and its anchors typed ids. What it doesn't give is `null`, except
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
`oxplow_bundled.accepted@1 { unverified, inferred, deviated }` and
`oxplow_bundled.changes_requested@1 { unverified, inferred, deviated,
note? }`, which `accept.star` and `request_changes.star` return in
`events:` (subjects: the effort, its work item and, for an acceptance, the
claims and decisions it took unchecked; caused by the run's
`command.executed`). They replaced a single `verdict@1` type, whose
verdict lived in an expiring payload (tsk886).
Before them, a verdict lived only in a comment's text; now the effort's
timeline carries who decided what, and another extension can react to it
("Reacting to another extension's types"). What makes the kind safe to
promise is already built: a published `type@v` is a contract
(`event_type_contract` refuses a changed schema — stronger than a
model's drift warning), a new shape is a new version with an upcast, and
a removed type's rows stay readable. It was promoted to a stable kind
on that evidence; every kind is stable now (`ui.decorators` and
`ref_kinds` in P10, "Decorators" and "Ref kinds"; `effects` and
`custom_components` in P11, below and "Custom components";
`ui.replacements` with oxplow-bundled's history graph; `providers` once
the provider host was), so a kind carries no stability.

**`effects` is stable** (P11, tsk956), on the evidence rule: a bundled
extension acts with one. oxplow-bundled's **`verify-unchecked`** reacts to
`oxplow_bundled.accepted`: its script takes the acceptance's subject from its envelope
(`event["subject"]`) — the effort, its item, and each claim and decision
accepted unchecked — and reads (`scope("sql.read")`) only what the event
can't say: which of those are still unverified or inferred now, and its
own earlier reactions. It is empty when
the effect already filed (or proposed) a follow-up for an earlier
acceptance of the same effort (its `v_effect_run` reaction to that event
is `ok` or `proposed`, tsk991; an earlier acceptance from before its
approval, or whose reaction skipped or failed, doesn't count). With
nothing unchecked it skips; otherwise it files **one** item on the
active tracker, like every new item (tsk1058) — "Verify what the review of <effort> accepted
unchecked", a checklist naming each claim and decision. Each line is
text, not markdown (`md_text`: whitespace becomes one space, markdown's
punctuation is escaped and a long one is cut at 300 characters), and the list stops
at 50 items, saying how many more the review has. A bundled effect
is approved like any (K1, tsk953: its embedded files are hashed alike),
so it runs only once a person approves it. A task an effect (or oxplow
itself) files has no `author` — it isn't the person's; its
`work_item.created` and the run's audit name the effect (its `native`
`author` is null). The loader no longer limits effects to private extensions.

**Retention** (P8.D5) is the namespace's window for payloads and large
content (data-model.md "event_log" retention): either omitted part is the
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
one listing that adds model and vocabulary health):
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
logged version (`data-model.md` "event_log"). `registered` is 0 too for a
type whose extension is there but whose declaration was refused (a
schema changed at a recorded version, a namespace collision) — its
health says why (`v_event_type` v2's column doc says so, tsk800).

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
`project` or `built-in` collector's (no extension, no namespace), a type
the collector's own `trigger.on` names (its run would trigger itself),
and more than `MAX_RUN_EVENTS` (100) a run. A fact collector doesn't
emit. A preview (`CollectorPreview.events`) and a `extension test` example
(`expect: { events: [{ type, payload? }] }`) show what would be logged;
a review's dry run counts them per type ("2 pr; emits 2 acme_pr.merged").

**The boundary.** A collector **ingests**: it turns the outside (or
existing state) into rows, and its events say *what it saw* — "a pull
request merged". It has no rights to act, needs no approval beyond its
program's, and may run again and again over the same input. An effect
**reacts**: it composes commands with an agent's rights, is approved by a
person, and runs at most once per event. So "when a PR merges, close its
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
`trigger: { on: [...] }` and an effect's `on:` may name core types, the
extension's own declared ones, and **another extension's**
(`acme_pr.merged` from an extension that isn't `acme-pr`). There is no
`depends:` key: the qualified type name is the dependency, as a model's
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
declaration is a load problem, like a model's contract drift, not a
failing run that counts toward a disable.

## Ref kinds

`ref_kinds:` (P8.D6; **stable since P10**; `extension_ref_kinds.rs`,
`vocabulary_reactor.rs`) adds kinds of thing a ref can name. It was
promoted on the github example's pull requests (an example counts as a
first-party use): kind `github_pr` over model `pull_request`, page `pr`,
`[[pr:12]]`. What is promised: the manifest keys below, the portable id
pattern subset, the page's `?ref=`, and `v_ref_kind`'s columns.

```yaml
ref_kinds:
  - kind: acme_pr             # <namespace>_<name>
    label: Pull request
    id: '^\d+$'               # anchored
    resolve: prs              # one of its models, with `ref` and `title` columns
    page: pr                  # one of its pages, opened with `?ref=<ref>`
    wikilink: pr              # optional `[[pr:12]]` sugar
    searchable: found         # optional: one of its models, with `ref`, `title` and `body`
    icon: git-pull-request    # one of REF_KIND_ICONS
```

**Loading** (after models and pages) refuses, at `extension.yaml:<line>`:
a kind outside the extension's namespace, an unanchored, broken or
non-portable id regex (the desktop runs it in JS's backtracking engine,
so it's held to a subset both engines read alike and JS can't be made
to hang on — tsk797, tsk917 — parsed by `id_pattern`: printable ASCII,
at most 256 characters, anchored `^…$` (an escaped `\$` isn't the
anchor); characters, classes (not negated, no `[` inside, no `&&` `--`
`~~` set operations), `\d`, `\w` and escaped punctuation (not `\<`
`\>`, word boundaries in Rust), each with at most one quantifier (`? *
+ {n} {n,} {n,m}`, at most 256; no lazy `?`, no `{,m}`); no groups,
alternation, `.` or other escapes. Two repeats of varying length must be
fenced by a character the first can't match — `^[A-Z]+-\d+$` loads,
`^\w+_\w+$` doesn't — so a failing match gives each back at most once,
and no id length needs capping. What is kept, and what both the registry
and `v_ref_kind.id_pattern` carry, spells every set out — `\d` is
`[0-9]`, never Rust's Unicode digits — escaping exactly what either
engine reads as syntax), a `resolve` that isn't one of its models with `ref` and `title`, a
`page` that isn't one of its pages, a `wikilink` core already reads
(a core kind, `git`, `dir`, `finding`, `tsk`), an icon not in
`REF_KIND_ICONS` (lucide names the desktop maps), a duplicate.

**Registering** is the vocabulary reactor's: core kinds plus each
extension's, built by one constructor (`extension_ref_kinds::kind_spec`,
which `extension test` uses too), so `validate_ref`, `[[acme_pr:12]]` and
`[[pr:12]]` know them while the extension is installed and not after. A
kind's namespace is a string prefix, so one may hold another's (`acme`'s
`acme_` holds `acme-pr`'s `acme_pr_`): a kind in both is **the more
specific namespace's** (tsk933) — `acme-pr` keeps `acme_pr_x`, and
`acme`'s declaration of it is an error naming whose namespace it is.
That, and any other kind two extensions both declare (the first keeps
it), is settled at load by the descriptor registry
(`descriptors::check_catalog`, which registers ref kinds most specific
namespace first); the second's declaration is a load error naming both. A `wikilink:` that is one of the extension's own
kinds is refused at load and costs only the sugar (both kinds load), so no name is both a
kind and a sugar, and both resolvers read a sugar before a kind's own
name (`canonical_wikilink`, `extensionWikilinkRef`). A **`wikilink:` prefix** another extension also uses (as
its prefix, or as its kind) is an error on each and **costs only the
sugar** (P10): `[[pr:…]]` links neither, but each namespaced kind still
registers and links as `[[acme_pr:…]]`, so installing one extension never
unlinks another's refs. A kind or prefix core holds is that extension's
error. The pass
restates `ref_kind` (V146) whole, read as `v_ref_kind` (kind, extension,
label, id pattern, revisioned, wikilinks, resolve, page, icon); unlike
event types, a removed extension's kinds leave, and refs to them are
unrecognized again.

**The desktop** (P8.D7, `apps/desktop/src/refKinds.ts`) reads
`v_ref_kind`'s extension rows once (`useRefKindsLoader`, in `App`,
re-read on `ModelsChanged`) into one process-wide list that the pure
helpers consult and `useRefKinds` subscribes to:
- `pageKindIconComponent` / `pageKindLabel` fall back to a kind's icon
  (`REF_KIND_ICONS`, a lucide component per allowed name — a Rust test
  keeps the two lists equal) and label;
- `refFromTabId("acme_pr:12")` opens `page:ext.acme.pr?ref=acme_pr:12`
  (`extPageRef` carries params; `ExtensionPageView` hands them to the
  lens as `initialParams`);
- `preprocessWikilinks` turns `[[acme_pr:12]]` / `[[pr:12]]` into a link
  when the id matches the kind's pattern (`extensionWikilinkRef`, as the
  backend's `canonical_wikilink` does), `urlTransform` lets the kind's
  scheme through, and the link's text becomes its title from the
  `resolve` model (`useExtensionRefTitle`) unless the author labelled it.

**Searchable kinds** (P9.D3, `kind_search.rs`). `searchable: <model>`
names one of the extension's models with `ref`, `title` and `body`
(checked at load, like `resolve`); `v_ref_kind.searchable` is its view
(V155). Its rows are in the site-wide search index (`search_fts`) under
the kind, so the launcher finds a pull request by its title or text and
the hit opens the kind's page (`searchHitTarget`, the one place a hit is
routed; the row shows the kind's icon and label).

- It is **index-time ingestion, as an asset** — not a query-time UNION:
  `search_fts` owns the text it ranks (BM25 and `snippet()` need it in
  the index), and a UNION would run every extension's SQL per keystroke,
  unranked. One `Materializer` per searchable kind (`search:<kind>`), its
  inputs the tables behind the model's view (the registry's lineage; a
  materialized model's own table): a commit touching one re-reads the
  view after the assets' quiet window and restates the kind's entries in
  its own transaction — never the writer's.
- `Assets::sync_search_kinds` keeps them in step with `ref_kind` and the
  model registry (the change loop runs it at start and when `ref_kind`,
  `model` or `model_input` change). A kind that stops being searchable —
  its extension removed, the key dropped — leaves the index with its
  entries; so does one that went while oxplow wasn't running (an
  `asset_state` row `search:<kind>` with no kind behind it).
- Bounded: 20,000 rows a kind, 16 KiB of body a row. A row whose `ref`
  isn't `<kind>:<id>` with an id matching the kind's pattern is skipped
  (a hit must be something that opens), with a warning in the log.
- Entries are project-global (`stream_id` NULL): a model is.

**Revisioned extension kinds: designed, not built** (deferred in P10 by
decision — nothing first-party has revisions to read). Today an extension's
kind's ref takes no `@rev`: a revision is read by its reader (`git:`,
`snap:` — the VCS and snapshot stores), and no extension's kind has one. The
design, so building it is filling in, not deciding:

- **The declaration:** `revisioned: true` on the kind. The loader then
  requires its `resolve` model to declare a **`rev`** column: one row per
  revision of a thing, `rev` NULL for the current one (so an unrevisioned
  ref keeps resolving as now: `WHERE ref = :ref AND rev IS NULL`).
  `KindSpec::revisioned` follows from it.
- **The page:** its lens declares a `rev` param, and the page opens as
  `page:ext.<ext>.<page>?ref=<ref>&rev=<rev>` (`rev` omitted: current).
- **Refs:** `[[pr:12@git:abc1234]]` and `github_pr:12@git:abc1234` parse
  through the one ref grammar (refs.md); the registry's `validate`
  accepts `@rev` only for a revisioned kind, as for core ones. A rev is
  opaque to the kind — the kind's model decides what it matches.
- **Search** entries stay rev-less: a hit is the thing, at its current
  revision (`searchable:` reads the `rev IS NULL` rows).
- **What would make it real:** the github collector keeping one row per
  pull request **head commit** (`rev` = `git:<head sha>`, the latest
  also stored with `rev` NULL), and a consumer that wants a pull request
  as it stood — a review or an effort linking the PR at the commit it
  reviewed. Until something reads it, building it would be an unproven
  contract.

## Effects

`effects:` (stable since P11, tsk956 — a shared or bundled extension's
too; `effects.rs`) are scripts that react
to a logged event by composing commands, run as
`Actor::Effect { effect: "<extension>/<id>" }` for the event's thread and
stream — an agent's rights and gates there (the writer gate, its own
stream's VCS), never a confirmation (commands.md "Only a person
confirms"):

```yaml
effects:
  - id: announce-done          # [a-z0-9-]+, unique in the extension
    summary: Note a finished item on its thread.
    on: [work_item.state_changed]  # core types, its own, or another extension's ("Event types")
    where: { to: done }            # optional: payload fields equal to these
    needs: [sql.read]              # the scopes its script calls: only what a script can (`scope_calls::CALLABLE`)
    entry: effects/announce.star   # transform({event}) → {commands, events?} | {skip}
    after: [page_ref.work_item]    # optional: consumers it waits for
```

**Loading** (P8.D9) reads `on`/`where` through the collectors'
`parse_trigger` (one rule for both) against core types and the
extension's own `event_types`, and needs the script in the folder,
defining `transform`; what's wrong is `extension.yaml:<line>`.

**Consent and start position** (P8.D9). An effect is a project program
(`exec_consent::ProgramKind::Effect`, key `effect:<extension>/<id>`,
listed in Settings → Data with what approving means): its approval hash
covers the script and **every file of its extension's folder** — the
manifest decides when and with what it runs — so any edit stops it until
a person approves it again. Approving (the RPC, a person's only) also
sets `effect_state.start_after_seq` (V148) to the log's head
(`effects::approved`): an effect never reacts to an event logged before
its latest approval — not the backlog, not what happened while it waited
to be re-approved. In the same transaction it drops the effect's scheduled
automatic retries (`drop_retries_of_tx`, tsk990): what they would send
was composed by the script as it was, and the approval is of the script
as it is; those reactions stay failed, for a person's Retry, which
composes afresh. `effects::gate(approved, start_after, seq)` is the one
check: `Unapproved`, `BeforeApproval` or `Runs`. The consumer hashes an
extension's folder once per catalog load (`FolderHashes`, keyed by the
catalog's `Arc`), not per event: an edit reloads the catalog, so it's
hashed again and stops the effect (tsk798). The approvals file is read
on each check, so revoking one (Settings → Data → Programs) takes effect
at its next event.

**A bundled extension's effect is approved the same way** (P11, tsk953).
What an approval covers is read through one resolver,
`extensions::files_at(project_dir, path)`: a bundled extension's
embedded files for `bundled:<name>`, else the folder on disk. The digest
(`exec_consent::files_hash`, over `ExtensionFiles::paths` / `bytes`) is
the same for a folder and its embedded copy, and stays byte-identical
for folders (path order component by component; pinned by
`a_folders_digest_is_pinned`). So a bundled effect has a version to
approve, there is no bundled branch in the gate, and a new oxplow that
changes the extension's files asks again. Settings → Data says the
effect comes with oxplow and offers **Read the script** — the
`program_source` RPC (UI only) returns any program's entry from where it
lives.

**Running** (P8.D10, `effect_triggers.rs`): one async pump consumer,
`effect.triggers` (registered at boot beside `collector.triggers`, with
the same `after`/`after_for` handling of each effect's `after:` — one
naming no consumer is warned about once per effect and name, not on each
ordering question the pump asks). Per
event, each enabled effect whose `on`/`where` match **reacts at most
once**, keyed by `effect_run (effect, event_id)` (V149, `v_effect_run`):
1. a row exists — a redelivery: nothing. A `started` row the live
   consumer claimed is a run that claimed a step outside oxplow and was
   cut off (`effect_triggers::cut_off`): recorded `failed`
   ("interrupted"). When its claim kept what it composed — every step a
   write to a provider keeping `idempotent_writes` (`claim_tx` writes
   `resend_json` then, P11, tsk954) — it is **sent again by itself** like
   a failure that may pass, its delay counted from when it started (so
   one found more than an hour late is a person's, and so is one whose
   start can't be read — tsk999); otherwise it is a person's, never sent
   again by itself. Recording the failure and scheduling its retry are one
   transaction (`effect_triggers::fail`, tsk999), here and for a failure
   that may pass. One a person's retry or
   backfill claimed is theirs, under way, and the pump leaves it be
   (tsk847). A person's retry or backfill — or an automatic attempt — is
   no pump delivery, so nothing redelivers one cut off between its claim
   and its record: at start (`boot.rs`,
   `effect_triggers::recover_interrupted`) every attempt still `started`
   whose origin isn't `live` is recovered the same way, with its
   `effect.result` saying what started it — Delivery lists it, and a
   person may retry it (tsk845) — and counted toward the effect's health
   as the pump counts one it finds (tsk999);
2. `effects::gate` isn't `Runs`: nothing;
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
`RunOrigin::Effect(key)`: its `command.executed@3` is caused by the
triggering event, and the reaction's `effect_run` row and
`effect.result@4 { effect, event, outcome, reason?, proposal?, attempt,
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
human-only, `Confirm::Always`, `External`; an `effects.run` operation
declared in `oxplow-foundation`, its services filled when the consumer
registers at boot — `ServicesSlot`) runs `effect_triggers::run_reaction(…, ReactionOrigin::Retry)`,
the same steps as a live reaction from the loop guard on, as the next
attempt:
- the reaction's latest attempt must be `failed` (`Invalid` otherwise: one
  that ran, was skipped or left a proposal isn't retried; one never made
  has nothing to retry);
- the effect must be enabled and **approved as it is now** — a retry runs
  today's script, composing afresh from the event, not what the failed
  attempt composed;
- when the event was logged doesn't matter (`start_after_seq` is the live
  consumer's rule; the person named the event);
- it counts toward the effect's health like any attempt, and is recorded
  `effect.result@4 { …, attempt, origin: retry }` (v3 upcasts keeping its
  attempt and origin; v2 as `attempt: 1, origin: live`).

It asks every time because of what a failure can hide: an attempt
interrupted with a step outside oxplow under way may have landed that
step, and a retry sends it again. The confirmation's text says so.
Settings → Data → Delivery lists the reactions whose latest attempt
failed (`useFailedReactions`), each with its reason and **Retry**
(`InlineConfirm`).

**Sent again by itself** (P10, `effect_triggers::auto_retry_due`). An
attempt that failed in a way that may pass (`CommandError::Unavailable`,
tsk914: a step's provider erred, timed out, lost its reply, or asked to
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
attempt moves it there, so no composition outlives its retry (tsk999) — and the automatic attempt **sends exactly that**
instead of running the script again (tsk887): composing afresh could read
changed rows, change a step's input and so its key, and make a write that
landed again. It is sent only while every step still goes to a provider
keeping the promise; otherwise it isn't sent (`Reacted::NotResent`) and the
failure counts, a person's. A composed `oxplow.work_item.create` files on the
tracker active when it's **sent**, like every create (tsk1058): pointed
elsewhere meanwhile, the retry goes there — or, when that tracker keeps
no idempotent writes (oxplow's own), isn't sent by itself and waits for a
person. Each step carries its idempotency key, the
same on every attempt ([commands.md](./commands.md) `effect_step_key`), so
a write that landed lands once. A person's **Retry** composes afresh from
what the effect reads now — the person decides it should. A loop every 5 s (`spawn_auto_retry`, at boot — its first tick only
after the provider registry's first generation, so the providers the
config names are up; tsk915 — one row's error logged and the rest
going on, tsk932) runs what is
due as the next attempt, `origin: auto` (`effect.result@4`), on the event
at its newest version (`at_latest`, as every runner hands it over; one that
no longer upcasts drops the retry — tsk911), when the effect is still
there, enabled and approved as it is now and the retry is at most an hour
overdue (`MAX_RETRY_LATENESS`: one due longer ago — oxplow was closed — is
a person's, its failure counted; tsk915; a due time that can't be read
counts as late, tsk999) — otherwise the
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
before an effect's approval. A person has it react to that past with
`oxplow.effect.backfill { effect, from_seq? | since?, to_seq? }` (human-only,
`Confirm::Always`, `External`): the events in the range that match its
`on` and `where` — each read at its type's newest version, as the pump
delivers it — and that it **never reacted to** (`unreacted_tx`: no
`effect_run` row) and its own run didn't lead to (the loop guard's `own`,
`event_lineage::lineage_tx`: such an event is never its trigger, so it
isn't planned — an effect that changes what it reacts to would otherwise
meet its own changes on every backfill and never get past them, tsk846),
oldest first, **up to the effect's `start_after_seq`** — what was logged
after its approval is the live consumer's, so the two never attempt one
event at once; a range past it is capped (tsk847) — each through
`run_reaction(…, ReactionOrigin::Backfill)`: attempt 1 with `origin:
backfill`, the same dedupe, loop guard, approval and health as a live
reaction. So a second backfill finds nothing, the live consumer never
reacts to a backfilled event again, and three failures in a row disable
the effect and stop the run (`stopped`) — three of the run's own in a row
stop it too, counting ones that will be sent again by themselves (whose
health waits for the retry; tsk932), so a backfill doesn't press on into
an outage. A run makes at most
`BACKFILL_BATCH` (200) reactions; the result `{ planned, ran, skipped,
proposed, failed, remaining, stopped? }` says what is left for another
run. The effect must be enabled and approved as it is now.

`oxplow.effect.backfill_plan` (a read, anyone's) answers `{ planned, from_seq,
to_seq, batch }` for the same input: what a backfill would react to. Both read
the candidates by log position a page at a time (`SCAN_PAGE`) to the end
of the range, applying `where` to every one — never to a first window of
them, which a selective `where` over a busy type would leave empty for
good (tsk848) — counting all and keeping the batch's first. The bus's
confirmation shows a command's summary and input, not a count, so the
count is the plan's: an approved effect's row in Settings → Data →
Programs has **Backfill…**, which reads the plan, says how many events
the effect never reacted to and that it may call outside oxplow for each
(`backfillAsk`), and runs only on the second click — on the range it
showed (`to_seq` from the plan, so what was logged in between isn't in
it), its button saying what one run does: "Run on the first 200 of 500"
when the plan is more than a `batch` (`backfillRunLabel`, tsk849).

**There is no consumer-level replay.** Core's consumers are re-derivable
(a projection is rebuilt, not replayed); replaying the log through every
consumer would re-fire collectors noisily and effects without consent.
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
    input_schema: { type: object, required: [ref], properties: { ref: { type: string } } }
    entry: handlers/finish_review.star # defines transform(x), x = { input }
    confirm: never                     # never (default) | always | destructive; children only add
    access: write                      # write (default) | record | read (composes nothing, returns a result)
    invokers: { human: true, agent: true, lens: true }   # required: who may run it (within its operation's floor, for a scope command)
    needs: [sql.read, work_items.comments]   # scopes it calls; capabilities / features it needs active
    ui:                                # how a person meets it (`.context/commands.md` "Offering a command")
      label: Finish Review
      group: Review
      about: work_item                 # ref-bound; absent: search offers it
      input: { ref: "{{ref}}" }
    examples:
      - { name: happy, input: { ref: "work_item:oxplow:tsk1" }, expect_commands: [oxplow.work_item.transition] }
      - { name: gone, input: { ref: "work_item:oxplow:tsk9" }, answers: { sql.read: [[]] }, refuses: no such }
```

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

A shared extension's commands are `Stable`, a private one's
`Experimental`. `transform` returns `{ commands: [{ name, input }], result?, events? }`, or
`{ refuse: "<why>" }` to decline — the run is `Invalid` with that reason
and writes nothing (`composed` → `Composed::{Run, Refused}`). An `access:
read` command composes nothing and logs no events (refused at run time
otherwise): it reads and returns a `result`, is never confirmed and needs
no scope that changes things; the bus runs it on a read snapshot,
unrecorded. An example may give `answers:` — per scope, its calls'
answers in order, standing in so it doesn't depend on the project's data
(a scope without any is served for real; one that runs out is an
error) — and may expect a refusal with `refuses: <part of the reason>` instead of
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
`name:` is exactly that name — `a` never lands on `abc`; ids, lenses,
and models find their lines the same way): the name, `access`, `confirm`,
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
(summary "… (extension `x`)", the declared invokers / confirm / access,
`Lifecycle::Experimental`). Its composer runs `compose_calls`, answering
`sql.read` on the connection it's given (`semantic_layer::read_on`: the
`query_sql` authorizer, row cap and timeout, the read session restored —
`query_only` off — before any writes) and counting each call in the run's
`ScopeTrace` (pure, so the bus may compose more than once: once to
route, again in the run's transaction, whose calls are the ones recorded).
The bus runs what it composes ([commands.md](./commands.md) →
"Composition"): when every call stays in oxplow's records, as children
in one transaction (`steps::run_composed`; the script runs once, when the bus routes the run) — each child's invokers, policy and
confirmation apply (an agent's run whose child asks becomes a proposal
with the children as its dry run), one audit row with `{ result,
children }`, the children's events caused by the run, the reversed
children as its undo; when one leaves it (`work_item.*` on another
provider's item, tsk713), as **steps** — all checked first, then run in
order, each landing as it runs, stopping at the first failure, one audit
row, no undo. A script can't do I/O of its own: `files()` sees
nothing, `ai_*` is refused without a host, and `scope` reaches only
what it `needs`. **Registration**
(`ExtensionCommands`, `Services.extension_commands`): the enabled
extensions of the **primary worktree** (one bus, like providers — a
command authored in another stream registers once merged), one
namespace at a time, all-or-nothing; registered by the generation at boot and on the
extension catalog's change signal (`ExtensionCatalog::changes`, P7.B6):
the workspace watcher fires it for a file under `oxplow/extensions/`
**in the primary worktree** only (another stream's edits wait for the
merge), the `config.extensions` reactor for the `extensions` key
(enabling or disabling). The extension models, the provider registry and
the metric catalog follow the same signal; a namespace something else
already holds (a provider) is refused and kept as the extension's
`problem`. Replacement is prepared against a cloned command registry and
published with one write-lock swap; rejection keeps the previous command batch.
Missing/disabled sources are removed. Registration tracks namespace, declarations,
package version and catalog identity, so provenance changes rebuild the batch
without requiring a script change. Each handler captures version and generation
from the same `CatalogSnapshot` as its declarations; a retained handler never
looks up a newer candidate's identity when invoked. Other registries and SQL
models still refresh independently. A launcher `{ command }` entry may name one.

## Contributing metrics (current)

`extension.yaml` takes `measures:`, `metrics:` and `dimensions:` in the
`.oxplow/project.yaml` schema, checked with the same
`oxplow_config::validate_*` functions, and fact collectors (a `collectors:`
entry with `facts:`, parsed by `oxplow_config::collectors` like the
project's), plus:

- Metrics must be `key:` definitions; `use:` belongs to the project.
- Entity metrics (`entity:` + `where` / `time` / `value`, tsk322) work
  here too, usually over the extension's own source views. Their fragments
  are checked at seed time, so one over a view whose source hasn't synced
  yet stays out of the catalog until the next reseed.
- Dimensions (tsk328), fact or entity (`entity` / `expr` / `join`), layer
  like metrics (`resolve_dimensions` takes an `extensions` layer) and slice
  an extension's entity metrics over the same view. They are stored as
  `scope = 'global'` plus the `extension` column (V89).
  - `promote` is refused: toggling the extension would rebuild the
    metric cube each time, so promoting stays a project decision.
  - A disabled or removed extension's dimensions are deleted
    (`delete_extension_dimensions_not_in`, never a promoted one). Facts
    don't reference dimension rows, so nothing else is lost.
- Fact collectors run `starlark` / `jaq` only, with no `env` /
  `credentials` / `network`. `exec` is refused: a program that records
  facts runs only from the project's own `collectors:`, with approval;
  an extension's programs are entity collectors, which a person approves.
  `entry` must exist in the extension.
- Precedence is built-in < global < each extension < project
  (`oxplow_config::resolve_{metrics,measures,dimensions}` take an
  `extensions` layer; scope `extension:<name>`). A collector id two owners
  declare runs once — project over extension over built-in
  (`MetricsService::fact_collectors`). An enabled extension's
  metrics are **on** unless the project mentions the key (a `use:`
  override or disable marker, or its own definition).
- `MetricsService::extension_catalog` loads enabled extensions from the
  primary worktree on each resolve; fact-collector scripts are read through
  `extensions::read_extension_file` (bundled or disk), and a collector's
  `report` still resolves against the project.
- Storage: the scope CHECK only allows built-in / global / project, and
  rebuilding `measure` / `metric_spec` would cascade-delete facts (V54),
  so the store writes `extension:<name>` as `scope = 'global'` plus the
  `extension` column (V84) and reads it back. `v_metric_spec` shows
  `scope = 'extension'` and the `extension` name.
- A generation prunes extension specs no longer declared (a disabled or
  removed extension) with `delete_extension_specs_not_in`; its measures
  stay, since deleting one would take its facts. A new generation follows when an
  `oxplow/extensions/*/extension.yaml` changes (`WorkspaceChanged`).

## Health, disable and repair (P7.C1–C3)

Every contribution that runs — an external provider instance, a collector, an effect
(P8.D11, kind `effect`, V150) — shares one failure policy
(`contribution_health.rs`, the `contribution_health` table read as
`v_contribution_health`):

- **Counting.** A failed run or call counts; a good one starts the count
  over. A refused input, a missing approval, or a cancel isn't a failure.
- **Disabling.** The third failure in a row disables it on this machine
  and logs `contribution.disabled@1` (the row and the event commit together). A
  provider instance stops. A disabled collector doesn't run: the
  scheduler, the `collector.triggers` consumer and the snapshot sweep skip
  it, and `oxplow.collector.sync` refuses it with its reason (`Invalid` at
  `/id`). A disabled effect stops reacting (`effect.triggers` skips it).
  For an effect only a `failed` reaction counts — an interrupted one
  included, a lost race to another delivery not; `skipped` and `proposed`
  never do, and a reaction whose commands ran starts the count over. Nothing cascades: a lens over its view still runs, carrying a
  warning (`LensRun.warnings`) that its rows aren't refreshing.
- **Repairing.** The `contribution.repair` pump consumer files a work item on
  the active work-items provider, as the system: title `Repair <extension>
  <contribution>: <reason>`, body the repair prompt (`contribution_repair::render`,
  golden `crates/oxplow-app/tests/fixtures/repair-prompt.md`: what failed,
  its intent and declaration, its recent failures, what `extension check`
  reports, its intent examples, `engine:` against the running oxplow, and
  what to do). It goes to the active tracker like every item (tsk1058).
  When that tracker can't take it — not running, or the very contribution
  just disabled (tsk714) — the delivery fails naming it (retried, then a
  dead letter in Delivery, beside the tracker's own alert); nothing is
  filed in oxplow's list instead. A later
  disable while the item is open comments on it;
  once it's done or canceled the next disable files a new one
  (`contribution_health.repair_item`, `v_contribution_health.repair_item` while open).
  oxplow never sends it to an agent.
- **Enabling.** Only a person: `oxplow.contribution.enable { extension, kind,
  contribution }` (human-only). It names the kind — a provider and a
  collector may share an id; there must be a failed one of that kind (or
  a provider instance the registry knows).
- **In the app (C3).** Settings → Extensions shows one health line per
  contribution under its extension (`contributionHealth.ts`, live over
  `v_contribution_health`): `OK` with its average time, a missed schedule and
  undelivered events; `Failing (N in a row): <error>`; or `Disabled:
  <reason>` with **Enable Again** (`oxplow.contribution.enable` through
  `personCommands`) and, while its repair item is open, **Repair with
  the Agent** — one line in the agent's input, `Repair the extension
  described in [oxplow ref <repair item>] — read it first.`, never sent
  (the repair brief is the item's body; `no-agent-input-automation.test.ts`
  pins the button as its only caller). Settings → Data → **Delivery**
  lists the pending dead letters (`delivery.ts`, live over
  `v_event_dead_letter`) with Retry and Discard (`InlineConfirm`); the
  rail's Alerts shows "N events couldn't be delivered" while any wait.
- **For agents.** `questions/extensions.yaml` and the extension skill's
  "Health and repair" section: read `v_contribution_health`,
  `v_collector_run` and `v_event_dead_letter`, read the repair item
  first, fix, check, test, `oxplow.collector.sync` — and ask the person to
  Enable Again (an agent can't run `oxplow.contribution.enable`).

## Implementations

An extension declares capability implementations (stable,
`implementations:`; `extensions/implementations.rs`):

```yaml
implementations:
  - { capability: work_items, id: oxplow, entry: "oxplow:tasks" }
```

- `capability` is a choosable one, or one many implementations serve
  (`oxplow_domain::capability`: `agent_harness`, `acp_adapter`,
  `ai_provider` — every one declared is active); `id` is what
  `activeProviders` names, or a many-capability's key (an agent session's
  `harness`, a session's `acp_agent`) — lowercase, never `none`.
- `config:` configures the built-in, checked at load against its schema
  (`BuiltIn.config_schema`, `capability::check_config`; a built-in with
  none refuses any config): an ACP adapter's `{ command, args?, env?,
  systemPrompt?: meta|prompt }`.
- `entry` names a built-in in core's standard library
  (`capabilities::BUILT_INS`: `oxplow:tasks`, `oxplow:commit-or-switch`,
  `oxplow:snapshots`, `oxplow:snapshot-hashes`, `oxplow:wiki`), the way a collector names
  `oxplow:junit`. A
  built-in's features are core's table's — it's core's code — never the
  manifest's. A built-in whose items' refs carry a provider id
  (`BuiltIn.provider`: `oxplow:tasks` is `oxplow`) is declared under that
  id and no other.
- **A process implements a capability through `providers:`**, not
  here: a capability whose spec carries a provider contract (the work
  list, the effort policy, an agent harness) may be an external
  provider's instance — the work list and the effort policy chosen by
  `activeProviders` like a built-in, a harness registered beside the
  built-ins under its instance id, as many as run
  ([providers.md](./providers.md) "What a provider may implement").
- **An `ai_provider` or an `effort_policy` may be a script** instead: an
  `entry` that isn't `oxplow:<name>` names a `.star` file of the extension
  (`implementations::script_decl`, `Source::Script`). Nothing else is a
  script.
  - An AI provider's takes `config: { baseUrl?, ops? }` (the contract,
    consent and the shipped scripts are [ai-providers.md](./ai-providers.md)
    "Scripted providers"); it runs once a person approves it
    (`ProgramKind::AiProvider`).
  - An effort policy's is effects-shaped — `transform({event})` →
    `{ commands: [{ name, input }] } | { skip: "why" }` — takes no config,
    and names the scopes it calls in the entry's `needs:` (only what a
    script can, `scope_calls::CALLABLE`; checked at load, with the script
    defining `transform`). `needs` is refused anywhere else. It is a
    program a person approves (`ProgramKind::EffortPolicy`, key
    `effort_policy:<ext>/<id>`, `exec_consent::effort_policy_program`):
    the hash covers the script and every file of its extension, as an
    effect's does, and Settings → Data → Programs lists it with what it
    reads and offers the script to read.
- `oxplow-foundation` declares the agent harnesses (`oxplow:claude-code`,
  `oxplow:codex-cli`, `oxplow:opencode`, `oxplow:acp`), the ACP agents'
  programs (`oxplow:acp-adapter`, one declaration per preset) and the AI
  providers, as scripts (`providers/anthropic.star`,
  `openai_compatible.star` — twice, `openai` with OpenAI's `baseUrl` —
  `openrouter.star`, `typesafe.star`); foundation is required, so
  `CapabilityRegistry::new` registers nothing for them. The harnesses are
  registered by declaration (`harnesses::prepare_built_ins`, published by the
  generation → `Services.harnesses`, an `oxplow_domain::agent::registry::HarnessRegistry`).
- A required capability's default is core's own: `CapabilityRegistry::new`
  registers it (snapshots' `oxplow`, `oxplow:snapshots`), so it's there
  whatever is disabled, and a manifest declaring that id is an error.
- `oxplow-bundled` declares the optional defaults (the work list, the
  effort policy and knowledge's wiki, `oxplow:wiki`). Disabled, nothing
  declares them and all three resolve to none (`capabilities::CapabilityRegistry::resolve`,
  `.context/work-tracking.md`). It also offers snapshots' `hashes`
  (`oxplow:snapshot-hashes`, "Track changes only": no `contents`); a
  chosen `hashes` with bundled disabled falls back to core's `oxplow`.

**Declared needs.** A lens or an advisory may declare `needs:
[work_items, snapshots.contents]` — capabilities, or one of a
capability's features (`oxplow_domain::capability::check_need`; an
unknown one is a load error). A lens run checks them against what's
active (`LensContext.active`, a `capabilities::Active` snapshot that
`lens_context` fills): unmet, it doesn't run and returns `inactive: {
needs, message }` ("Needs: Work list (choose one in Settings → Capabilities)."),
which the lens view shows instead of its empty state. An advisory whose
needs aren't met doesn't run. A context built without `Services` (param
checks) checks nothing. A capability's none declares every feature (it
takes everything, keeping nothing), so a feature need is met by none and
only a need of the capability itself (`work_items`) isn't. `oxplow-bundled`
declares no `work_items` need: with no list its task lenses, hint and
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
- **Offered** (`capabilities::agent_text`): from consented, enabled
  extensions, when its `needs:` are met and, if an implementation lists
  it in its `skills:`, while that implementation is the active one. A name
  another extension already took is left out (logged).
- **Installed.** Each spawn writes what's offered into the runtime
  (each harness's `launch`, `crates/oxplow-harnesses`); boot, an
  extension change and a `capability.switched` rewrite the runtimes
  already on disk (`refresh_agent_text` → each harness's `refresh_text`). A skill folder oxplow wrote
  carries a `.oxplow` marker, so one no longer offered is removed and a
  person's own stay; the Claude `commands/` folder is oxplow's whole.
  ACP agents get the offered set's index and `get_skill`.
- `oxplow-bundled` gives the `work-items` skill (any work list) and the
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
  the person dismisses one (`oxplow.hint.dismiss`, which an agent can't run).
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
- **Consent.** A shared extension's advisories (committed by a
  teammate, or installed from git) speak into the agent's context, so they
  run only once a person approved them. `exec_consent::advisory_program`
  turns an extension's advisories into a program (kind `advisories`), with
  one arg per advisory: its id, trigger, repeat rule, heading and query.
  Settings → Data → Programs lists them, and any change needs approving
  again. `advisories::consented` filters before running; bundled
  extensions aren't gated.
- `for_thread(svc, thread, on, cause)` runs them for a thread, reading
  the main worktree's extensions (whatever the thread's stream) and
  binding the thread's stream into the scope.
- `validate_extension` dry-runs each advisory with every param NULL and
  checks it returns `message` (and `key` for `once_per: row`).
- oxplow-bundled ships hints on outcomes: `large-uncommitted` (turn-end,
  once per effort: an open effort holding 15 or more files — a commit
  landing its work would have closed it) and `landed-in-progress`
  (turn-end, to the person, once per item: an item still in progress
  though a commit landed its work and nothing is open on it); and two
  metric ones, `metric-deltas` and `threshold-crossed` (see
  [metrics.md](./metrics.md)). **Bundled advisories set no coding
  requirements** (decided 2026-10-07): they say what oxplow observed, never
  how a project should code — no coverage target, no "add tests" — and
  a project that wants one writes its own advisory (the `coverage-target`
  nudge was removed for this). Advisories read
  stored views (`v_effort_metric_delta`, `v_effort_observation`), never the
  engine directly.
- The report-less-run nudge stays in core: it's about collection hygiene,
  not an instrument.

## Agents: the MCP surface

Everything a human can see or build, an agent can see or build too, over
MCP. That includes whatever **extensions** add: an extension's sources,
entities, dimensions, metrics and lenses show up in the same generic tools
as core's. Extensions never add their own MCP tools. That keeps the agent's
tool list stable no matter how many extensions are installed.

**Understanding the semantic layer**

- `query_sql`: read-only SQL over the published models. `v_model` /
  `v_model_column` / `v_model_lineage` describe every view (core `v_*` and
  extension `v_<ext>_<entity>`) with its column docs and inputs;
  `v_dimension`, `v_metric_spec` and `v_measure` list the dimensions,
  metrics and measures, extension-declared ones included, each with its
  scope.
- A metric's numbers are SQL too: `SELECT bucket, MEASURE('<key>') FROM
  metric_grid('day'[, '<dim>'])` — sliced by a dimension, in any bucket.
  Changing metrics is the `metric.*` commands (`run_command`).

**Working with lenses**

- `list_lenses`: every lens (`<ext>/<slug>` id, title, description, viz,
  params).
- `get_lens(id)`: the lens definition: query, params with defaults, viz,
  columns/links and the file it lives in.
- `run_lens(id, params?, format?)`: **what the UI shows** for those
  params, as its text rendering by default — `LensText { lens, title,
  params, columns, rowCount, truncated, reads, alert, text }`, no rows
  (the text carries them) — or, with `format: "json"`, the full
  `LensRun` with its rows.
- **Text rendering** (design rule 17, `crates/oxplow-app/src/lens_text.rs`,
  P6.A1): one renderer per kit component — a table or list is a markdown
  table of the displayed columns (the UI's `displayColumns` rule); a
  number or markdown lens is its first cell; a bar chart is its labels
  and values with the total; a line chart pivots to one column per
  series with each series' total; a treemap lists labels by size,
  largest first; a grid renders its children in order under `###`
  titles; a tree is an indented list; a timeline is `at — label (ref)`
  lines oldest first; a detail is `**label**: value` lines; steps are a
  numbered `[x]`/`[>]`/`[!]`/`[ ]` checklist; hunks are each file's
  unified diff between its two revisions (up to 20 files, 8 KB each).
  What a component needs beyond its rows — a grid's child runs, the
  hunks' diffs — is `lens_text::resolve`d first (`text_of`). Tables
  stop at 50 rows and say how many more; an empty result says the
  lens's `empty` text. It's what `run_lens`, `get_open_page` and `copy`
  return, so an agent reads a lens the way the person sees it.
- `get_open_page(thread_id)` **(current)**: what the human has open in
  that thread, for any page kind (`task:42`, `file:…`, `lens:…`). For a
  lens it adds `lens`, the lens re-run with the human's *current*
  params as its text rendering, so "look at what I'm looking at" works.
  - The UI reports the active page (`report_open_page`, UI-only) together
    with whatever that page published to `tabs/openPageDetail.ts`. The
    lens page publishes `{lensId, params}`.
  - The state is held in memory on the thread runtime
    (`ThreadRuntimeRegistry::open_page`).
  - A hidden-but-mounted page can't overwrite the report, because only
    the active page's detail is sent.
- **Row actions (current):** right-click a lens row → "Ask About This"
  (P6.D1): `[oxplow ref <ref>]` for the first ref the row links to, else
  `[oxplow lens <id> row: col=value, …]` (`rowAsk`).
- **The prompt catalog (current):** what a person can ask.
  - Core's are the answerability questions, by area
    (`crates/oxplow-agent-text/assets/questions/*.yaml`,
    `oxplow_agent_text::answerability_prompts` → `AnswerabilityPrompt {
    area, prompt, about }`; `PromptSource::Area` in the catalog); a question with `about: <ref
    kind>` is phrased with "this" and is offered on pages for that kind
    (its `reaches` keeps concrete fixture values for the answerability
    check, which also checks `about` names a registered kind).
  - An extension's are `intent.prompts: [{ prompt, about? }]` in its
    manifest (checked: non-empty, **one line** — a pasted line break is
    Enter in the terminal, so a multi-line prompt would send itself —
    and `about` a registered ref kind).
  - `prompt_catalog::prompt_catalog(extensions)` merges them (core first,
    then enabled extensions by name); RPC `prompt_catalog { stream_id }`
    (UI only) serves it.
  - The UI: the **Catalog** page (`page:catalog`, `pages/CatalogPage.tsx`:
    prompts by source, `v_model` by owner, `oxplow.config.list_keys`), the nav
    bar's Ask menu (suggested prompts for the page's ref kind), and
    `EmptyState`'s prompts (usability.md → "Empty states").
- **No launcher entries.** The manifest's old `launcher:` list (a ref,
  a command or a prompt) is gone: a page is a `pages:` entry or a lens; a
  command — one that runs another with fixed input, or puts a prompt in
  the agent's input (over `agent_input.write` `draft`) — is a command the
  extension declares with a `ui`, offered like oxplow's own
  (`commandOffers`). The launcher (`components/extensionLauncher.ts`
  `launcherPages`) lists lenses and `pages:`.
- **Lens actions (current, P6.B1):** `actions:` are commands, run by an
  agent with `run_lens_action` as the lens acting for it (see "Actions
  are commands").

**Building**

- Agents author extensions and lenses **by editing files** under
  `oxplow/extensions/<name>/` with their normal Edit tool, under the usual
  write guard. The loader hot-reloads.
- **No scaffold tools (decided).** A scaffold tool would write project
  files outside the write guard and the caller's worktree. The skill carries the templates instead, and humans have
  Save as Lens in Explore Data. The same rule holds for metrics:
  the `oxplow.metric.scaffold` command returns the metric extension's files for
  the agent to write itself.
- `validate_extension(name)` returns load errors, schema errors and a dry
  run of every lens query, so the agent can check its work without the UI.
- `list_extensions`, `list_collectors`, `run_collector(owner, id)`
  (never approves).
- **Worktree streams: preview, don't run (tsk377).** Collected data is
  project-wide (one `ext__<ext>__<entity>` table), so a run (the
  `oxplow.collector.sync` command over `CollectorRunner::sync`, from any caller)
  always runs the **primary** worktree's copy. An agent
  writing a collector in a worktree stream checks it with MCP
  `preview_collector(owner, id, stream_id)`: `collector_runner::
  preview_collector` runs that worktree's version through the same
  `produce` step (same consent: an exec collector needs a person's
  approval of that exact hash, and so does a Starlark one that calls a
  model; other derived collectors don't) and returns the coerced
  rows per entity (first 50, plus totals), storing nothing and recording
  no run.

**Teaching the agent**

An `oxplow-extension` skill (shipped in `crates/oxplow-agent-text/assets`)
teaches the format, the `v_*` contract and the loop "read `v_model` →
query_sql → write files → validate_extension → run_lens". "Improve with Agent" on a lens pastes
`[oxplow lens <ext>/<slug>]` plus its params into the agent's context.

**Getting newcomers there (tsk373).** "New Lens with Your Agent…"
(`lens.newWithAgent`, a Tasks search command in the launcher; needs a
thread) puts `NEW_LENS_PROMPT` (`lens/lensModel.ts`) into the agent's
input via `insertIntoAgent`. It never sends; the person finishes the
sentence. The Extensions empty state and Explore Data's intro offer the
same button, and getting-started's "Your first stream" has a "Build your
own view" step.

## Example extensions (`examples/extensions/`)

Each loads clean (`documented_examples_load_without_errors`):

- **`github`** — an `exec` collector bringing pull requests in as the
  entity `v_github_pr`, with a lens joining them to streams and tasks;
  and (P10) a pull request as a **ref kind**, `github_pr` — model
  `pull_request` (`ref`, `title`, `body`; it resolves and is searchable),
  page `pr` (the `pr` lens, given `?ref=`), `[[pr:12]]`. `crates/oxplow-
  sdk/tests/examples.rs` checks and tests it and opens a pull request.

## The `oxplow-bundled` extension

Everything oxplow ships that measures, visualizes or reviews is this one
extension (`extensions/oxplow-bundled/`): its namespace is
`oxplow_bundled` (commands, event types, measures, collectors) and its
models publish as `v_oxplow_bundled_*`.

What moves out of core, and what it becomes:

| Today (core) | Becomes |
|---|---|
| Planning / Review / Quality dashboards | `grid` lenses (**done**: `planning`, `review`, `quality`) |
| Code-quality runner, dup scan, FindingPage, DuplicateBlockPage | **done:** the `findings` / `duplicate-blocks` lenses; the dup scan runs in core's change analysis; `DuplicateBlockPage` stays core as the compare page |
| Change-analysis cards (treemap, look-here-first, functions, co-change, zones) | **done:** the `change-review` grid in the `effort.review.details` / `vcs.commit.details` / `vcs.status.details` slots; core keeps a changed-files tree (`ChangedFilesTree`, `useChangedFiles`) |
| Fact collectors (`oxplow/collectors/*.star`, idiom `.star`; gauges until P7.B3) | extensions can declare fact collectors (tsk311); the built-in catalog stays core as the opt-in standard library |
| Metric-threshold nudges | **done:** the `threshold-crossed` advisory (with `metric-deltas`) |
| Usage / page analytics / token pages, `ThreadTokenTotal`, `EffortTokenUsage` | **done:** the `usage` grid; `task-tokens` (`work_item.detail.body` slot) and `thread-tokens` (`thread.plan.header` slot) |
| Git History's commit graph | **done:** the `history-graph` component, replacing core's plain `CommitList` (`vcs.history.graph`) |
| Local history dashboard | stays core (snapshots are substrate); the `recent-snapshots` lens in `review` covers the at-a-glance view |
| Effort metrics block, effort coverage page, tests-run and nudge blocks | **done:** `effort.review.details` slot lenses `effort-tests` (grid: coverage, untested files, test runs, failed tests, analysis findings), `effort-metric-deltas`, `effort-nudges` |
| Rail panels: Comments, Work, Uncommitted, Go To | **done:** Go To (`go-to`, the thread's bookmarks then its recent or most visited pages through the `mode` choice param, `go-to-bookmarks` collapsed — needing bookmarks as project data (`v_bookmark`), lens param `options` and page-kind icons; the rail has no core panels left); Comments (`comments`, a panel `open`ing the inbox) and Work (`work` grouped over the `thread_work` model, which also lists the thread's unlinked efforts — open under In progress, closed under Finished, whose header opens the `thread-activity` lens of the thread's turns by effort — `work-line` collapsed, `work-count`, the `clear_finished` command + `finished_cleared` model); and Uncommitted (`uncommitted`, a tree over the working change's stage-one file list, which change analysis now keeps current on every move). Only the UI moves: stores, commands and views stay core. What Work needed and lenses lacked became general features, listed under "Added for extraction" — the pattern for any panel an extension builds |

**Owns what reads tables (P7.B5).** Derived data computed only from
core's rows is the extension's, as SQL: the "look here first" score is
its model `change_interest` (`v_oxplow_bundled_change_interest`, over
`ref('change_file')` and `ref('change_function')`), which the
`change-look-here` lens reads; co-change surprises are its models
`co_change_pair` (`materialize: { every: 1h }` over the commit index —
"the last 180 days" moves with the clock) and
`change_co_change`, which the `change-co-change` lens reads — and the
`file-co-change` lens ("Usually Changes With", mounted at
`diff.file.header`, P9.A2) reads `co_change_pair` for the file a diff
shows; an
effort's churn is its fact collector `oxplow_bundled.effort_churn`
(`on: [effort.finished]`, `after: [change.analyze]`, reading the
effort's change files) recording `oxplow_bundled.effort_churn_lines`.
Full-tree duplicated lines (tsk388) read the tree, not tables, so they
are a core built-in collector, `oxplow.duplicate_lines`, over the
public `duplicate_blocks(min_lines)` builtin — what any extension could
write. What needs two revisions' trees — files, functions, imports, test
signals, per-function churn, the scoped duplicate scan — stays in core's
change analysis.

**Stays in core, deliberately simple:** a basic **metrics explorer** and
**simple dashboards** (Nathan, 2026-09-27). The base version must let
people see what metrics exist and pin a few to a dashboard as a starting
point, a bit like Metabase. It doesn't need to be fancy. The explorer
browses the semantic-layer catalog (entities, measures, metrics), shows a
table or simple chart sliced by a dimension, and can save the result as a
lens or pin it to a dashboard. Dashboards are a plain grid of pinned
metrics and lenses. Tracked in the "basic metrics explorer" task.
**Done (tsk309):** Metrics is a catalog (search, enabled/all, range, branch,
sparkline + latest value); Metric Detail is trend + recordings + stats +
enable + Add to dashboard; dashboard tiles are line / number metrics, text
and lenses under a range/branch filter ([dashboards.md](./dashboards.md)).

**Also stays in core** (it is substrate other features need): snapshots,
collection ingest, attribution, token ingest, page visits (the rail and
launcher use them), the fact store and engine, the Go To page and
bookmarks (`v_bookmark`; the rail's Go To pane is the extension's), the Git
dashboard. diff-view shrinks to title + file list + diff + the
`effort.review.details` slot; WorkItemPage keeps the `work_item.detail.body` slot.

New in the extension: the **effort review packet**, all live exception
lenses:

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
and enabling it restores the old pages' behavior as lenses.

## Added for extraction

Features added to core because the extraction needed them (tsk280),
available to every extension:

- `bar`, `line`, `treemap` and `grid` viz, with `chart` and `children`.
- `commit` and `metric` link kinds; `file` links with `line`. (A `task`
  kind came and went: a work item links as `page`, by its ref.)
- `work_item.detail.body` and `thread.plan.header` slots, with slot params checked at load.
- Lens `launcher.category` and `hidden`.
- Lens `actions:` as commands (P6.B1; the tsk329 registry retired).
- Disabling extensions per project.
- Advisories: the generic nudge primitive (see "Advisories").
- Lens tabs carry params: `lens:<ext>/<slug>?k=v` (`lensRef(id, params)`),
  so a slot lens's heading opens its page with the slot's values, and
  history and bookmarks keep them.
- Panel `collapsed:` — a lens shown as a collapsed panel's summary
  (for the Work panel).
- Panel `count:` — a lens whose row count (or `number`) is the panel's
  header count without raising an alert.
- Column `icon:` naming any page kind draws that page's icon.
- Lens param `options:` — a choice, shown as a toggle in a panel's
  header and a select on a lens page (for Go To's Recent / Most
  visited).
- Lens `group: { by, link? }` on `list` / `table` — rows under linked
  sub-headings, also in compact panels — and actions with `group:` in a
  group's heading.
- Column `icon:` / `tone:` from fixed vocabularies, and lens `emphasis:` /
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
provide OS confinement for an approved external program.

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
candidate with model views, vocabulary, providers and UI metadata through the
generation coordinator. Runtime lookup uses the active catalog rather than
promoting current filesystem declarations. State hosts still enforce their
namespace and revision checks.

## Database generation candidates

`VocabularyService::prepare` freezes enabled vocabulary declarations and the
host work item recognizer from a CatalogSnapshot without writing or swapping.
Enabled load errors reject preparation. `PreparedVocabulary::replace_tx` uses
a savepoint and rejects the complete set on any event/ref/subscription or
recognizer diagnostic, restoring event contracts, retention and ref-kind rows.
A changed recorded event schema and a shared wikilink prefix are candidate
errors; a candidate never publishes a partial result.

`PreparedExtensionDatabase` captures models, vocabulary and enabled contribution
descriptors from the same snapshot, and every entity the enabled collectors
declare: its transaction first creates each entity that has no view yet, empty
(`collector_store::ensure_entities_tx`; one that exists keeps its rows — they
are the collector's), so a model over an entity compiles before the collector
first runs. Without it an extension whose model reads its own collector's
entity is held back forever, its collector never active to fill the entity
(`the_documented_examples_activate`: the github example). Its transaction helper calls strict model
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
capability, command, provider, model and vocabulary boot writers have been
retired from this path; config/entity-triggered model work is routed through the
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

## Prepared native runtime adapters

`PreparedRuntimeAdapters` captures one CatalogSnapshot and constructs declared
work-item, harness, effort-policy, ACP, knowledge, snapshot and model-provider
maps without changing running registries. It rejects enabled load errors and
strict ACP/AI factory failures before any map publication. Script/gate objects
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
A failure later in staging that names one extension — a model whose SQL
doesn't compile (checked inside the publication transaction, which rolls
back), a provider or capability that can't stage — holds that extension the
same way (`extension_catalog::Failed`) and the candidate is staged again; if
its last working version fails too, it is left out. A failure that names no
held-able extension (the host, a required bundled one) rejects the candidate.
The active catalog is separate from raw candidate loads. Its
`source_generation` preserves the raw fingerprint, while the core baseline has a
distinct `core:<source_generation>` runtime identity. Runtime catalog reads use
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
Precommit rejection retains the previous active snapshot. A panic during commit
or publication leaves this boundary unavailable until host recovery. Cancelling
the async caller does not release the worker's writer while it can still commit.

`GenerationPin` is host-only and conveys no authority. RunContext can retain an
opaque pin across spawned host work; child contexts inherit it and fresh jobs do
not. Nested reader scopes reuse their pin so a waiting fair-lock writer cannot
deadlock a parent query. SQL and command entrypoints pin before query materialize
or command lookup; lens/effect/event-consumer paths use the same reader scope, and
host workers carry the concrete pin in RunContext. Task-local scopes alone do not
propagate into spawned tasks. Provider host-call registration is cancellation-safe
across candidate publication. Native/background and synchronous metadata
entrypoints still need a complete audit. Metric declaration rows now join the
publication transaction; state-value recapture remains a post-publication reader. The renderer receives the published metadata change signal;
raw filesystem changes continue to trigger candidate preparation. Until the first
generation is activated (boot's `initialize`, before ingress opens), no reader is
served: the generation is the only publisher of commands, models, vocabulary,
providers and capabilities.

Custom component preparation reads immutable bundle bytes on a blocking worker,
with at most 64 bundles and 128 MiB in a candidate. Publication swaps the whole
active bundle map without rereading files. Active bytes remain available independently
of the sixteen-entry history used by previous frames. A retained frame from a
different activated generation must reload before invoking a query or command;
a bundle of another root (a stream's lens) is built from that root's files. This check requires the owning
runtime reader scope for atomicity with command or query execution.

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

Collector scheduling and fact-collector lookup retain a reader before selecting
declarations. A carried RunContext pin can re-enter a reader on another task
without acquiring a second fair-lock reader behind a waiting writer. Entity-run
notifications compare the current entity view/version/SQL/column/key contracts
with the signature captured in the publication transaction. Data-only runs do
not restart providers; changed entity schemas request one coherent candidate.
Provider initial sync and entity-state metric recapture do not themselves emit
collector-run completion events.

## Jobs

A **job** is model work an extension asks for that runs outside any
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
    replay: never                     # or idempotent
commands:
  - { name: review.ask, summary: …, scope: jobs.write, op: enqueue, invokers: { human: true, agent: true, lens: true } }
  - { name: review.cancel, summary: …, scope: jobs.write, op: cancel, invokers: { human: true, agent: true, lens: true } }
  - { name: review.result, summary: …, scope: jobs.read, op: snapshot, invokers: { human: true, agent: true, lens: true } }
  - name: evidence.lookup             # a tool: reads, admits a job, says its input
    summary: The TODOs in one file.
    entry: commands/evidence_lookup.star
    needs: [sql.read]
    input_schema: { type: object, required: [path], properties: { path: { type: string } } }
    access: read
    invokers: { human: false, agent: false, lens: false, job: true }
```

- **A tool** is one of the extension's own commands that admits a job
  (`invokers.job`), only reads (`access: read`) and has an
  `input_schema`; anything else is a load error. The model calls it by
  its local name with `.` as `_` (`evidence_lookup`: model APIs allow no
  dots); its description is the command's `summary`, its parameters the
  schema. `query_sql` is reserved: with `sql.read` the model gets it, a
  read-only query over the published models (as MCP `query_sql`; 200
  rows). Core's commands admit no job, so a model reads oxplow through
  `query_sql` and never writes: a job's writes come from an effect on
  its end.
- **Approval.** A job is a program (Settings → Data → Programs: "Job
  `<ext>/<id>`", the tools its model may call). Its hash covers
  `extension.yaml`, so a changed declaration — a new tool — asks again.
  Until approved its intent waits, queued; nothing is sent. Every model
  call also needs its AI provider's approval.
- **Enqueue** (`jobs.write/enqueue`): `{ job, intentKey, request }`,
  `request` `{ schemaVersion: 1, role, messages, tools? }` — never a
  model, endpoint or credential: the role's model is bound when it runs.
  A job that declares `tools` takes none in its request. One intent key
  is one job (asking again returns it). At most 1024 queued or running
  in a project, 256 per extension. The request stays private (it isn't
  in the audit or any model).
- **Running.** The scheduler (four at a time, polling each second)
  claims a queued job, resolves its authority afresh — the declaration
  and approval as they are now, the thread's writer gate — and runs it
  as `Actor::Job` (`job:<id>:<attempt>:<fence>`) on a run of its own,
  linked to the run that asked. Each model round streams on the role
  (`AiCompute::stream_typed`, a recorded computation: the same request
  reads its recorded answer, replayed at once). What comes is the
  attempt's **output** (`extension_job_output`, `v_job_output`): text
  coalesced into a row every 100 ms (or 4 KiB), each tool call and what
  it answered, each round's usage. Each write is its own short
  transaction — none is held while the model writes — and the table
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
  `stopped: cancelled`. `jobs.read/snapshot` returns `{ id, job,
  version, state, attempt, cancellationRequested, text, toolCalls,
  result }`, `text` being what the current attempt has streamed so far.
- **Cancel.** `jobs.write/cancel` (the extension's own jobs) or
  `oxplow.job.cancel` (a person's, any job: `invokers` human and lens).
  A queued job ends at once. A running one has its run cancelled: the
  model's connection is dropped within 100 ms
  (`oxplow_ai::client::unless_cancelled`), the attempt closes
  `cancelled` with what had come, and the call's `v_ai_call` row is
  `error: cancelled` with that partial reply. Revoking the job (its
  approval, its extension disabled) stops it the same way.
- **Limits.** Past its `timeout` a job fails (`timed out after Ns`),
  keeping its output; the dispatcher stops a worker still running 15 s
  later. The lease is 30 s, renewed every 10 s: an attempt whose lease
  ran out (oxplow stopped) is `interrupted` when recovery finds it.
  Nothing replays an interrupted job yet, `idempotent` or not.
  A queued job survives a restart and runs after it; one that was
  running is `interrupted` once its lease is out. A running job whose
  declaration changes (a new generation) or whose approval is revoked
  is cancelled with what it had, never completed under the new
  generation. An extension has at most 256 queued or running jobs (a
  repeat of an intent it already has is not a new one). No database
  transaction is held while a model streams. These are pinned in
  `extension_job_hardening_tests.rs`.
- **Its end** is the event `contribution.job.completed@1 { job_id,
  contribution, version, attempt, fence, outcome: succeeded | failed |
  cancelled | interrupted }`, logged in the transaction that ended it
  (however it ended: completion, cancel, recovery) with the anchors and
  cause the job was asked with. It carries no request or result: an
  effect reads the result with `scope("jobs.read", { id })` and writes
  what it says (the triage example's `publish-summary`).
- **Seeing jobs.** `v_job` (state, attempt, rounds, tool calls, tokens
  so far) and `v_job_output` are published models, so a lens or
  component over them follows a job live. oxplow-bundled's **Jobs** lens
  (Activity) lists the running and the last day's, with Cancel; a row
  opens its **Job Output**.
- **What a job can't do**: drive an agent session, ask a person (a
  command that needs confirmation is refused, never proposed), write
  (its tools read), run core's commands, or call a model its approval
  and the AI provider's don't allow.

## Host-owned secret references

`oxplow-domain/src/extension_secret.rs` defines a strict version-1 identity:
`{schemaVersion:1,id:<canonical UUIDv7>}`. It contains no owner, path, keychain
account, secret bytes or authority. References may appear in state/config;
values remain in the machine keychain under the primary project and current
owning extension. Copying team configuration cannot approve use or read another
namespace. Disabled/uninstalled owners cannot save or inspect a reference;
explicit human revocation can delete a retained keychain value after uninstall.

`ExtensionSecrets` has human settings ingress only: save/rotate returns a
reference, inspection returns presence, revocation returns no value. The RPC
methods are `save_extension_secret`, `extension_secret_is_set` and
`revoke_extension_secret`; no native command, script service or MCP route exists.
Secret bytes and keychain error details are never returned or audited. Values
are bounded to 16 KiB. This backend ingress has no new settings form yet.

The first trusted consumer is a remote MCP HTTP provider using the shipped
adapter. Its bearer credential must be named `OXPLOW_MCP_BEARER` and carry
`secretRef`; references are refused for package executables, stdio servers,
OAuth credentials and other environment variable names. The fixed name avoids
process environment control variables. The reference identity, endpoint and
declaration participate in program approval. The host checks exact current
owner/consent before and after keychain resolution and gives bytes only to the
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

## Interrupted installation recovery

`extensions/candidates.rs` owns a shared in-process install/recovery lock and a
bounded, versioned marker inside hidden `.ext-publish-*` staging. Publication
syncs copied regular files and directory entries before moving the old target.
It restores or retains the prior copy on failure. The two directory renames
are filesystem staging; they do not themselves publish runtime authority.

`Services::boot` recovers before loading configuration or constructing the
catalog. Recovery scans at most 4096 parent entries and 128 staging folders,
reads markers up to 256 bytes, checks names and plain directory/file shapes and
refuses repo-controlled symlink parents. A missing target with a known prior
copy is restored and synced. An unpublished first-install candidate is discarded,
never activated automatically. Unknown entries, malformed markers and ambiguous
target-plus-backup states are retained with a fixed warning, so recovery cannot
delete the only identified prior copy. Unix uses directory sync; other platforms
retain ordinary filesystem semantics. The lock is process-local; adversarial
cross-process filesystem races and crash injection remain validation gaps.
