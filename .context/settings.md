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
sidecar, re-read under the lock and atomically replace only the changed YAML key,
preserving unrelated values and comments.

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
`scope("config.read", {"key": "acme.limit"})`. Their secret reads are limited to
the owning namespace. Scripts write by composing the ordinary config commands.
The retained `extension_config.read` snapshot is a nested read projection over
the same settings, not separate storage; it has values and origins, no stamps,
CAS or migration protocol. SQL `config('/pointer')` and conditions receive a
public projection that omits secrets before any secure read. Configuration-aware
extensions republish when their resolved public settings change.

`v_config_value` is the registry-backed, redacted settings inventory, including
labels, categories, schemas, origin and presence. Metric/dimension definitions
are catalog contributions and are not preferences in that inventory.
`v_config_key` lists registered core and extension keys with docs and schemas.
The existing Settings UI consumes the inventory; a visual redesign is separate.

## Secrets

A secret uses the same get/set/unset, validation and precedence. The service
stores its serialized value in the OS keychain. Local/project identities include
the canonical project root; global identities do not. All include the global
configuration home, so `OXPLOW_HOME` also isolates secure settings. A project-level
secret is machine-held and is not distributed through Git. Secret defaults and
plaintext YAML secret overrides are refused. Secure storage failures return safe
errors and never fall back to files or an inherited value.

Authorized reads return the actual value; inventories expose only presence and
origin. Settings values are omitted from write audit inputs. Secret-derived
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
Configuration files are watched, including the global file in every open
project. Inventory queries use the same injected global directory as the host
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
