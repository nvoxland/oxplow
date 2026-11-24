# Resources and their URIs

What this doc covers: how oxplow names the things in it — the URI
grammar, the resources extensions declare, how each language builds and
reads a URI, where URIs are stored, and the migration that moved every
stored name onto them.

**The words.** A **resource** is a kind of thing in oxplow — a file, a
knowledge page, a request, a task, an extension's pull request — named
`<namespace>/<name>` (`oxplow/file`, `github/pr`). A **URI** names one
thing of a resource: `oxplow:oxplow/request?id=req12`. The direction is the
web's: a URI names the thing, a page that shows it is one **renderer** of
it, and how that renderer shows it (a line, a filter, a pick, a mode, an
open panel) is its **view**: the `#`, never part of the identity. A page's
`?` params name what it shows; the `#` is how. `oxplow_get_open_page`
returns both, `page` (identity) and `view`. Dataset columns, command inputs and event
fields that hold a URI are named `uri` (V101); prose says URI.

## Using URIs (read this first)

1. **The owner spells a URI; everyone else carries it.** A URI is built in
   one place per language (the builders below) and passed along verbatim.
   Nothing concatenates one (`"oxplow:oxplow/request?id=" + id`,
   `'…' || id`) or takes one apart by hand (`strip_prefix`, `split`,
   `substr`): read it with the readers.
2. **Links are URIs, and may dangle.** Store the URI. Extensions come and
   go and records get deleted; a URI nothing resolves is a dead link,
   shown as its raw text, never an error. Reads never fail on one. A task says what it is about with a link `about` any URI
   (`oxtask.task.link { uri, target, link_type: about }`); find the
   items about a thing by that link (`v_oxplow_task_link.to_uri`).
3. **Two things stay strict:** the grammar (a malformed URI is refused at
   every write) and the event log (a subject must be a URI of a declared
   resource whose params hold).
4. **A field named for one resource carries its bare id, not a URI**
   ("Bare ids" below). A URI is for a field that can name anything.

