# Refs: one identity for everything

What this doc covers: the canonical ref grammar, the kind registry, and
how refs replace tab ids, `page_ref` kinds and `[[…]]` wikilink shapes.
Built so far: the grammar (P1.1, tsk403); the kind registry, the Rust
`[[…]]` translation and the canonical `page_ref` vocabulary (P1.2,
tsk404); the TS parser, tab ids, wikilink hrefs and comment anchors
(P1.3, tsk405); shell routes as `page:<name>[?params]` and the
`Record<PageKind, …>` render registry (P1.3b, tsk418).

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

`crates/oxplow-domain/src/refs/kind.rs`: `KindSpec { kind, id_regex,
revisioned, provider_scoped, wikilink_prefixes }` and
`KindRegistry` (`register` refuses a collision; `validate` checks a ref's
kind, id shape and whether it may carry `@rev`). `core_kinds()` registers
the §4.2 vocabulary plus `finding`, `task_note` and `run`. Only
`work_item` is provider-scoped in P1 (`oxplow:tsk42`,
`issues:ENG-12`); `wiki`, `commit` and `symbol` keep bare ids and use the
capability's active provider.

`config` (a config key, `config:zones`) is registered too (P2.4b, tsk450).
A `page` id is a shell route (`page:settings`) or an extension's page
(`page:ext.<extension>.<page>`, P6.G2): its id pattern is exactly those
two shapes, so a `.` appears only in `ext.<extension>.<page>`.
`answer` (a lens an agent showed on a thread, `answer:12`, P6.C1) is
registered with a numeric id; `build::answer_ref` and `build::lens_ref`
(`lens:<extension>/<slug>`) build them. `proposal` (a command waiting for
a person's decision, `proposal:12`, P6b) has a numeric id too;
`build::proposal_ref` builds it. So do `claim` (an agent's claim about its
work, `claim:7`) and `decision` (one it recorded or oxplow inferred,
`decision:3`) — what the review commands name (P7.C4); `build::claim_ref`
and `build::decision_ref`. `collector` (`collector:<owner>/<id>`, the
owner an extension, `project` or `built-in`, the id dotted identifiers:
`collector:project/repo.scan_clone`) names a collector — the subject of
its `collector.synced@1` (P7.B3); `build::collector_ref`.

## Building refs in Rust (built, P2.4b)

Producers never hand-format a ref. `oxplow_domain::refs::build` has one
builder per kind oxplow emits: `stream_ref`, `thread_ref`, `effort_ref`,
`turn_ref`, `snapshot_ref`, `commit_ref`, `command_ref`, `config_ref`,
`work_item_ref(TaskId)` (`work_item:oxplow:tsk42`) and `work_item_id`
(the provider-scoped id alone, `oxplow:tsk42`, for `page_ref` rows).
The inverse is `task_of_work_item_ref` (strict: a full ref naming an
oxplow task), for oxplow's own implementation. `system_source(component)` gives
the `system:<component>` event source a system producer uses; an
actor's runs use `Actor::source()`.

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

**Searchable kinds** (P9.D3). A plugin kind declared `searchable:
<model>` has that model's rows (`ref`, `title`, `body`) in the site-wide
index under its kind — an asset per kind (`kind_search.rs`), so search
hits carry `{ kind, ref_id }` for plugin kinds as for core ones and the
desktop routes them through `searchHitTarget`. Core's tasks, comments,
thread notes and wiki pages are indexed the same way (tsk864): each an
asset over its `v_search_<kind>` model (`kind_search::CORE_KINDS`),
whose rows also carry their stream and aren't bounded. Like every
published model, a feed's `ref` is the canonical ref a hit opens
(`work_item:oxplow:tsk7`, `task_note:not5`, `comment:cmt3`,
`wiki:<slug>`; tsk922) — `CORE_KINDS` names the prefix each search kind's
rows carry before the id — and a refused read of a table names its own
model first (`read v_task or …`). Only files stay
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
stream (`archived_threads_and_streams_leave_search`). A plugin
kind is never `revisioned` yet: no plugin kind has a reader for a
revision. The design is recorded (extensions.md "Ref kinds" →
"Revisioned plugin kinds"): `revisioned: true`, a `rev` column on the
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
`task_note`, `wiki`, `file`, `finding`; a work item's id is
`<provider>:<id>` (`oxplow:tsk<n>`). An effort's impacts name their kind
in the agent tools' vocabulary (`IMPACT_KINDS`: `wiki | work_item | file
| directory | git_commit | finding`; `effort.report` refuses any other,
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

**A snapshot, an effort and an agent turn are entity pages** (P2.11):
`snapshot:<N>`, `effort:<effN>`, `turn:<trnN>` (kinds `snapshot` /
`effort` / `turn`, built by `snapshotRef` / `effortDiffRef` / `turnRef`
in `tabs/pageRefs.ts`), each rendering its diff in `DiffViewPage`
(`DiffViewPayload.mode`). Only an ad-hoc pair of endpoints stays a
route (`page:diff-view?start=…&end=…`). A lens `effort-diff` link
normalizes a row id (`7`) to `effort:eff7`. Tab ids are always built
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
