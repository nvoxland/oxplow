# Resources and their URIs

What this doc covers: how oxplow names the things in it — the URI
grammar, the resources extensions declare, how each language builds and
reads a URI, where URIs are stored, and the migration that moved every
stored name onto them.

**The words.** A **resource** is a kind of thing in oxplow — a file, a
wiki page, an effort, a work item, an extension's pull request — named
`<extension>/<name>` (`oxplow/file`, `github/pr`). A **URI** names one
thing of a resource: `oxplow:oxplow/effort?id=eff12`. The direction is the
web's: a URI names the thing, a page that shows it is one **renderer** of
it, and how that renderer shows it (a line, a filter, a pick, a mode, an
open panel) is its **view**: the `#`, never part of the identity. A page's
`?` params name what it shows; the `#` is how. `oxplow_get_open_page`
returns both, `page` (identity) and `view`. Dataset columns, command inputs and event
fields that hold a URI are named `uri` (V101); prose says URI.

## Using URIs (read this first)

1. **The owner spells a URI; everyone else carries it.** A URI is built in
   one place per language (the builders below) and passed along verbatim.
   Nothing concatenates one (`"oxplow:oxplow/effort?id=" + id`,
   `'…' || id`) or takes one apart by hand (`strip_prefix`, `split`,
   `substr`): read it with the readers.
2. **Links are URIs, and may dangle.** Store the URI. Extensions come and
   go and records get deleted; a URI nothing resolves is a dead link,
   shown as its raw text, never an error. Reads never fail on one. A work
   item says what it is about with a link `about` any URI
   (`oxplow.work_item.link { uri, target, link_type: about }`); find the
   items about a thing by that link (`v_work_item_link.to_uri`).
3. **Two things stay strict:** the grammar (a malformed URI is refused at
   every write) and the event log (a subject must be a URI of a declared
   resource whose params hold).
4. **A field named for one resource carries its bare id, not a URI**
   ("Bare ids" below). A URI is for a field that can name anything.