| Where | Build | Read |
|---|---|---|
| Rust | `oxplow_domain::uri::build::{request_uri, file_uri, …}` (a shared model's record has no builder: `ResourceRegistry::record_uri(store, id)`); `keyed(resource, value)` for a resource named by its key param; `of(resource, &[(k, v)])` / `of_with_view` for one known only at run time | `parse_as::<T>(uri, resource)` (the key, typed), `id_of` (the key), `param_of(uri, resource, name)`, `parse_resource` (keeps the view), `resource_of`, `file_of_uri` → `(path, rev, lines)`; resource names are `uri::resources::{REQUEST, FILE, …}` |
| SQL (datasets, pages, scripts' queries) | the owning dataset's `uri` column; to build one, `uri('oxplow/request', 'id', 'req' \|\| id)` (a NULL value leaves its param out, so `uri()` is never NULL: over a `LEFT JOIN`, `CASE WHEN x.id IS NULL THEN NULL ELSE uri(…) END`) | `SELECT URI`, `WHERE uri = :uri`, `JOIN … ON x.uri = y.subject`; `uri_param(URI, 'id')`, `uri_resource(URI)` |
| Starlark (commands, effects, collectors, policies, mappings) | `uri.build("oxplow/file", path = p, view = {"lines": "3-9"})` (a param a string or an int; `None` leaves it out) | `uri.parse(u)` → `{resource, params, view}`, `uri.param(u, name, resource=)`, `uri.id(u, resource=)`, `uri.resource(u)`, `uri.is_uri(v)`, `uri.contribution(u)` |
| TS (desktop) | `uri/uri.ts` builders (`knowledgeUri(store, id)`, `taskUri(provider, id)`, `fileUri(path, rev)`, `fileLinesUri`, `pageUri(page, params, view)`, `metricUri`, `withUriView`, `viewParamsOfUri`, `resourceUri(resource, params, view)`, …) | `parseUri`, `identityOf`, `typeOf` (the resource), `paramOf`, `viewOf`, `idOf(uri, resource)`, `keyOf`, `taskOf`, `isExternal` |

**Join two views by their ids, not their `uri` columns.** A dataset's
`uri` is built per row, so `JOIN … ON x.uri = y.turn` reaches no index: it
rebuilds the URI for every row of the other side, once for each row of
this side. Over `v_turn_file` × `v_agent_turn` that took the Shared work
part past `query_sql`'s 5 s; the same join on `turn_id` / `request_id` is
instant. Compare a `uri` to a param (`WHERE uri = :uri`) freely; join by
the id columns the views carry next to it (`request_id`, `turn_id`, `id`).

**Guards.** `apps/desktop/src/uri/uri.guard.test.ts` (TS),
`source_guards::refs_are_built_and_parsed_through_the_grammar` (Rust) and
`source_guards::scripts_and_sql_carry_refs_rather_than_spelling_them`
(shipped `.star`, page and dataset SQL, embedded Starlark templates) refuse
a hand-built or hand-parsed URI outside the builders. When one fires, the
fix is a builder or reader — never a string.

Settings use `oxplow/config?key=<namespaced setting key>`. The resource accepts
registered dotted keys (including scalar leaf names); historical unqualified
keys remain valid URI identities for old event subjects, although the settings
API no longer accepts those legacy keys.

## Bare ids

A field whose name already fixes the resource — a command input, a
command result, an event payload field — carries that resource's **bare
id**, the id type's own spelling. A URI there adds only length, escaping,
a parse step and a wrong-resource error. Rust parses one with
`commands::util::bare_id::<T>(raw, resource, field)`, which refuses a URI
saying the id it names.

| Field | Bare id |
|---|---|
| `thread`, `from_thread`, `to_thread`, `stream`, `turn`, `request` (`oxplow/request`), `session` (an agent session), `note`, `comment` | the prefixed id: `thr3`, `str1`, `trn4`, `req2`, `ses2`, `not3`, `cmt5` |
| `snapshot`, `parent`, `start_snapshot`, `end_snapshot`, `proposal`, `supersedes`, `answer`, `run` | a JSON integer |
| `event`, `job_run`, `contribution_run` | the UUID |
| `collector`, `job` | the contribution id: `triage/todos` |
| `commit`, vcs.head.moved `from` / `to` | the sha |
| `record.read` `record` | the row's URI (`oxplow:oxplow/knowledge?id=…&store=…`) |

**What stays a URI:** a field that can name anything (`uri` when it isn't
fixed, `target`, `subject`, `evidence`, comment `context_chain` /
`referenced`, impacts, navigate / bookmark / page targets, a metric's
`subject`, `open_after`), event subjects, and **tasks everywhere**
(`task`, an item's `uri`, `parent_uri`, `before`, `after`): an item's
id is unique only with its provider. Models keep integer ids
(`v_thread.id = 3`); SQL that feeds a fixed input reads the key from the
row's URI (`uri_param(t.uri, 'id')`), never `'thr' || id`. In Starlark,
build the URI (`uri.build("oxplow/request", id = r)`) only where a bare id
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
oxplow:oxplow/task?id=tsk42&provider=oxtask
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
  (`#oxplow.renderer=<namespace>/<page>` is a view key the desktop reads:
  it picks which renderer draws the URI when the daemon offers several).
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
(`build::key_param`): `id` for a stored record (`request?id=req1`,
`thread?id=thr3`, `turn?id=trn1`), a word for the value's shape
(`commit?sha=`, `branch?name=`, `extension?name=`,
`ai_model?name=`, `config?key=`, `nudge?kind=`, `consumer?name=`; a record of a shared model has two, its id and its store's param, e.g. `knowledge?id=<id>&store=<store>`, built by `ResourceRegistry::record_uri(store, id)` and read by `record_of`), and
the resource's own name for a contribution (`page?page=work`, `command?command=oxplow.file.save`,
`collector?collector=triage/todos`, `effect?effect=acme/notify`,
`resource?resource=github/pr`). Resources with several identity params
have none: `file?path=&rev=&filesystem=`, `dir?path=&filesystem=`, `symbol?path=&symbol=`,
`task?provider=&id=`, `subagent?session=&agent=`,
`extension_deployment?name=&hash=`, `block?path=&lines=`,
`test?class=&name=`. `person` is the person using oxplow, with none
(`oxplow:oxplow/person`, what a task is assigned to when it's theirs;
an optional `id` names one of several, for later). A page's own params ride beside its key
(`page?page=settings&area=agents`); `build::params_of` splits them.

**Event sources aren't URIs.** An event's `source` says who emitted it —
`human`, `system:<component>`, `agent:thr3`, `provider:<instance>`,
`effect:<namespace>/<id>` — built by
`uri::build::{system_source, effect_source, provider_source}` and
`Actor::source()`, read with `effect_of_source`. A provider instance
isn't a resource. Nor are a ui-preference scope's
`resource:<namespace>/<name>` and `component:<namespace>/<id>`.

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
  (`v_<namespace>_<dataset>`), or core's for the oxplow namespace
  (`resolve: agent_turn`). A `pattern` is anchored and kept in one form
  both regex engines read alike (`param_pattern.rs`).
- **Loading.** `crates/oxplow-app/src/extension/extension_resources.rs` parses the
  block (`ResourceEntry` → `ResourceDecl`); each resource is a contribution
  of kind `resource` (`oxplow:oxplow/resource?resource=<namespace>/<name>`), so a
  resource two extensions declare is the later one's load error. A
  resource has no `page:`: it opens as itself, and the pages that render
  it say so with `pages[].for` ([extensions-pages.md](./extensions-pages.md)
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
- **A store's own ids.** The registry carries one id recognizer per live
  store of every shared model (`with_record_ids(RecordIds { resource, store,
  store_param, key, pattern, loose })`, from the models' fills:
  `fills.id_pattern`, `fills.loose`, the model's `resource` and
  `store_param`). Core names no model: `tsk42` is
  `oxplow:oxplow/task?id=tsk42&provider=oxtask` because oxtask's fill says
  `id_pattern: "tsk[0-9]+"` and `loose: true`, and a bare `[[some-page]]` is
  a page of oxwiki because its fill's pattern matches it. A `loose` store's
  ids count in running text and `[[…]]` and are tried before paths and shas;
  a store whose ids are not loose counts only inside `[[…]]`, after the sha
  check. `[[<store>:<id>]]` names a record of any store. The first store told
  of wins an id two match (`record_uri_of`, `find_record_uris`). A body's
  mentions come back as one list, `ExtractedRefs.records`; the typed view is
  `Reference::Record { resource, store, id }`, and the link check probes it
  through its resource's `resolve` dataset like any other URI.
- **What names a resource elsewhere** is its name: `ui.about` on a command
  (`about: oxplow/task`), an intent prompt's `about:`, a decorator's
  `resource:`, a fact subject's `subject_kind` (`oxplow/file`), a search
  hit's `resource` and a search filter's `resources`, a provider
  capability's `resource` (`ProviderContract.resource`: a URI with a
  `provider` param is that instance's).

## Resolving and reading

`ref_resolver::resolve_ref(svc, uri)` → `RefSummary { uri, resource,
title, detail, body_excerpt, resolved }`: richer resolvers for commits,
files, directories, knowledge pages and findings; any other resource (a
task too) through its declaration's `resolve` view, by `uri`: its `title`
column, and the view's `state` and `body` columns, when it has them, as
the detail and the excerpt. Core code that needs a resource's rows reads
them through `ref_resolver::resolve_view(svc, resource)` (the view and its
title column), never a dataset by name.
Nothing resolves it → `resolved: false`.

`ref_read::read` (`oxplow_read`, `records.read`) returns `{ uri, resource,
row, related, content, truncated }`: the `resolve` row, the related lists
of tasks, commits and requests (and of any model's item: each of the
model's children by its name, `SELECT * FROM v_<ns>_<model>_<child> WHERE
parent = ?1`; a name core lists keeps core's), a file's text at its `rev` (its
`#lines=` view), an event's or model call's body (`#part=`), a page's
definition.

A resource page (`for:`) is handed the same record as its first loader,
`content` (`pages/content.rs::resolve_content`): a file's `stat` row (text on
request), the `resolve` dataset's row, or `{ uri }` for any other resource.
`ResourceRegistry::resolving_dataset` is the one lookup of a resource's
`resolve` dataset that `ref_read` and `content` share.

`link_check` checks a wikilink's URI against its resource's `resolve`
dataset; `symbol_resolve` finds a symbol now (`v_symbol`) or at its `rev`.

## `[[…]]` sugar

`uri::canonical_wikilink(resources, interior)` turns a wikilink interior
into a URI: a URI passes through when its resource is declared; one of
a store's ids → its task; `git:abc1234` and a bare sha → a
commit; `dir:src/` → `dir?path=src`; `finding:fnd-1`; a declared
`wikilink.prefix` (`pr:12` → `oxplow:github/pr?id=12`); `src/a.rs@HEAD:42`
→ `file?path=src/a.rs&rev=git:HEAD#lines=42`; a kebab slug → an oxwiki page; `<store>:<id>` with an enabled store's id → that store's page.
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
  its URI. The writers are `page_ref_projections` (knowledge pages, notes,
  tasks' bodies, comments and links, findings, commits, and agent turns: a person's
  prompt's references, `ref_type = prompt_mention`, written when the
  prompt is delivered) — each owns its source's slice.
- **`comment`** (V99): `target_uri` (and `target_resource`), a
  `context_chain_json` and `referenced_json` of URIs;
  `comment.written@1` / `comment.deleted@1` carry `{ comment, target }`.
- **The search index**: `search_entry.kind` is the resource and `uri`
  the URI; `kind_search` keeps one index per searchable resource (an
  asset `search:<resource>`), the `search.index` consumer the files.
  A hit is `{ resource, uri, stream_id, title, snippet, score }`.
- **Everything else** stores the URI string where it stored the old ref:
  every dataset's `uri` column, `task.uri` / `parent_uri`, links,
  bookmarks, page
  visits, ui-preference scopes, fact subjects, event subjects and payload
  fields, command inverses and pending proposals.

## The migrations (V99, V100)

`V99__uris.sql` rewrote every stored ref of the old grammar as its URI,
and `V100__bare_ids.sql` moved the fields of "Bare ids" off URIs. Both
are in the squashed baseline now, with the SQL functions and the parser
of the old grammar they used: no code reads the old grammar any more. A
database or a file that still holds an old ref is history: rewrite it by
hand, or leave it. Command inputs and results logged before V99 stay as
they were.

## The rename (V101)

`V101__uri_names.sql` renamed every name that said `ref` for a URI to
`uri`. Tables: `fact_subject.uri`, `search_entry.uri` (was `ref_id`), `bookmark.uri`,
`symbol.uri`, `change_function.uri`.
`wiki_page.file_refs_json` / `dir_refs_json` (the table is `knowledge_page` now) became `files_json` /
`dirs_json`: a knowledge page's citation is a file path, so its names say `files`
(`verify_refs` became `oxplow.refs.verify`, with `files`). Undo
inverses and pending proposals had their input keys renamed (`ref` to
`uri`, `parent_ref`, `evidence_ref`, `refs` and the rest likewise). The
core datasets, command inputs and results, templates (`{{uri}}`,
`{{uri.id}}`, `{{row.uri}}`, `{{param.uri}}`), page params, the
`?uri=` page param and a page `timeline: { uri }` follow.

It left alone: `ref('<dataset>')` in dataset SQL (a dataset reference, not a
URI), git refs (`branch_ref`, `basis_ref`, `ref_moves`), `page_ref` and
its `ref_type`, `v_oxplow_knowledge_ref`, `TabRef`, and internal identifiers
that sit on no wire (`resolve_ref`, `refOpener.ts`, `refFromTabId`).

## The desktop

A tab's id is its URI (`TabRef.id`; a web page's is its URL), and a
`TabRef` is `{ id, pinned? }`: there is no kind and no payload, so the
desktop reads a tab from its URI (`typeOf`, `paramOf`). `apps/desktop/src/resources.ts` reads
`v_resource` (icons, labels, an extension's wikilink prefixes). **A page
is a renderer of a resource**: the daemon says which pages can draw a
URI (`pages/renderers.rs`: `for`, `accepts` over the URI's params, then
`accepts.content` over what it names; `oxplow.page.renderers`), and the
window picks the `#oxplow.renderer=` view key's, else the page marked
`default`, else the first (`pickRenderer`,
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

The activation API freezes declarations and the host task recognizer
from one catalog snapshot before writing. `PreparedVocabulary.replace_tx`
rejects any event, resource, subscription or recognizer diagnostic and
restores its database savepoint; it returns the candidate vocabulary
without swapping `VocabularyHandle`. `PreparedExtensionDatabase` composes
this with datasets and contribution descriptors, restoring the whole
candidate on rejection. When prepared capabilities are included, its
recognizer comes from the planned task-tracker selection rather than the
prior live registry, and capability rows and switch events share that
transaction. Coordinated commit and activation and reader pinning remain
the activation owner's unfinished work.
