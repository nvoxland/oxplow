# Unified settings

Core and extensions use one registry and one storage service. Settings APIs
never select a storage implementation: a definition's `secret` flag does.

## Definitions and resolution

Each definition has a fully qualified key, title, description, category, JSON
Schema, optional default, UI hints and ordering. The host
assigns ownership. Categories are independent of ownership. Extension
declarations are translated by `extension/extension_settings.rs`. Fixed object
properties become independent settings; arrays, dynamic maps and explicit
`x-atomic` objects are one setting. Prefer scalar options wherever they can be
edited independently.

**One level type.** `oxplow_domain::settings::Level` (`local`, `project`,
`global`) is the level for storage, the `config.*` and domain commands, the
`config.changed` event and `OxplowConfig.active_provider_origins`;
`oxplow-config` re-exports it. `settings::Origin` is a level or `default`
(`Origin::level()`, `From<Level>`). The frontend uses the generated `Level`.

Core declares its settings directly in `oxplow-config/src/core_settings.rs`:
`table()` holds one entry per fixed key (key, title, description, category,
schema, default, UI hint) with the function that applies its value to
`OxplowConfig`; `core_settings::keys` names the keys core code uses. A few
families have one setting per member and are applied by their key's shape:
`oxplow.activeProviders.<capability>`, `oxplow.agents.<harness>.enabled` and
`.<preference>`, `oxplow.ai.roles.<role>`, and `oxplow.extensions.<name>.enabled`.
`OxplowConfig` is a typed view, never a storage format: `config_from` builds it
from resolved values, each fixed setting's value or else its default (the
definition's default is the only one), so a new core setting is one table entry.
A structured value's schema comes from its Rust type (`schema_of::<T>()` on the
whole value, so nested definitions resolve). A test pins every core definition's
metadata and the sample values its schema accepts
(`oxplow-config/testdata/core_settings.json`; `OXPLOW_PIN_UPDATE=1` rewrites it),
so a change to either shows in review.
**Categories** are registered like settings (`SettingCategory`: id, title,
description, order, owner). Core declares its twelve in
`core_settings::categories()` (order 0 to 110); an extension declares its own in
`configuration: categories: { <id>: { title, description, order } }` (a core id
is an error at check); a category a setting names without a declaration gets a
title from its id and order 1000. `v_config_value` carries `category_title`,
`category_description` and `category_order`, and the Settings index and
category pages sort and title by them.

