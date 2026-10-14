# Refs: one identity for everything

What this doc covers: the canonical ref grammar, the kind registry, and
how refs replace tab ids, `page_ref` kinds and `[[…]]` wikilink shapes.
Built so far: the grammar (P1.1, tsk403); the kind registry, the Rust
`[[…]]` translation and the canonical `page_ref` vocabulary (P1.2,
tsk404); the TS parser, tab ids, wikilink hrefs and comment anchors
(P1.3, tsk405); shell routes as `page:<name>[?params]` and the
`Record<PageKind, …>` render registry (P1.3b, tsk418).

## Using refs (read this first)

oxplow is a web of things that link to each other. Every thing has one
readable identity, its **ref**, `<kind>:<id>[@rev][#frag]`
(`effort:eff12`, `file:src/a.rs@git:HEAD#L3`, `work_item:oxplow:tsk42`,
`collector:triage/todos`). The rules:

1. **The owner spells a ref; everyone else carries it.** A ref is minted
   in exactly one place per language and passed along verbatim. Nothing
   rebuilds one from an id (`"effort:eff" + id`, `'decision:' || id`) or
   takes one apart by hand (`strip_prefix("wiki:")`, `split(':')`,
   `ref[len("effort:eff"):]`).
2. **Links are refs, and may dangle.** Store the ref string. Extensions
   come and go and records get deleted; a ref nothing resolves is a dead
   link, shown as its raw text, never an error. Reads never fail on one.
   A work item says what it is about with a link `about` any ref
   (`oxplow.work_item.link { ref, target, link_type: about }`, or
   `create { links }`); find the items about a thing by that link
   (`v_work_item_link.to_ref`), never by a title or a naming convention.
3. **Two things stay strict:** the grammar (a malformed ref is refused at
   every write) and the event log (an event's subject must name a
   registered kind).

