# Extensions: Activation, candidates and repair

Health, disable and repair, and how a catalog snapshot is prepared and published (commands, vocabulary, runtime adapters, ingress). Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

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
  `contribution.disabled@1`, the Extensions page) with or without a work
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
- **In the app (C3).** the Extensions page shows one health line per
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
activation's `problems` (the Extensions page), so the reason is visible
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
problem in the Extensions page. Cancelling the async caller does not release
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
