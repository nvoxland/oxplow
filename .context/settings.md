# Unified settings

Core and extensions use one registry and one storage service. Settings APIs
never select a storage implementation: a definition's `secret` flag does.

## Definitions and resolution

Each definition has a fully qualified key, title, description, category, JSON
Schema, optional default, UI hints, ordering, and restart requirement. The host
assigns ownership. Categories are independent of ownership. Core definitions
come from `oxplow-config/src/core_settings.rs`; extension declarations are
translated by `extension/extension_settings.rs`. Fixed object properties become
independent settings; arrays, dynamic maps and explicit `x-atomic` objects are
one setting. Prefer scalar options wherever they can be edited independently.
Extension schema annotations are `title`, `description`, `x-category`, `x-ui`,
`x-order`, `x-secret`, `x-restart-required` and `x-atomic`.

An effective setting is the first value present at **local → project → global →
default**. Objects and arrays replace whole values; nothing merges across levels.
Every setting supports every level. `unset` removes only the chosen override.
Absent values are distinct from an explicit null, when a schema permits null.

Ordinary overrides are flat maps of registered keys:

- local: `.oxplow/local.yaml`, ignored by Git;
- project: `.oxplow/project.yaml`, shared through Git;
- global: `global_config_dir()/global.yaml`, shared across the person's projects.

`OXPLOW_HOME` overrides the global directory directly. Its default on macOS is
`~/Library/Application Support/net.voxland.oxplow/`. Both project files are rooted
in the daemon's main worktree, never a parent directory or another stream's copy.
The core loader projects resolved settings into `OxplowConfig`; that typed struct
is a runtime view, not another storage layer. Independent processes lock a stable
sidecar, re-read under the lock and atomically replace all keys in one write,
preserving unrelated values and comments.

Capability choices are independently registered scalar settings, generated from
`capability::choosable()`: `oxplow.activeProviders.<capability>` (currently
`snapshots`). The old aggregate `oxplow.activeProviders` map is not registered.
An extension supplies an implementation value, not new capability-definition
keys; unknown or non-choosable capabilities are refused. The typed
`active_providers` map is assembled from resolved scalars for runtime consumers.
Its origin metadata distinguishes local, project, global and default choices.
`capability.choose` resets just one scalar override, exposing inheritance; it
never writes an empty map that masks a project's choices.

## APIs and consumers

`oxplow.config.get {key, level?}` returns value, presence, secret metadata and
origin. Without a level it resolves inheritance; with a level it reads only that
override. `oxplow.config.set {key, value, level}` and `.unset {key, level}` require
an explicit `local`, `project` or `global`. `list_keys` returns definitions from
the registry. Values are validated by registered offline schemas before storage;
core writes also validate the typed runtime projection before changing files,
both at the selected level and with higher overrides applied. An override
invalidated by an extension schema change can still be replaced or removed;
undo is withheld when it would restore that invalid value.

Extension scripts declare `needs: [config.read]` and call
`scope("config.read", {"key": "acme.limit"})`. Their secret reads and all composed config writes are limited to
the owning namespace, including refusal of core settings and core credentials.
Scripts write by composing the ordinary config commands.
The retained `extension_config.read` snapshot is a nested read projection over
the same settings, not separate storage; it has values and origins, no stamps,
CAS or migration protocol. SQL `config('/pointer')` and conditions receive a
public projection that omits secrets before any secure read. Configuration-aware
extensions republish when their resolved public settings change.

`v_config_value` is the registry-backed, redacted settings inventory, including
labels, categories, schemas, origin and presence. Metric/dimension definitions
are catalog contributions and are not preferences in that inventory.
`v_config_key` lists registered core and extension keys with docs and schemas.
Settings uses a registry-driven category index and standard form editors; operational tools live on linked management pages.
The `config_value` dataset contract is version 5. For ordinary settings the inventory also exposes `local_value`, `project_value`
and `global_value`: explicit JSON overrides, SQL NULL when absent. Those columns
are always SQL NULL for secret definitions. The window decodes them into an
`overrides` map whose missing properties are distinct from explicit JSON null. `unavailable` distinguishes a secure
backend or invalid-value failure from an unset preference without exposing an
error message or claiming an origin. Project editing controls read project then
global overrides; they do not prefill from a winning local value and copy it into
shared config. Effective values remain available alongside the editing values.
The capability chooser can therefore show a hidden project choice while a local
choice wins, and its local reset exposes that retained choice.

## Secrets

A secret uses the same get/set/unset, validation and precedence. The service
stores its serialized value in the OS keychain. Local/project identities include
the canonical project root; global identities do not. All include the global
configuration home, so `OXPLOW_HOME` also isolates secure settings. A project-level
secret is machine-held and is not distributed through Git. Secret defaults and
plaintext YAML secret overrides are refused. Secure storage failures return safe
errors and never fall back to files or an inherited value.

