# Extensions: Events, resources and effects

The event envelope, event types, resources, ordering and effects. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

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
`ref_kinds` (now `resources`) in P10, extensions-slots.md "Decorators" and "Resources"; `effects` and
`custom_components` in P11, below and extensions-components.md "Custom components";
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
extension's `permissions:` (extensions-permissions.md "Permissions"); the commands its script
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