Extension schema annotations are `title`, `description`, `x-category`, `x-ui`,
`x-order`, `x-secret`, `x-mask` and `x-atomic`; any other `x-` key is refused at check
(`extension_settings::ANNOTATIONS`). `x-mask: { prefix, suffix }` (a secret's
only; each 0–8; the definition's `mask`, `settings::SecretMask`, default
`{0, 4}`) is how many characters of each end an agent sees. `x-ui` (the
definition's `ui`, `settings::SettingUi`) is one fixed vocabulary:
`{ editor: multiline }` for a text area. A labeled choice is the schema's own:
`oneOf` of `{ const, title, description }` (the editor's select shows the titles
and the chosen one's description; a `when: { config, in }` checks its values
against the consts as it does an `enum`; oxtask's task workflow part reads the
labels from `v_config_value.schema`). Anything else is an error at
`extension check` (`manifest_v2::configuration_of` runs the settings bindings,
so `x-order`, secret defaults and `x-ui` all show there).

An effective setting is the first value present at **local → project → global →
default**. Objects and arrays replace whole values; nothing merges across levels.
Loading checks every level is well formed (no bare or retired keys) but checks
only a key's effective value against its schema: a bad override that a higher
level shadows never stops the config loading.
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

**`oxplow.files.kinds`** (project category) holds the project's own file-kind
rules, `[{ kind, glob, label? }]`: a kind is lowercase words joined by dashes
and a glob parses as `testing.testPaths` do (a bad one is refused when set).
They are the `project-settings` rows of `v_file_kind_rule`, beside the enabled
extensions' `file_kinds:`; `config_reactors::restate_file_kinds` restates them
on a change and at boot.

## APIs and consumers

**Reads go through one snapshot.** `SettingsStore::snapshot()` reads the three
files once into `Documents` (cheap to clone; `snapshot_of(documents)` reuses a
capture, such as the app's metadata-checked cache in `settings::documents`).
`SettingsSnapshot` answers every read from that capture: `get(key, level)`
(effective with the default, or one level only; secrets included), `inspect`
(the same, secret value left out), `row(key)` and `inventory()` (the Settings
row: redacted effective value, per-level overrides and statuses, a failure kept
in its row) and `documents()` (raw levels, unknown keys included). Reading
several keys takes one snapshot, so they agree with each other. The store
keeps only the definition, validation and write methods.

`oxplow.config.get {key, level?}` returns value, presence, secret metadata and
origin. Without a level it resolves inheritance; with a level it reads only that
override. `oxplow.config.set {key, value, level}` and `.unset {key, level}` require
an explicit `local`, `project` or `global`. `list_keys` returns definitions from
the registry. Values are validated by registered offline schemas before storage
(a refusal is `SettingsError::ValueRefused`: each problem's JSON pointer in the
value and a reason in words from the schema alone, never the value, so a refused
secret stays secret; the command's error field is `/value<pointer>` and the
Settings editor shows the message);
a core value is also checked by its own rule (`core_settings::check`, the rule
`config_from` applies; no core rule reads another setting), and the write then
builds the typed config once over the files as they would be. An override invalidated by an extension schema change
can still be replaced or removed; undo is withheld when it would restore that
invalid value.

**One write.** `config.set`, `config.unset`, `config.update` and every domain
action (`file.mark_generated`, `capability.choose`, `replacement.choose`, the
agent settings, `metric.enable`) go through `settings_commands::write_changes`.
It checks each value (a required extension's switch is registered `const: true`
before the switches the files name, so it can't be turned off), skips a key
whose stored value is the same (no write, no `config.changed`, no undo; nothing
changed answers `changed: false`), builds the typed config the files would make
and refuses a write that would leave a project that loads unloadable (an unset
exposing a bad hand-edited value; files that already fail to load are repaired
one key at a time, and the answer says `loads: false` with the problem until
they load), writes once, refreshes the typed config and builds the undo. A
refused secret's problems have no pointer: a pointer into an object would name
its keys. The composed-run ownership rule is on the generic `config.*`
commands only: a domain action is its own grant.

Extension scripts declare `needs: [config.read]` and call
`scope("config.read", {"key": "acme.limit"})`. Their secret reads and all composed config writes are limited to
the owning namespace, including refusal of core settings and core credentials.
Scripts write by composing the ordinary config commands.
Settings have three read paths. Scripts and providers use
`scope("config.read", {key, level?})`. SQL uses `config('/pointer')`. Pages and
agents use `v_config_value`. There is no nested snapshot read, and no stamps,
CAS or migration protocol. SQL `config('/pointer')` and conditions receive a
public projection that omits secrets before any secure read. Configuration-aware
extensions republish when their resolved public settings change.

`v_config_value` is the registry-backed, redacted settings inventory, including
labels, categories, schemas, origin and presence. Metric/dimension definitions
are catalog contributions and are not preferences in that inventory.
`v_config_key` lists registered core and extension keys with docs and schemas.
Settings uses a registry-driven category index and standard form editors; operational tools live on linked management pages.
The `config_value` dataset contract is version 6. For ordinary settings the inventory also exposes `local_value`, `project_value`
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
origin. An agent never holds a secret: its own `oxplow.config.get` of one returns
`value: null`, presence, origin and `masked` (`SecretMask::apply`: the declared
ends around `••••`, shown only when they are at most a third of a string, dots
otherwise; AI provider keys show 7 and 4, `sk-ant-••••WXYZ`), a separate field
so a mask can't be saved back as the value. An extension command reading its own
secret (composed, owner-checked) still gets the value when an agent runs it. A failed individual resolution retains its definition with
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
keys and personal.yaml/ai.yaml/instances.yaml configuration are not read (a present `ai.yaml` is reported, below). Registry
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
`oxplow-config` settings tests. App tests cover generic APIs, per-key core checks, unchanged writes,
canonical undo, extension reads/SQL, secret audit boundaries and provider/collector
consumers. Extension fixtures cover the Settings inventory and task workflow
commands. Tests write the real file format: flat registered keys.

## Host sharing and ordinary batches

Host settings stores reuse bounded caches of registry declarations and immutable
ordinary-document snapshots. File metadata (including inode/change time on Unix)
invalidates parsed documents; every operation rechecks it. Registry identity uses
only settings-relevant declarations, dynamic key identities, instance topology,
and provider-declaration file metadata, not ordinary preference values or whole
extension manifests. Each caller receives an isolated registry fork; adding local
provider/test definitions cannot modify the cached base. Compiled offline schema
validators remain shared by schema content.

AI keeps a declaration-specific fork until provider kinds or profile identities
change. Its hot configuration reads batch keys from one ordinary snapshot and do
not acquire the registry's write lock. The Settings inventory derives effective
values, overrides, and per-level statuses from one snapshot, reading each secure
identity once per level. Endpoint binding uses those same ordinary documents.
A redacted read of a secret (the inventory's statuses, `inspect`) is its
presence: `SecretStore::exists_with_documents`, which the app's secure store
answers with an attribute-only keychain search that decrypts nothing and so
never prompts (its endpoint binding, inside the value, is checked only by a
read that uses the secret). Keychain reads go through one process-wide cache
(`oxplow_ai::secrets::CachedSecrets`): a secret is decrypted at most once per
run. Reads that miss the cache take one lock and look again, so readers that
start while the first waits on the person's keychain answer get that answer,
not a prompt each. A presence check the keychain can't answer by search falls
back to that cached read. Each real keychain read logs `reading a secret from
the keychain` with the item's name (never its value) to the daemon log
(`.oxplow/logs/daemon.<date>.log`), so a prompt can be traced to its read. macOS prompts twice for one read
by an unsigned build, and again after each rebuild; dev builds signed with a
stable local identity (`DEV.md` "Sign dev builds") are trusted across rebuilds. OS credential access may still fail, in which case only that row/level is
unavailable. Unknown harness preferences remain stored through bootstrap until
their schema returns (the runtime keeps them as plain JSON per harness); harness
switches and role objects are validated before the typed view is built, with
errors naming the setting key. `enabled` is reserved under every harness key:
provider declarations are refused for it, and a test holds built-in harnesses to it.

`config.update {level, set, unset}` writes ordinary settings together: validate
all keys, lock and edit one document, then atomically replace it once. Domain
settings actions complete this external write before reporting success; errors
propagate instead of being logged after audit. Secrets do not participate in a
YAML batch. Multi-key undo uses the same batch API.

## Standard Settings UI and independent features

The Settings index lists categories (by their registered order, with titles and descriptions), links to the changed settings, and searches keys, titles, descriptions and values.
Category URLs share one native `settings` visualization, backed by `v_config_value`.
The visualization composes the shared `Section`, `Facts` and `DataTable` kit
components rather than defining its own page chrome. Each setting has a stable
section identity (`<viz node>/<setting key>`) and participates in the ordinary
Contents outline with the correct nested heading level. The page host counts
settings rows alongside other row-based sections when deciding whether to mount
Contents, including pages that have no filters or other rail content. All-level and technical
metadata stay native disclosures within the section, outside the outline.
`SettingEditor` owns only the level-specific draft/save/reset behavior, while
`SettingField` supplies structured schema controls styled by the standard form kit.
Every setting shows its effective value/source separately from the level a
change goes to: a scalar's "Saves to" beneath its inline control, any other
setting's View/edit level selector (`.context/usability.md` Settings). All levels is collapsed initially; it exposes hidden overrides and the
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
`namespace/profile` uses `namespace.profile` in the registered key. That encoding
is `oxplow_domain::ai::role_setting_key` / `role_from_setting_key`, the one copy
the AI loader, core settings and the AI service share. Objects and
arrays still replace whole values. External integration provider instances remain
supported; they are not direct AI connections.

The old agents/agentConfig, extension enable/disable lists, AI provider/role maps,
and newThreadSession settings are superseded. The snapshot hook timeout is an
internal default. Where oxplow's own extensions run from is no setting: a dev
build runs each one whose source the project has (an `extensionSources` entry or
`.oxplow/extensions/<name>/` with a manifest) from that source; a release build
never does (`ExtensionSettings::from_source`). Extension switches resolve to one
effective `oxplow.extensions.<name>.enabled` per extension (`ExtensionSettings::switches`),
and a capability's choice records the level it came from (`chosen_by`: `local`,
`project`, `global`); there are no separate team and personal layers. Metric definitions remain extension contributions; metric settings only
select them and adjust preferences. Change-ranking coefficients are extension
constants. Session-context injection remains configurable pending a separate decision.

Obsolete aggregate core keys fail with the file path, replacement guidance and no value disclosure;
no file is rewritten or migrated. Extension catalogs, including frozen metric
preparation catalogs, retain their host's injected Global directory. A different
machine/global directory cannot change feature enablement in an isolated host.

A file edited on disk (a `git pull`, an agent's own edit) is not a settings
write: the watcher's `reload_config_from_disk` logs no `config.changed` (no
command ran, so there is no actor, command run record or undo, and the watcher also fires
on a command's own write, which would log it twice). It re-applies what is
derived from the config itself: the snapshot filter, the project's file kinds
(`restate_file_kinds`, when `oxplow.files.kinds` moved), extension activation,
and `ConfigChanged` for open pages.

Settings writes emit `config.changed@3` with local/project/global levels;
capability selections emit `capability.switched@3`, whose `chosen_by` is the
level that chose (`local`, `project`, `global`), `default` or `fallback`.
V40 and V42 carry existing core event rows to these versions, renaming the old personal
level to local without changing values or expired payloads. As with other core
event bumps, only the newest versions remain registered.

Standalone AI loading also refuses retired provider/role aggregates explicitly,
and refuses while the old `ai.yaml` sits in the global directory, naming the file
(never its values) and what replaced it.
The clean break does not migrate or delete legacy keychain entries; re-enter keys
through the current provider settings. Session opens require an explicit harness (a required field of the command's schema);
`agent_session.open` is the single slot-opening command (no `open_slot` alias).