Authorized reads return the actual value; inventories expose only presence and
origin. A failed individual resolution retains its definition with
`unavailable: true` and unknown value/origin, allowing Settings to load and
repair credentials. Direct reads still fail closed. Settings values are omitted from write audit inputs. Secret-derived
command reads propagate the internal `config.secret` trace marker through
composition, failures and rehearsal; recording redacts derived inputs, results
and errors, withholds undo payloads and payload events, and refuses to persist a
secret-bearing approval proposal. Caller results remain available. Consumer
errors crossing the command boundary are sanitized before Alerts can retain them.

AI provider API keys are registered secret strings; endpoint binding is host
metadata inside secure storage, not part of the caller's value. A destination
change refuses the old key until it is saved for the new endpoint. Collector
credentials and named provider instance credentials are also registered secrets.
Their existing UI commands are adapters over this service. OAuth session/token
state and opaque host secret-reference identities remain operational state.

## Compatibility and lifecycle

This is a clean break, with no automatic migration. Legacy unqualified project
keys and personal.yaml/ai.yaml/instances.yaml configuration are not read. Registry
commands reject undeclared keys; namespaced file values for an absent extension
are retained and validated when its definitions return. Old database configuration
rows are no longer runtime configuration. Applied migrations remain historical.

Core bootstrap definitions are available before extension discovery. Extension
registration checks ownership, duplicate keys, offline schemas and defaults.
Configuration files are watched by exact path, including the global file in
every open project. The global directory has a nonrecursive watch; only its
`metrics/` library is recursive. Missing global directories are discovered by
polling the configured paths, never by recursively watching an ancestor (which
could be the person's home). Creation, deletion and replacement rearm the bounded
registrations; unrelated files named `project.yaml`, `local.yaml` or `global.yaml`
do not reload settings. Inventory queries use the same injected global directory as the host
service, including isolated test and fixture hosts. Secret consumers read the secure backend; provider credential changes
restart the corresponding instances. Runtime state such as health, token refresh
state, command history, recents and sessions stays outside configuration files.

## Development verification

Pure registry, schema, inheritance, file safety and secure-backend cases are in
`oxplow-config` settings tests. App tests cover generic APIs, typed preflight,
canonical undo, extension reads/SQL, secret audit boundaries and provider/collector
consumers. Extension fixtures cover the Settings inventory and task workflow
commands. Test-only fixture writers translate typed runtime documents into the
new file format; there is no such compatibility path in production.

## Host sharing and ordinary batches

Host settings stores share a bounded cache keyed by project/global paths, secure
backend identity, extension declarations and configuration content. Values
are read fresh; file content or declaration changes rebuild dynamic registry
entries. Declaration-only projections fork their registry so they cannot mutate a
cached host registry. Compiled offline schema validators are shared by schema
content in a separate bounded cache. Bulk runtime projections and provider
credential inventories read the three documents once per related operation;
provider lists reuse one instance/scope snapshot.

`config.update {level, set, unset}` writes ordinary settings together: validate
all keys, lock and edit one document, then atomically replace it once. Domain
settings actions complete this external write before reporting success; errors
propagate instead of being logged after audit. Secrets do not participate in a
YAML batch. Multi-key undo uses the same batch API.

## Standard Settings UI and independent features

The Settings index lists categories and searches titles, descriptions and keys.
Category URLs share one native `settings` visualization, backed by `v_config_value`.
Every setting shows its effective value/source separately from the View/edit level
selector. All levels is collapsed initially; it exposes hidden overrides and the
registered default. Choosing a level is read-only. Save and Discard apply to one
setting; removing an override differs from saving an empty value or explicit null.
Ordinary drafts belong to the tab history by key and level. Secret drafts stay in
memory. Refreshes retain drafts and show when their saved baseline changed.
The inventory adds `default_value`, `setting_order`, and `local_status`,
`project_status`, `global_status` (`set`, `unset`, `unavailable`). Secure statuses
never contain values; failed credentials remain replaceable/removable.

Independent feature switches replace aggregate enable lists:
`oxplow.agents.<harness>.enabled` and `oxplow.extensions.<name>.enabled`.
Harness preferences live beneath their harness key. Agents are alphabetical;
there is no priority or implicit default. A session explicitly names its harness,
and a new thread opens the picker. Last successful picker choice is UI memory,
never a config setting or an automatic session start. Existing sessions keep their
harness when it is disabled for new sessions.

Direct AI connections have one group per declared provider,
`oxplow.ai.<provider>.enabled`, `.baseUrl` and secret `.apiKey`. There are no
named AI accounts/instances. These credentials are separate from harness logins.
Each role has an atomic assignment at `oxplow.ai.roles.<role>`; an extension role
`namespace/profile` uses `namespace.profile` in the registered key. Objects and
arrays still replace whole values. External integration provider instances remain
supported; they are not direct AI connections.

The old agents/agentConfig, extension enable/disable lists, AI provider/role maps,
and newThreadSession settings are superseded. The snapshot hook timeout is an
internal default. Development source selection is the debug-only launch environment
`OXPLOW_EXTENSIONS_FROM_SOURCE=true` (set by `bun run run:dev`), rather than a product
setting. Metric definitions remain extension contributions; metric settings only
select them and adjust preferences. Change-ranking coefficients are extension
constants. Session-context injection remains configurable pending a separate decision.
