# Refs: one identity for everything

What this doc covers: the canonical ref grammar, the kind registry, and
how refs replace tab ids, `page_ref` kinds and `[[…]]` wikilink shapes.
Target design: [target-architecture.md](./target-architecture.md) §4.
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
`work_item:linear:ENG-12`, `file:src/a.rs@git:HEAD#L10-20`,
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
revisioned, provider_scoped, lifecycle, wikilink_prefixes }` and
`KindRegistry` (`register` refuses a collision; `validate` checks a ref's
kind, id shape and whether it may carry `@rev`). `core_kinds()` registers
the §4.2 vocabulary plus `finding`, `task_note` and `run`. Only
`work_item` is provider-scoped in P1 (`oxplow:tsk42`,
`linear:ENG-12`); `wiki`, `commit` and `symbol` keep bare ids and use the
capability's active provider.

`config` (a config key, `config:zones`) is registered too (P2.4b, tsk450).

## Building refs in Rust (built, P2.4b)

Producers never hand-format a ref. `oxplow_domain::refs::build` has one
builder per kind oxplow emits: `stream_ref`, `thread_ref`, `effort_ref`,
`turn_ref`, `snapshot_ref`, `commit_ref`, `command_ref`, `config_ref`,
`work_item_ref(TaskId)` (`work_item:oxplow:tsk42`) and `work_item_id`
(the provider-scoped id alone, `oxplow:tsk42`, for `page_ref` rows).
The inverses are `task_of_work_item_ref` (strict: a full ref naming an
oxplow task) and `task_from_work_item_id` (lenient: also a bare `tsk42`
or `42`, as agents write in impacts). `system_source(component)` gives
the `system:<component>` event source a system producer uses; an
actor's runs use `Actor::source()`.

`refs::build::validate_work_item_ref` is stricter: efforts key on the
ref string (one open effort per work item is a unique index), so it
accepts only the canonical spelling — no `@rev`, no `#frag`, no escaped
spelling, and `oxplow:tsk<n>` exactly as `work_item_ref` writes it
(`tsk01` is refused). `command` ids have two or more dot segments;
`commit` ids are 7–64 hex (SHA-256 repositories).

`refs::validate_ref` parses a ref and checks it against the kind
registry. The event log's `append_tx` runs it on every `subject`
entry, so an event naming `zones`, `config:`, `nope:thing`,
`commit:xyz` or `effort:12` (ids are `effN`) is refused with the event
type in the message.

## `[[…]]` sugar → canonical refs (built)

`refs::canonical_wikilink(interior)` turns a wikilink interior into a
`CanonicalRef`: `tsk42` → `work_item:oxplow:tsk42`, `git:abc1234` and a
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
`task_note`, `wiki`, `file`, `finding`; a task's id is
`work_item_id(TaskId)` = `oxplow:tsk<n>`. `normalize_impact_kind` still
accepts the spellings agents write (`task`, `git_commit`, `directory`)
but only ever stores canonical ones. Migration V92 wiped the old rows;
the boot backfill regenerates them. `v_commit_task` reads the new shape.
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