| Where | Build | Read |
|---|---|---|
| Rust | `oxplow_domain::refs::build::{effort_ref, file_ref, wiki_ref, …}` — one per core kind; `of(kind, id)` / `at(kind, id, rev, frag)` for a kind known only at run time; a work list builds its own items' refs (`oxplow_tasks::work_item_ref`) | `build::parse_as::<T>(r, kind)` (typed id), `id_of(r, kind)` (decoded id), `parse_kind(r, kind)` (keeps rev and frag), `kind_of(r)`; kind names are `refs::kind::{EFFORT, FILE, …}` |
| SQL (models, lenses, scripts' queries) | the owning model's `ref` column (`oxplow_db::ref_models::REF_MODELS`: `v_effort.ref`, `v_decision.ref`, `v_thread.ref`, …) — the only SQL that spells a prefix | `SELECT ref`, `WHERE ref = :ref`, `JOIN … ON x.ref = y.subject_ref`; never `'kind:' || id`, never `substr` on a ref |
| Starlark (commands, effects, collectors, policies) | `ref.build(kind, id, rev=, frag=)` (`id` a string or an int) | `ref.parse(r)` → `{kind, id, rev, frag}`, `ref.id(r, kind=)`, `ref.kind(r)`, `ref.is_ref(v)` |
| TS (desktop) | `refs/ref.ts` builders (`effortRef`, `wikiRef`, `pageRef`, `refOf`, …) | `parseRef`, `idOf`, `isRefOfKind` |

**Resolving.** `ref_resolver::resolve_ref` names any ref: richer
resolvers for `work_item`, `commit`, `file`, `dir`, `wiki` and
`finding`; any other core kind through its owning model's title column
(`REF_MODELS`); an extension's kind through the `resolve` view its
`ref_kinds:` names. Nothing resolves it → `resolved: false`.

**Guards.** `refs/refs.guard.test.ts` (TS), `source_guards::refs_are_built_and_parsed_through_the_grammar` (Rust) and `source_guards::scripts_and_sql_carry_refs_rather_than_spelling_them` (shipped `.star`, lens and model SQL, embedded Starlark templates; a kind's owning model in `REF_MODELS` is exempt) refuse a hand-built or hand-parsed ref outside the owning
place. When one fires, the fix is the builder or reader above — or, when
a kind has none, adding it to `refs::build` / `ref_models` — never a
string.

## The grammar (built)

```
ref  := kind ":" id [ "@" rev ] [ "#" frag ]
kind := [a-z][a-z0-9_]*            snake_case, starts with a letter
id   := 1+ chars                    `@` `#` `%` percent-encoded (%40 %23 %25)
                                    `:` `/` `?` `=` `&` and spaces are legal raw
rev  := rev_kind ":" value          git:HEAD, git:4c44d4, snap:01J9…; omitted = working tree
frag := kind-defined                file/dir: L10 or L10-20
```

Examples: `effort:eff362`, `commit:4c44d495`, `work_item:oxplow:tsk42`,
`work_item:issues:ENG-12`, `file:src/a.rs@git:HEAD#L10-20`,
`lens:acme/blocked?stream_id=2`, `page:settings`.

**Parsing rules,** in this order: the first `:` ends the kind; the first
`#` ends the id-and-rev (fragments aren't encoded); the first unescaped
`@` before that starts the rev. So `:` inside an id is fine, which is what
lets a provider-scoped id (`oxplow:tsk42`) read naturally. A literal `@`
in a file name must be `%40`; a bare `file:src/a@b.rs` is rejected as a
bad revision (`b.rs` isn't `<kind>:<value>`), and the error says how to
encode it.

**Why only three reserved characters.** Refs are meant to be typed, pasted
and read by people and agents. Encoding `/` or `?` would make
`lens:acme/blocked?stream_id=2` unreadable for no gain; only the three
characters the grammar itself uses need escaping.

**A revision is always `<kind>:<value>`.** `@HEAD` is refused: the reader
must know whose revision it is (`git:HEAD`, `snap:01J9…`). This is what
makes snapshots and VCS independent capabilities (§6.1–6.2).

**Code.** `crates/oxplow-domain/src/refs/grammar.rs`:
`CanonicalRef { kind, id, rev, frag }` (decoded id), `parse`, `new`,
`Display` (re-encodes), `RefParseError` with a stable `reason()` name per
variant. Pure, no IO.

**The golden fixture** `crates/oxplow-domain/tests/fixtures/ref_grammar.json`
pins the grammar. `tests/ref_grammar.rs` asserts it, and the TS parser
(`apps/desktop/src/refs/ref.ts`: `parseRef`, `formatRef`, `ref`,
`kindOf`; asserted by `ref.test.ts`) asserts the same file, so the two
parsers can't drift. Add a case there when you extend the grammar;
never change an existing case's expectation without changing both parsers.

## The kind registry (built)

**Contribution kinds are ref kinds.** `core_kinds()` registers one ref
kind per kind of contribution, from `contribution::KINDS`
(`refs::kind::contribution_kind`): `<kind>:<extension>/<id>` (`job`,
`effect`, `collector`, `ai_profile`, `lens`, `page`, `model`, `panel`, …)
or `<kind>:<name>` for the globally named ones (`command`, `measure`,
`metric`, `dimension`, `event_type`, `ref_kind`). A page is also a shell
route (`page:settings`), a model also one of core's (`model:effort`), and
a collector also a project's
(`collector:project/repo.scan_clone`) until those become extensions. `job_run` names one run of a job (`job_run:<uuid>`);
`ai_model` a language model by the name its harness reports.

**Event sources aren't refs.** An event's `source` says who emitted it —
`human`, `system:<component>`, `agent:thr3`, `provider:<instance>`,
`effect:<extension>/<id>`, `effect:effort_policy:<id>`, `job_run:<id>` —
and some coincide with refs, but a provider instance or an effort policy
isn't one. They are built beside the refs (`refs::build::{system_source,
effect_source, provider_source}`, `Actor::source()`) and read with
`effect_of_source`, never spelled elsewhere.

`crates/oxplow-domain/src/refs/kind.rs`: `KindSpec { kind, id_regex,
revisioned, provider_scoped, wikilink_prefixes }` and
`KindRegistry` (`register` refuses a collision; `validate` checks a ref's
kind, id shape and whether it may carry `@rev`). `core_kinds()` registers
the §4.2 vocabulary plus `finding`, `thread_note` and `run` (a work
item's comment has no kind of its own: its mentions are its item's
edges). Only
`work_item` is provider-scoped in P1 (`oxplow:tsk42`,
`issues:ENG-12`); `wiki`, `commit` and `symbol` keep bare ids and use the
capability's active provider.

`agent_session` (an agent slot on a thread, `agent_session:ses3`,
`build::agent_session_ref`) is registered with the `ses` id. It isn't
`agent:` — that name is kept for the harness registry.

`symbol` (`symbol:<path>/<id>`, `build::symbol_ref(path, id)`) names a
definition in a file. The id is oxplow's own, from tree-sitter
(language-support.md "Symbols"): containers and name joined by `::`,
then `(param types)` in languages that overload by parameters (Java, C#,
C++) or a Clojure defmethod dispatch value, then `~n` for the nth repeat
of the same id in the file. The identity carries no revision: the ref
names the symbol as it is now. Pin a version with `@snap:<id>` or
`@git:<sha>` (`build::at`); the pin is how you read, never part of the
identity. The id regex is `^[^/\n][^\n]*/[^\n]+$` (the path is the longest
`/`-prefix naming a file). Examples:

- `symbol:crates/oxplow-app/src/lib.rs/Services::boot`
- `symbol:src/parse.rs/<Parser as Display>::fmt` (Rust trait impl)
- `symbol:src/main/java/com/acme/Parser.java/Parser::parse(String,int)`
- `symbol:src/acme/core.clj/area(:circle)`
- `symbol:scripts/build.py/main~2`

Not built: a location inside a symbol. The design is a fragment of a
content fingerprint plus the nth instance (`…/Parser::parse#h:3f9a~2`),
not a line offset: a relative line silently points at the wrong line
after an insert, a fingerprint fails visibly. There is no rename
following: a renamed symbol is a dead link.

`config` (a config key, `config:zones`) is registered too (P2.4b, tsk450).
`extension` (`extension:<name>`, an extension by its name) is the subject
of a contribution's `contribution.disabled@1` / `contribution.enabled@1`
and of an effect's `effect.result`; `build::extension_ref`.
A `page` id is a shell route (`page:settings`) or an extension's page,
the contribution `page:<extension>/<id>`; a slash tells them apart.
`answer` (a lens an agent showed on a thread, `answer:12`, P6.C1) is
registered with a numeric id; `build::answer_ref` and `build::lens_ref`
(`lens:<extension>/<slug>`) build them. `proposal` (a command waiting for
a person's decision, `proposal:12`, P6b) has a numeric id too;
`build::proposal_ref` builds it. So do `claim` (an agent's claim about its
work, `claim:7`) and `decision` (one it recorded or oxplow inferred,
`decision:3`) — what the review commands name (P7.C4); `build::claim_ref`
and `build::decision_ref`. `collector` (`collector:<owner>/<id>`, the
owner an extension or `project`, the id dotted identifiers:
`collector:project/repo.scan_clone`, `collector:oxplow-foundation/oxplow.fn_count`) names a collector — the subject of
its `collector.synced@1` (P7.B3); `build::collector_ref`.

## Building refs in Rust (built, P2.4b)

Producers never hand-format a ref, and readers never split one by hand.
`oxplow_domain::refs::build` has one builder per core kind, each taking
the id as the grammar escapes it, and the kind names are constants
(`oxplow_domain::refs::kind::{EFFORT, FILE, WIKI, …}`):

- generic: `of(kind, id)` and `at(kind, id, rev, frag)` — for a kind only
  known at run time (an extension's);
- records: `project_ref`, `stream_ref`, `thread_ref`, `effort_ref`,
  `turn_ref`, `snapshot_ref`, `commit_ref`, `branch_ref`;
- files and code: `file_ref`, `file_ref_at(path, rev, lines)`, `dir_ref`,
  `symbol_ref(path, id)`, `wiki_ref`;
- activity: `comment_ref`, `event_ref`, `lens_ref`, `answer_ref`,
  `page_ref(name, query)`, `extension_page_ref`;
- catalog: `metric_ref`, `model_ref`, `extension_ref`, `command_ref`,
  `config_ref`, `finding_ref`, `collector_ref`;
- review and runs: `thread_note_ref`, `agent_session_ref`, `run_ref`,
  `proposal_ref`, `claim_ref`, `decision_ref`.

Readers: `parse_as::<T>(r, kind)` (the id parsed as `T`), `id_of(r, kind)`
(the decoded id; refuses a revision or fragment), `parse_kind(r, kind)`
(keeps them) and `kind_of(r)`; a string that isn't a ref of the kind is
`NotA` (“`x` isn't a <kind> ref (<kind>:<id>)”). `work_item_id_of_ref`
reads a work item's provider-scoped id. A work list builds its own items'
refs (oxplow's tasks: `oxplow_tasks::{work_item_ref, work_item_id,
task_of_work_item_ref}`). `system_source(component)` gives the
`system:<component>` event source a system producer uses; an actor's runs
use `Actor::source()`.

**The guard.** `source_guards::refs_are_built_and_parsed_through_the_grammar`
scans production Rust in every crate (outside `oxplow-domain/src/refs/`
and `oxplow-tasks/src/refs.rs`) and fails on a string literal that begins
with a core kind's prefix (`format!("effort:…")`, `"wiki:"`,
`strip_prefix("file:")`, a SQL `'work_item:' ||`) and on a ref-named value
split at its colon. Its kept list is empty. What only looks like a ref (an
actor source, an approval key, a revision, a URL) is not one: spell it so
the pattern doesn't match, don't add an exception.

`refs::build::validate_work_item_ref` is stricter: efforts key on the
ref string (one open effort per work item is a unique index), so it
accepts only the canonical spelling — no `@rev`, no `#frag`, no escaped
spelling. What a list's own ids look like is that list's to say, not
core's.

**A work list's own ids in text.** The running `KindRegistry` carries
the active work list's id recognizer (`with_work_item_ids(provider,
id_pattern)`; set from the capability registry when services are built
and at every vocabulary rebuild, a `capability.switched` included —
`vocabulary_reactor::with_work_item_ids`). So `tsk42` in text, `[[…]]`
or loose, is `work_item:oxplow:tsk42` while oxplow's tasks are the list,
`ENG-12` an issue tracker's while it is, and nothing is one with none
(`work_item_id`, `find_work_item_ids`: whole tokens of letters, digits,
`_` and `-`). An implementation declares its `id_pattern`
(`BuiltIn.id_pattern`, a provider's `providers:` entry). `command` ids have two or more dot segments;
`commit` ids are 7–64 hex (SHA-256 repositories).

**There is no process-wide registry (P8.D1, tsk761).** The kinds live in
the running `Vocabulary { events, kinds }`
(`crates/oxplow-domain/src/vocabulary.rs`), held in a `VocabularyHandle`
(`Arc<RwLock<Arc<Vocabulary>>>`) that `Services.vocabulary` owns and every
store and consumer clones. Installing an extension swaps a whole new
vocabulary in; a writer takes `handle.current()` once per transaction, so
a swap never changes what an open transaction validates against. Every
function that reads kinds takes them: `refs::validate_ref(&kinds, r)`,
`canonical_wikilink`, `classify_wikilinks`, `extract`, and the
`page_ref_projections` edge builders. Tests build `core_kinds()` or their
own handle; two handles in one process never share a kind. An extension's
`ref_kinds:` (P8.D6, stable since P10 — extensions.md "Ref kinds") join
the running kinds through the vocabulary reactor, and `v_ref_kind` lists
them all. The documented one is the github example's `github_pr`
(`[[pr:12]]`, its `pr` page). A `wikilink:` prefix two extensions share
costs only the sugar; each namespaced kind still registers.

**Searchable kinds** (P9.D3). An extension's kind declared `searchable:
<model>` has that model's rows (`ref`, `title`, `body`) in the site-wide
index under its kind — an asset per kind (`kind_search.rs`), so search
hits carry `{ kind, ref_id }` for extensions' kinds as for core ones and the
desktop routes them through `searchHitTarget`. Core's tasks, comments,
thread notes and wiki pages are indexed the same way (tsk864): each an
asset over its `v_search_<kind>` model (`kind_search::CORE_KINDS`),
whose rows also carry their stream and aren't bounded. Like every
published model, a feed's `ref` is the canonical ref a hit opens
(`work_item:oxplow:tsk7`, `thread_note:not5`, `comment:cmt3`,
`wiki:<slug>`; tsk922) — `CORE_KINDS` names the prefix each search kind's
rows carry before the id — and a refused read of a table names its own
model first (`read v_work_item or …`). Only files stay
on the `search.index` pump consumer, since their text comes from
snapshot blobs. A restate writes only the entries that differ (tsk896):
each `search_entry` keeps its title-and-body hash (`content_hash`, V164,
replacing V158's kind-wide digest, which skipped an unchanged kind but
rewrote every entry on any edit) — so a start, which registers every kind
and builds each once, rewrites nothing when nothing changed, and editing
one wiki page rewrites that page's entry (measured at 500 pages of ~5 KB:
31.8 ms → 0.47 ms). The kind's rows are still read and hashed on every
recompute. A thread or
stream delete, a move, a soft delete: the kind follows its model, so
nothing is left behind (the old upsert-only boot backfill kept orphans).
**Archived work leaves search** (tsk921): archiving a stream purges its
files (`purge_stream_files`), and the task, note and comment models skip
an archived thread or stream (`archived_at`), so their kinds restate
without it; a backlog task (no thread) stays. Wiki pages belong to no
stream (`archived_threads_and_streams_leave_search`). An extension's
kind is never `revisioned` yet: none has a reader for a
revision. The design is recorded (extensions.md "Ref kinds" →
"Revisioned extension kinds"): `revisioned: true`, a `rev` column on the
`resolve` model (NULL = current), a page lens taking `rev` (`?ref=…&rev=…`),
`[[pr:12@git:abc1234]]` through this grammar, and search entries kept
rev-less.

A kind's index is re-registered — and recomputed whole — when anything
that decides its rows changes (`SearchableKind`): its view, the view's
**compiled SQL** (from `sqlite_master`; an edit of the model changes what
is indexed without touching the tables, tsk851), its id pattern, or the
tables behind the view; otherwise it recomputes when a commit touches
one of those tables. A kind's entries leave only when `ref_kind` stops
declaring it searchable (its extension removed, its `searchable:`
dropped) — never because its view isn't published at the moment: at
start the extension models are dropped and compiled again, and "not
compiled yet" isn't "gone" (tsk852). What a gone kind left is found
wherever it is (tsk853): its `asset_state`, its `asset_failure` (a kind
whose recomputes only ever failed has nothing else), or entries in the
index with neither — a restate can commit after a removal's cleanup, or
a process die between the two — every kind in `search_entry` that isn't
core's (`kind_search::CORE_KINDS`, or files) or declared searchable.

`refs::validate_ref` parses a ref and checks it against the kind
registry. The event log's `append_tx` runs it on every `subject`
entry, so an event naming `zones`, `config:`, `nope:thing`,
`commit:xyz` or `effort:12` (ids are `effN`) is refused with the event
type in the message.

## `[[…]]` sugar → canonical refs (built)

`refs::canonical_wikilink(interior)` turns a wikilink interior into a
`CanonicalRef`: one of the active work list's ids → its work item
(`tsk42` → `work_item:oxplow:tsk42` with oxplow's tasks), `git:abc1234` and a
bare sha → `commit:abc1234`, `dir:src/` → `dir:src`, `src/a.rs@HEAD:42` →
`file:src/a.rs@git:HEAD#L42` (`@disk`/`@local` add no rev), a kebab slug
→ `wiki:<slug>`, and the canonical form itself passes through when its
kind is registered. `word:tail` with an unknown kind is **not** a slug
named `word`; it's a malformed ref and translates to nothing.
`ClassifiedWikilink` carries both `canonical` and the typed `Reference`
view (derived from it via `TryFrom<&CanonicalRef>`), so the graph writer
and the link checker never drift.

## Stored `page_ref` rows are canonical (built)

`crates/oxplow-db/src/page_ref_projections.rs` writes `(kind, id)` pairs
that are exactly a canonical ref's: kinds `work_item`, `commit`, `dir`,
`thread_note`, `wiki`, `file`, `finding` (a work item's comments are its
own edges, `comment_*`); a work item's id is
`<provider>:<id>` (`oxplow:tsk<n>`). An effort's impacts name their kind
in the agent tools' vocabulary (`IMPACT_KINDS`: `wiki | work_item | file
| directory | git_commit | finding`; `oxplow.effort.report` refuses any other,
V165 renamed stored spellings and V20 made `task` `work_item`, its ids
canonical refs), which `impact_kind` projects to the canonical ones; a
`work_item` impact's id is its ref or one of the active list's own ids. Migration V92 wiped the old rows; the boot
restate regenerates them. `v_commit_work_item` reads the new shape.
`ref_resolver::resolve_ref` and `CommentTarget` use the same kinds.

## The TS side is on canonical refs (built)

A `TabRef.id` for an entity page *is* its canonical ref, and
`TabRef.kind` is the ref's kind (`PageKind` in `tabs/tabState.ts`):

| Was | Now |
|---|---|
| `task:tskN` (kind `task`) | `work_item:oxplow:tskN` (kind `work_item`, payload `itemId: "tskN"`) |
| `git-commit:<sha>` | `commit:<sha>` |
| `dir:<path>` (kind `directory`) | `dir:<path>` (kind `dir`) |
| `file:<p>:@ref:<x>` | `file:<p>@git:<x>` (`@snap:<id>` for snapshots; `revisionSlot`/`revisionFromSlot` in `revision.ts`) |
| `metric-detail:<key>` | `metric:<key>` |

The same vocabulary is used by every TS surface that names an entity, so
one string round-trips everywhere: the `page_ref` graph reads
(`useBacklinks.canonicalIdForTarget` returns `oxplow:tskN` for a task),
markdown hrefs (`MarkdownView` emits `commit:<sha>` and
`work_item:oxplow:tskN`, collapses them back to `[[git:sha]]` /
`[[tskN]]`; `InternalLink` allowlists the same schemes), comment anchors
(`data-ref-kind="work_item" data-ref-id="oxplow:tskN"`, `commit`, `dir`,
matching the backend's `CommentTarget`), and lens `link.kind` targets.
`kindForTabId` and `refFromTabId` go through `parseRef` first, so a `:`
inside an id never splits it.

**One place spells a ref; one place reads it.** A ref's owner spells it
and everyone else carries it. In the desktop, `apps/desktop/src/refs/ref.ts`
is the only file that writes a kind prefix (`refOf(kind, id)` and a builder
per kind the desktop names: `threadRef`, `streamRef`, `effortRef`,
`commentRef`, `agentSessionRef`, `wikiRef`, `workItemRef(provider, id)`,
`eventRef`, `proposalRef`, `configRef`, `pageRef(name, params?)`,
`extPageRef`, `lensRef`), and `parseRef` / `kindOf` / `idOf(ref, kind)` are
how a ref is read; everything else takes a ref from a builder or from data
and passes it on. A revision (`git:`, `snap:`) is not a ref and stays
`revision.ts`'s. `refs/refs.guard.test.ts` fails on a hand-built or
hand-parsed prefix of any core kind anywhere else (a template
`` `kind:${…}` ``, `"kind:" +`, `startsWith("kind:")`,
`slice("kind:".length)`, `replace(/^kind:/…)`, or `indexOf(":")` on a
ref-named variable); its list of kinds is `kinds` in
`crates/oxplow-domain/tests/fixtures/ref_grammar.json`, which
`ref_grammar.rs` asserts equals `core_kinds()`, so both languages read one
list. Its kept list is empty: use a builder.

**A snapshot, an effort and an agent turn are entity pages** (P2.11):
`snapshot:<N>`, `effort:<effN>`, `turn:<trnN>` (kinds `snapshot` /
`effort` / `turn`, built by `snapshotRef` / `effortDiffRef` / `turnRef`
in `tabs/pageRefs.ts`), each rendering its diff in `DiffViewPage`
(`DiffViewPayload.mode`). Only an ad-hoc pair of endpoints stays a
route (`page:diff-view?start=…&end=…`). A lens `effort-diff` link
takes the effort's ref (`v_effort.ref`, `effort:eff7`). Tab ids are always built
through the ref helpers (`fileRef(path).id`, never `` `file:${path}` ``):
`escapeId` percent-encodes a path's `@` / `#` / `%`, which a hand-built
id would read as a revision or fragment.

**Shell routes are `page:<name>[?params]`** (`page:agent`, `page:tasks`,
`page:diff-view?start=s1&end=s9`, `page:external-url?url=…`). A route is a
page of the shell, not a thing in the graph, so it never appears in
`page_ref`; its `TabRef.kind` is the route name. The route names keep
their hyphenated spelling (`diff-view`) — they are ids inside the
`page` kind, not kinds, and the snake_case rule is for kinds. The
params grammar, the exhaustive `ROUTES` inverse table and the
`pageRenderers` registry are described in
[pages-and-tabs.md](./pages-and-tabs.md).

Decided 2026-09-28: there is no compatibility layer for old ids. Only
Nathan uses oxplow; stale `page_ref`/`page_visit` rows are wiped in V92
and regenerated, and the UI starts new localStorage keys
(`oxplow.layout.v2.*`, `oxplow.bookmarks.v2.*`); `legacyRedirects.ts`
is gone.

## Strict vocabulary candidates

The activation API freezes declarations and the host work item recognizer from
one catalog snapshot before writing. PreparedVocabulary.replace_tx rejects any
event/ref/subscription/recognizer diagnostic and restores its database savepoint;
it returns the candidate vocabulary without swapping VocabularyHandle. A shared
wikilink prefix rejects the strict candidate, whereas the legacy live reactor
still registers the kinds without their conflicting sugar. PreparedExtensionDatabase
composes this with models and contribution descriptors, restoring the complete
candidate on rejection. When prepared capabilities are included, its recognizer
comes from the planned work-list selection rather than the prior live registry,
and capability rows/switch events share that transaction. Coordinated
commit/publication and reader pinning remain the activation owner's unfinished
work; these helpers do not change the live reactor's partial diagnostics policy.