| Where | Build | Read |
|---|---|---|
| Rust | `oxplow_domain::uri::build::{effort_uri, file_uri, wiki_uri, work_item_uri, …}`; `keyed(resource, value)` for a resource named by its key param; `of(resource, &[(k, v)])` / `of_with_view` for one known only at run time | `parse_as::<T>(uri, resource)` (the key, typed), `id_of` (the key), `param_of(uri, resource, name)`, `parse_resource` (keeps the view), `resource_of`, `work_item_of_uri` → `(provider, id)`, `file_of_uri` → `(path, rev, lines)`; resource names are `uri::resources::{EFFORT, FILE, …}` |
| SQL (datasets, pages, scripts' queries) | the owning dataset's `uri` column; to build one, `uri('oxplow/effort', 'id', 'eff' \|\| id)` (a NULL value leaves its param out, so `uri()` is never NULL: over a `LEFT JOIN`, `CASE WHEN x.id IS NULL THEN NULL ELSE uri(…) END`) | `SELECT URI`, `WHERE uri = :uri`, `JOIN … ON x.uri = y.subject`; `uri_param(URI, 'id')`, `uri_resource(URI)` |
| Starlark (commands, effects, collectors, policies, mappings) | `uri.build("oxplow/file", path = p, view = {"lines": "3-9"})` (a param a string or an int; `None` leaves it out) | `uri.parse(u)` → `{resource, params, view}`, `uri.param(u, name, resource=)`, `uri.id(u, resource=)`, `uri.resource(u)`, `uri.is_uri(v)`, `uri.contribution(u)` |
| TS (desktop) | `uri/uri.ts` builders (`effortUri`, `wikiUri`, `workItemUri(provider, id)`, `fileUri(path, rev)`, `fileLinesUri`, `pageUri(page, params, view)`, `metricUri`, `withUriView`, `viewParamsOfUri`, `resourceUri(resource, params, view)`, …) | `parseUri`, `identityOf`, `typeOf` (the resource), `paramOf`, `viewOf`, `idOf(uri, resource)`, `keyOf`, `workItemOf`, `isExternal` |

**Guards.** `apps/desktop/src/uri/uri.guard.test.ts` (TS),
`source_guards::refs_are_built_and_parsed_through_the_grammar` (Rust) and
`source_guards::scripts_and_sql_carry_refs_rather_than_spelling_them`
(shipped `.star`, page and dataset SQL, embedded Starlark templates) refuse
a hand-built or hand-parsed URI outside the builders. When one fires, the
fix is a builder or reader — never a string.

## Bare ids

A field whose name already fixes the resource — a command input, a
command result, an event payload field — carries that resource's **bare
id**, the id type's own spelling. A URI there adds only length, escaping,
a parse step and a wrong-resource error. Rust parses one with
`commands::util::bare_id::<T>(raw, resource, field)`, which refuses a URI
saying the id it names.

| Field | Bare id |
|---|---|
| `thread`, `from_thread`, `to_thread`, `stream`, `effort`, `turn`, `session` (an agent session), `note`, `handoff`, `comment` | the prefixed id: `thr3`, `str1`, `eff12`, `trn4`, `ses2`, `not3`, `hnd1`, `cmt5` |
| `snapshot`, `parent`, `start_snapshot`, `end_snapshot`, `claim`, `decision`, `proposal`, `supersedes`, `answer`, `run` | a JSON integer |
| `event`, `job_run` | the UUID |
| `collector`, `job` | the contribution id: `triage/todos` |
| `commit`, vcs.head.moved `from` / `to` | the sha |
| knowledge.page.* `page` | the slug |

**What stays a URI:** a field that can name anything (`uri` when it isn't
fixed, `target`, `subject`, `evidence`, comment `context_chain` /
`referenced`, impacts, navigate / bookmark / page targets, a metric's
`subject`, `open_after`), event subjects, and **work items everywhere**
(`work_item`, an item's `uri`, `parent_uri`, `before`, `after`): an item's
id is unique only with its provider. Models keep integer ids
(`v_thread.id = 3`); SQL that feeds a fixed input reads the key from the
row's URI (`uri_param(t.uri, 'id')`), never `'thr' || id`. In Starlark,
build the URI (`uri.build("oxplow/effort", id = e)`) only where a bare id
becomes a link or is compared with a dataset's `uri`.

## The grammar

```
uri          := "oxplow:" extension "/" resource [ "?" params ] [ "#" view ]
params, view := key "=" value ( "&" key "=" value )*
extension    := [a-z][a-z0-9-]*          the declaring extension's segment
resource     := [a-z][a-z0-9_]*
key          := [A-Za-z][A-Za-z0-9_.-]*
```

```
oxplow:oxplow/work_item?id=tsk42&provider=oxplow
oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-20
oxplow:oxplow/page?page=acme/blocked&stream_id=2
oxplow:github/pr?id=12
```

- **Named params only, keys unique, order free.** The canonical form sorts
  keys bytewise and leaves raw only ASCII letters, digits and
  `-._~/:?,@!$'()*;`; everything else is `%XX`, uppercase hex. A raw `+`
  is a plus, never a space. So `?id=4&group=5` and `?group=5&id=4` are
  one URI. A duplicate key is refused.
- **The view is not the identity.** `identity()` / `identityOf` is the
  canonical URI without its `#…`: what `page_ref`, the search index and
  tab identity key on. Views core reads: `lines=10-20` (a file),
  `part=input|output|prompt|result|summary` (an event's body),
  `part=request|response` (a model call's), `part=result` (a subagent's
  last message). Keys core will read from now on are `oxplow.*`
  (`#oxplow.renderer=<extension>/<page>` is a view key the desktop reads:
  it picks which renderer draws the URI when several declare it).
- **A revision is a param.** `rev=git:HEAD`, `rev=snap:3`: the
  `Revision` strings (`revision.ts`), never `@…`. Only files, directories
  and symbols take one. What a `rev` means belongs to the file's
  filesystem: the `Revision` vocabulary is the local filesystem's, and
  another filesystem has its own (a deployment hash).
- **A file names its filesystem.** `filesystem=<id>` (`oxplow:temp`,
  `acme:s3`; [filesystems.md](./filesystems.md)) on a file or directory
  URI says where its bytes come from. Absent is `oxplow:local`, the
  stream's worktree, so every URI written before filesystems keeps its
  identity; the builders (`file_uri_at`, `dir_uri_at`, `fileUri`,
  `dirUri`) leave out an explicit `oxplow:local` so one file has one
  identity. Readers resolve the id through `Services.filesystems`, and
  an unknown id is refused naming the ones there are.
- **Any other scheme is an external resource** (`https:`, `file://`):
  `AnyUri { Oxplow, External(url::Url) }` types the split; a link may be
  either.
- **Code.** Rust `oxplow_domain::uri::oxplow_uri` (`OxplowUri`,
  `ResourceName`, `AnyUri`; `OxplowUri::new` panics on a bad key — a
  builder's keys are literals — and `OxplowUri::build` returns the error,
  for keys known at run time); TS `apps/desktop/src/uri/uri.ts`. Both
  split the string by hand (the URL standard reads `+` as a space and
  normalizes paths, so the two languages would disagree) and assert one
  golden fixture, `crates/oxplow-domain/tests/fixtures/uri.json`. Add a
  case there when you extend the grammar; never change an existing one's
  expectation without changing both parsers.

**Key params.** A resource named by one value has a key param
(`build::key_param`): `id` for a stored record (`effort?id=eff1`,
`thread?id=thr3`, `turn?id=trn1`), a word for the value's shape
(`commit?sha=`, `wiki?slug=`, `branch?name=`, `extension?name=`,
`ai_model?name=`, `config?key=`, `nudge?kind=`, `consumer?name=`), and
the resource's own name for a contribution (`page?page=tasks`, `command?command=oxplow.file.save`,
`collector?collector=triage/todos`, `effect?effect=acme/notify`,
`resource?resource=github/pr`). Resources with several identity params
have none: `file?path=&rev=&filesystem=`, `dir?path=&filesystem=`, `symbol?path=&symbol=`,
`work_item?provider=&id=`, `subagent?session=&agent=`,
`extension_deployment?name=&hash=`, `block?path=&lines=`,
`test?class=&name=`. A page's own params ride beside its key
(`page?page=settings&area=agents`); `build::params_of` splits them.

**Event sources aren't URIs.** An event's `source` says who emitted it —
`human`, `system:<component>`, `agent:thr3`, `provider:<instance>`,
`effect:<extension>/<id>`, `effect:effort_policy:<id>` — built by
`uri::build::{system_source, effect_source, provider_source}` and
`Actor::source()`, read with `effect_of_source`. A provider instance or
an effort policy isn't a resource. Nor are a ui-preference scope's
`resource:<ext>/<name>` and `component:<ext>/<id>`.

## Resources are declared

There are no core resources. **Every resource is declared by an extension**
in its manifest's `resources:` block — oxplow-foundation declares oxplow's
(`extensions/oxplow-foundation/extension.yaml`):

```yaml
resources:
  - name: pr                    # oxplow:github/pr?id=12
    label: Pull request
    params: { id: { required: true, pattern: '^\d+$' } }
    additional: false           # true: takes params beyond its own (page)
    resolve: pull_request       # one of its datasets, with `uri` and `title`
    title: title                # the column of `resolve` that titles one
    searchable: pull_request    # a dataset with `uri`, `title`, `body`
    icon: git-pull-request      # one of ICONS, or assets/<name>.svg in the extension
    wikilink: { prefix: pr, param: id }   # [[pr:12]]
```

- **The extension segment** is the extension's name, or `oxplow` for the
  extension whose namespace is oxplow's (only a bundled one may take it).
  `resolve` and `searchable` name the extension's own datasets
  (`v_<ext>_<dataset>`), or core's for the oxplow namespace
  (`resolve: agent_turn`). A `pattern` is anchored and kept in one form
  both regex engines read alike (`param_pattern.rs`).
- **Loading.** `crates/oxplow-app/src/extension/extension_resources.rs` parses the
  block (`ResourceEntry` → `ResourceDecl`); each resource is a contribution
  of kind `resource` (`oxplow:oxplow/resource?resource=<ext>/<name>`), so a
  resource two extensions declare is the later one's load error. A
  resource has no `page:`: it opens as itself, and the pages that render
  it say so with `pages[].for` ([extensions.md](./extensions.md)
  "Pages"). `v_resource` is at v4 (V103 dropped the `page` column).
- **The registry.** `oxplow_domain::uri::resources::ResourceRegistry`
  (`register` refuses a second resource of a name or a second wikilink
  prefix; `validate` checks a URI's resource, its required params, the
  patterns, and no undeclared param unless `additional`). It lives in the
  running `Vocabulary { events, resources }`
  (`crates/oxplow-domain/src/vocabulary.rs`), in a `VocabularyHandle` that
  `Services.vocabulary` owns; the vocabulary reactor rebuilds it from
  every enabled extension and restates the `resource` table, read as
  `v_resource`. `Vocabulary::core()` — what a process starts from, and what
  a test builds — is core's event types plus oxplow-foundation's resources
  read from its manifest (`resources::foundation()`); manifest checks that
  run before any vocabulary (a decorator's `resource:`, a prompt's
  `about:`) read the same list (`extension_resources::resource_problem`).
- **A work list's own ids.** The registry carries the active work list's
  id recognizer (`with_work_item_ids(provider, id_pattern)`, set from the
  capability registry at every rebuild): `tsk42` in text, `[[…]]` or loose,
  is `oxplow:oxplow/work_item?id=tsk42&provider=oxplow` while oxplow's tasks
  are the list (`work_item_uri_of`, `find_work_item_uris`).
- **What names a resource elsewhere** is its name: `ui.about` on a command
  (`about: oxplow/work_item`), an intent prompt's `about:`, a decorator's
  `resource:`, a fact subject's `subject_kind` (`oxplow/file`), a search
  hit's `resource` and a search filter's `resources`, a provider
  capability's `resource` (`ProviderContract.resource`: a URI with a
  `provider` param is that instance's).

## Resolving and reading

`ref_resolver::resolve_ref(svc, uri)` → `RefSummary { uri, resource,
title, detail, body_excerpt, resolved }`: richer resolvers for work items,
commits, files, directories, wiki pages and findings; any other resource
through its declaration's `resolve` view and `title` column, by `uri`.
Nothing resolves it → `resolved: false`.

`ref_read::read` (`oxplow_read`, `records.read`) returns `{ uri, resource,
row, related, content, truncated }`: the `resolve` row, the related lists
of work items, commits and efforts, a file's text at its `rev` (its
`#lines=` view), an event's or model call's body (`#part=`), a page's
definition.

`link_check` checks a wikilink's URI against its resource's `resolve`
dataset; `symbol_resolve` finds a symbol now (`v_symbol`) or at its `rev`.

## `[[…]]` sugar

`uri::canonical_wikilink(resources, interior)` turns a wikilink interior
into a URI: a URI passes through when its resource is declared; one of
the active list's ids → its work item; `git:abc1234` and a bare sha → a
commit; `dir:src/` → `dir?path=src`; `finding:fnd-1`; a declared
`wikilink.prefix` (`pr:12` → `oxplow:github/pr?id=12`); `src/a.rs@HEAD:42`
→ `file?path=src/a.rs&rev=git:HEAD#lines=42`; a kebab slug → a wiki page.
`word:tail` with no declared prefix is not a slug; it names nothing.
`ClassifiedWikilink` carries `canonical` and the typed `Reference`, so the
graph writer and the link checker never drift. The desktop's
`postprocessWikilinks` collapses URIs back to the sugar.

## Where URIs are stored

- **`page_ref`** (V99): `(source_uri, target_uri, source_resource,
  target_resource, ref_type, source_extra, local_snapshot_id,
  closest_vcs_rev, vcs_rev_exact)`, keyed `(source_uri, target_uri,
  ref_type)`; `PageRefEdge { source, target, … }`. Every end is an
  identity (no view); a file edge's version pin rides in its columns, not
  its URI. The writers are `page_ref_projections` (wiki pages, notes,
  handoffs, work items' bodies, comments and links, efforts' touched files,
  summaries and impacts, findings, commits) — each owns its source's slice.
- **`comment`** (V99): `target_uri` (and `target_resource`), a
  `context_chain_json` and `referenced_json` of URIs;
  `knowledge.comment.written@2` / `deleted@2` carry `{ comment, target }`.
- **The search index**: `search_entry.kind` is the resource and `uri`
  the URI; `kind_search` keeps one index per searchable resource (an
  asset `search:<resource>`), the `search.index` consumer the files.
  A hit is `{ resource, uri, stream_id, title, snippet, score }`.
- **Everything else** stores the URI string where it stored the old ref:
  every dataset's `uri` column, `work_item.uri` / `parent_uri`, links,
  efforts' `work_item`, claims, decisions, handoffs, bookmarks, page
  visits, ui-preference scopes, fact subjects, event subjects and payload
  fields, command inverses and pending proposals.

## The migration (V99)

`V99__uris.sql` rewrote every stored URI in one transaction, through
`ref_to_uri(text[, extension])` and `ref_to_uri_in_json(json, field, …)`
— SQL functions over `oxplow_db::uri_migration`, **the one place the old
grammar survives** (`uri_migration/v1.rs` parses it; nothing else does).
It is frozen: its table of old kinds never grows. Notable reads: a work
item `work_item:<provider>:<id>`; a symbol `<path>/<symbol>` or, as the
metric scans spelled one, `<path>::<name>` (told apart by the file name
before the first `::`); a block `<path>:<lines>`; a test `<class>::<name>`;
a contribution's id → its key param, a page's own params kept;
`@rev` → `rev`, `#L10-20` → `#lines=10-20`; an old URI whose params can't
be a URI's is left as it was. Event payloads rewrite a fixed list of URI
fields (and `work_item.recorded`'s item); `command.executed` payloads and
executed audit inputs stay as history. Tables restated at boot (the
`contribution` table) are emptied; an extension kind's search entries go
and rebuild from its view. On a 1.5 GB local database it takes about 30 s.

## The bare-id migration (V100)

`V100__bare_ids.sql` moved the fields of "Bare ids" off URIs, through
`uri_keys_in_json(json, field, …)` (a field written `<name>:int` becomes
a number): every event type whose payload changed, at its new version
(no reader carries an upcast, as at V70); undo inverses; pending
proposals, whose old URIs V99 missed (it looked for a NULL decision, not
`'pending'`). A job's `intent_key` is immutable and keeps an embedded URI.

## The rename (V101)

`V101__uri_names.sql` renamed every name that said `ref` for a URI to
`uri`. Tables: `work_item.uri` / `parent_uri`, `work_item_comment.uri`,
`work_item_link.from_uri` / `to_uri`, `task_link.to_uri`,
`fact_subject.uri`, `search_entry.uri` (was `ref_id`), `bookmark.uri`,
`symbol.uri`, `change_function.uri`, `claim.evidence` (was
`evidence_ref`; it can be the word `reviewer`, so it is no `*_uri`).
`wiki_page.file_refs_json` / `dir_refs_json` became `files_json` /
`dirs_json`: a wiki citation is a file path, so its names say `files`
(`verify_refs` is now `oxplow.knowledge.verify_files`, with `files`,
`verified_files`, `removed_files`). Events: stored `work_item.recorded`
rows moved to v3 (`item.uri`, `item.parent_uri`) and
`knowledge.page.recorded` rows to v2 (`page.uri`, `page.uris`). Undo
inverses and pending proposals had their input keys renamed (`ref` to
`uri`, `parent_ref`, `evidence_ref`, `refs` and the rest likewise). The
core datasets, command inputs and results, templates (`{{uri}}`,
`{{uri.id}}`, `{{row.uri}}`, `{{param.uri}}`), page params, the
`?uri=` page param and a page `timeline: { uri }` follow.

It left alone: `ref('<dataset>')` in dataset SQL (a dataset reference, not a
URI), git refs (`branch_ref`, `basis_ref`, `ref_moves`), `page_ref` and
its `ref_type`, `v_knowledge_ref`, `TabRef`, and internal identifiers
that sit on no wire (`resolve_ref`, `refOpener.ts`, `refFromTabId`).

## The desktop

A tab's id is its URI (`TabRef.id`; a web page's is its URL), and a
`TabRef` is `{ id, pinned? }`: there is no kind and no payload, so the
desktop reads a tab from its URI (`typeOf`, `paramOf`). `apps/desktop/src/resources.ts` reads
`v_resource` (icons, labels, an extension's wikilink prefixes). **A page
is a renderer of a resource**: which one draws a URI comes from the
`pages[].for` declarations, then the `#oxplow.renderer=` view key, then
the page marked `default`, then the first (`pages/renderers.ts`,
[pages-and-tabs.md](./pages-and-tabs.md) "Renderers"). A URI nothing
renders (no page, or no row in its dataset) opens on foundation's
fallback `resource` page: its label and params, or the raw URI muted as
a dead link. It is never an error and never logged or toasted. The
row is looked up by the URI's identity: its view (`#part=output`) is for
the page, not the dataset.
**Where a URI opens is decided once**: `tabs/refOpener.ts` and
`tabs/openRef.ts` (`openRef`, `PageNavigation.openRef`) — a wikilink, a
page link, a backlink row, a search hit, a component's `navigate` and
`oxplow_navigate` (`replace`, `new_tab`; by default the shown tab goes there) all hand it a URI. `refFromTabId` parses a tab id
(history, bookmarks, a restored tab) and is not an opener: every oxplow
URI and every allowlisted http(s) URL is a tab, else null. A stream's or
thread's URI selects it; every other URI opens as itself. Comment anchors
carry one `data-ref="<uri>"`. Tab sessions are stored under
`oxplow.layout.v7.sessions`; old
tabs aren't migrated.

## Strict vocabulary candidates

The activation API freezes declarations and the host work item recognizer
from one catalog snapshot before writing. `PreparedVocabulary.replace_tx`
rejects any event, resource, subscription or recognizer diagnostic and
restores its database savepoint; it returns the candidate vocabulary
without swapping `VocabularyHandle`. `PreparedExtensionDatabase` composes
this with datasets and contribution descriptors, restoring the whole
candidate on rejection. When prepared capabilities are included, its
recognizer comes from the planned work-list selection rather than the
prior live registry, and capability rows and switch events share that
transaction. Coordinated commit and publication and reader pinning remain
the activation owner's unfinished work.
