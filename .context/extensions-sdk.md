# Extensions: The SDK and answerability

The SDK behind `oxplow extension new|check|test` and the answerability check. Part of the extensions docs; the terms and the index of docs are in [extensions.md](./extensions.md).

## The SDK

`crates/oxplow-sdk` (P1.14; target §10.5) is the one
implementation behind three doors: the `oxplow extension new|check|test`
CLI, the agent's `oxplow_validate_extension`, and `write_page`. Every
door gives an author the same report, so an agent editing from a terminal
and one calling MCP read identical `file:line: what — fix` lines.

- **`scaffold_example(root, example, name)`** (`extension new --from <example>`,
  `--list-examples`) copies one of `examples/extensions/*` into the project.
  The SDK carries them (`crates/oxplow-sdk/src/examples.rs`: compiled in by
  `build.rs` in a release build, read from the folder in a debug one, as
  `oxplow-app` does for `extensions/`). The copy keeps the example's name
  (its commands and datasets are named by it); another name is refused:
  rename after copying. `every_example_scaffolds_checks_and_tests_clean`
  keeps each one passing `check` and `test`.
- **`scaffold(root, kind, name, origin)`** (`extension new <kind> <name>`)
  writes `.oxplow/extensions/<name>/extension.yaml` (v2, an `intent` whose `origin` is the URI passed with `--origin`,
  one example) and, but for a bare extension, that example's fixture
  `fixtures/basic.yaml` — then the kind's starter (P7.C6):
  - `page` — `pages/<name>.yaml`, open tasks in the viewer's stream, with
    a Start row action (`oxplow.work_item.transition`, a `uri` column it
    binds but doesn't show) and an `answers:` entry; its example is
    `{ page: <name>, params: { stream_id: 1 } }`;
  - `component` — a `custom_components:` entry (a bundle under
    `components/<name>/`) and a page whose `component` node it feeds;
    `panel` — a `panels:` entry whose `body` is a hidden page;
  - `collector` — a Starlark collector `items` over `v_work_item` declaring
    entity `item`, a dataset `open_items` over `ref('item')` and a page over
    the dataset; its example runs the collector over fixture rows;
  - `metric` — a code metric, built by `oxplow_app::metric_scaffold::
    extension_files` (the one template: `oxplow.metric.scaffold` hands the
    same files to the agent to write). `--language` (default `rust`; the
    ones with a sample: rust, typescript, javascript, python, go, c, cpp)
    and `--glob` (`*` and `**` only; default the language's files) go
    through `scaffold_with` and `MetricStarter`. It declares a per-path,
    semi-additive `<ns>.complex_functions` measure on files, the
    `<ns>.area` dimension (the top folder), a Starlark collector `<ns>.scan`
    on `snapshot.taken` (`source_files()` + `code_metrics()`: one fact per
    file with functions over `LIMIT`, carrying the worst function's line
    and `dims: {<ns>.area}`), the `sum` metric (`lower-better`, sliced by
    the area) and a Code launcher page over `metric_findings()` whose
    `path` column links `file` at the `line` column. Its example runs the
    collector over a two-folder fixture corpus and expects the facts;
  - `command` — `commands: [note]` whose `handlers/note.star` composes
    `oxplow.work_item.comment`, with an example and a `ui` about
    `work_item`; its intent example dry-runs it;
  - `effect` (P8.D12) — `effects: [on-done]` on `work_item.state_changed`
    `where: { to: done }`, whose `effects/on-done.star` composes
    `oxplow.work_item.comment`; its intent example dry-runs it on a fixture
    event;
  - `policy` — an `implementations:` entry (`capability: effort_policy`,
    id = the name with `_` for `-`, `needs: [sql.read]`) whose
    `policies/<name>.star` opens an item's effort when it starts on a
    thread and closes its open efforts (read through `scope`) when it
    ends; its example is a checkpoint expecting `{ skip: $any }`, and it
    passes the effort-policy suite as written;
  - `provider` — below; `extension` — the manifest only.
  It refuses an existing folder, a bad name and an origin that isn't a URI. **What
  it writes passes `check` with no warnings and `extension test` clean**
  (a provider once a program replaces its stub) —
  `crates/oxplow-sdk/tests/just_works.rs` holds each kind to it, through
  to loading in a real oxplow: the page's row action names a registered
  command, the collector syncs and its dataset publishes, the command runs
  through the bus; a work list and an effort policy each pass the kit
  with the fake behind their stub, the policy with its scaffolded example.
- **Recorded fresh-agent runs** (`crates/oxplow-sdk/fixtures/just-works/
  <kind>/`): `scripts/record-just-works.sh <kind>` gives `prompt.md` to
  `claude -p` in an empty git project — `--safe-mode`, so nothing but
  the oxplow-extension skill (appended to its system prompt) and the
  `oxplow` CLI built from the checkout — and keeps `run.json` (without
  the denied commands, which carry local paths, the session id or the
  cost — `recorded_runs_carry_no_local_paths_session_or_cost`), the
  extension it wrote (`produced/`), and `check.txt` / `test.txt`;
  `notes.md` is written by hand. `recorded_agent_runs_still_check_and_
  test_clean` replays every `produced/` against today's oxplow. Recorded
  once (it costs a real run): `collector` (2026-10-02, 64 turns, clean).
- **`check(root, name, catalog, layer: Option<&SqlGateway>)`** is
  `catalog.named` (manifest shape, lifecycle, cross-refs, page shape;
  a Starlark collector's script must parse and define `transform`, an
  error at the collector's line) plus `extensions::validate_extension`'s
  dry run — **always** (P7.C6), and **never on the project's database**:
  on the throwaway oxplow — this build's schema, no data, what the
  extension declares turned on, its datasets and metrics seeded so a page
  may read `metric_grid()` / `metric_findings()` — or on the caller's own
  (`extension test`'s). What happens to be in a project's database is no
  part of a check, and a CLI older or newer than the app is never refused
  over its schema. The agent's `oxplow_validate_extension` checks the same
  way, starting a throwaway in the app. The dry run first compiles, in a rolled-back read,
  the extension's datasets beside the other enabled extensions' (P4.9 —
  resolution, lineage, and the contract, so a changed contract at a
  published version fails here before it publishes) over **empty
  stand-ins for every declared entity that hasn't synced**
  (`datasets::EntityStub`: a typed temp table under the entity's view);
  `datasets::check_extensions` returns those views (`CheckedDatasets.views`).
  Every later query of the check — command examples' reads,
  advisories, pages — reads through them as an **overlay**
  (`SqlGateway::with_overlay`, `SqlQuery::temp_views`: recreated on the
  query's own connection and dropped after), so a fresh collector →
  dataset → page checks clean before its first sync and a page may read
  its own unpublished dataset. The dry run of a page
  (`pages::checks::dry_run`, `extensions::viz_column_problems`) runs its
  `sql` loaders once with every param at its default (an earlier loader
  reads as a table; `command` and `page` loaders do not run, since a
  check acts as no one). A loader that fails is an error. So is a node
  that names a column its loader lacks: a role block's column
  (`chart`, `tree`, …), a column's `key`, `link`, `icon` or `tone`
  column, an action's `when:` or a `{{row.x}}` placeholder. Layout
  problems are warnings. It returns a `CheckReport { ok, errors,
  warnings, dry_run, extension }`; `render_findings` prints it as text
  (`error: <file:line …>` lines then a one-line summary) or JSON.
  `oxplow_validate_extension` also warns about a `fixtures/*.yaml` no intent
  example names (`check_fixture_pairing`; `dataset-*` / `provider-*` belong
  to a dataset or a provider); `extension test` warns about the reverse, an
  example with no fixture (a fresh scaffold has some). Each names the file
  and the fix.
- **`scaffold(…, Kind::Provider { capability }, …)`** (`extension new
  provider <name> [--capability <cap>]`, P5.D5) adds a `providers:` entry
  (id = the name with `_` for `-`, the capability). `Kind::parse("provider")`
  is `work_items`; `Kind::provider(cap)` refuses a capability without a
  provider contract (the CLI exits 2), and `--capability` on another kind
  is a usage error. `provider.json` comes from the capability's
  `ProviderContract`: the verbs it always needs (`confirm: never`,
  `access: record`; every feature off, so no feature-gated verb), the
  events it may emit with `Vocabulary::core()`'s schemas, a collector of
  its records' entity when it streams records, and an object config
  schema. What the contract doesn't say is the SDK's starter per
  capability (`provider_starter`, `verb_input`): each verb's summary and
  input schema, the intent example and its fixture — `create` expecting
  `{ uri: $any }` for a work list; `react` to a checkpoint expecting
  `{ skip: $any }` for an effort policy, whose manifest also `needs:
  [sql.read]` (it hears events only, and reads oxplow for what to close);
  `render` of `{ kind: ack }` expecting `{ body: $any }` for an agent
  harness, whose `launch` / `tool_use` / `render` inputs are the wire's
  (`{ session, endpoints, … }`, `{ body }`, `{ answer }`) and whose
  capability `data` starts as `{ env_markers: [], settings: [] }` (`ProviderStarter.data`; null for a
  contract that asks none). A capability with no starter gets its first verb with `{}` and `$any`.
  Then a stub `bin/provider` that exits 1 and `fixtures/provider-<id>.yaml`
  (`config: {}`).
- **`extension_test::test_extension(root, name, bless)`** (`extension
  test <name> [--bless] [--json] [--junit <file>]`, P5.D5, from either
  binary: `oxplow extension …` and `oxplow-daemon extension …` are the one
  CLI, `oxplow_sdk::cli`) runs on **a throwaway
  oxplow** (`throwaway::Host`) that is a real one: a temp project with a
  copy of the project's `.oxplow/extensions/` and of which it turned off
  (nothing else of its config — its provider instances would start its
  programs), `Services::in_memory_on_machine` with an in-memory keychain,
  its own global config and the environment it is given, activated by
  `ExtensionActivation::initialize` as boot does (the declared entities
  are created empty there; see extensions-activation.md "Database generation candidates"). What
  that activation held back is the report's first lines (this
  extension's an error, another's a warning). `start_runtime` then starts
  what examples need — the effect and collector triggers (each example
  delivers its event with `EventPump::settle`) and the job scheduler —
  and binds every AI role to the **fixture model** (`oxplow_ai::fixture`,
  a host-registered kind no activation replaces): an example's `dataset: {
  replies }` is what it answers, in turn. No paid call, no provider.
  It runs `check`, then every intent example with a fixture
  (`fixtures/<name>.yaml`: `input`, `expect`, `$any` matching anything):
  a page (`{ page, params? }` with `given:` steps, below), an event (`{ event
  }`: appended to the throwaway's log and delivered; the extension's
  effects react for real — `expect: { effects: [{ id,
  state }], commands }`, read from `v_effect_run` and `command.executed`),
  a job (`{ job, intentKey, request, cancel?, setup?, rows? }`: `setup`
  runs oxplow's own commands first, in order, each as a person or `as:
  agent` (thread 1, after `ensure_primary`), so its tools read real data;
  `rows` as a page's `given:` rows; then through the extension's
  commands over `jobs.write/enqueue` and `jobs.read/snapshot`; `cancel: true` cancels it through `jobs.write/cancel` once it
  has written — `expect: { state, result?, text?, toolCalls?, output? }`,
  `output` its rows `[{ kind, … }]` (extensions-jobs.md "Jobs"); a model reply is a string
  or `{ text?, toolCalls?: [{ name, arguments }], chunkDelayMs?, when? }`
  (`when`: answers the request whose last message carries that text);
  naming `effects`, `commands` or `pages`
  also delivers the `contribution.job.completed`
  the job logs, asked in a thread: the effects that reacted, the commands
  they ran and which of the named wiki URIs now exist), a component (`{ component, queries?,
  invokes? }`: `load_component`, `run_component_query`,
  `invoke_component_command` as its frame would — `expect: { queries:
  [{ rows }], invokes: [{ ok, refused? }] }`), a form (`{ form: <page>,
  node?, params? }`: the page's form node — `node`, else its first — opened
  on a run of the page with `pages::actions::page_form`, as a person opening
  it — `expect: {
  values?, fixed?, fields? }`, the values the fields start from, the
  read-only ones and the shown ones, `null` for all), and the dry runs that
  stay dry (`{ effect, event }`, `{ implementation, event }`, `{ command,
  input }`, a derived or fact collector, an event type, a wikilink).
  **One rule compares every `expect`** (`compare`): an object only on the
  keys it names (a key the result lacks matches only `null`, not even
  `$any`); a list every item, in order; `{ contains: [..] }` the items a
  list has, in any order, among others; `$any` any value that's there;
  `{ $exact: v }` the whole value, every key; `refused: <text>` a reason
  that contains the text. **One way to say no**: a command, an invoke or a
  command example that refuses or fails is `refused: <its reason>`. Then, per declared provider, the conformance kit
  ([providers.md](./providers.md) "The conformance kit"), the effort
  policy suite for a policy script, `questions.yaml` and incremental
  datasets. `TestReport { ok, errors, warnings, blessed, ran, generation,
  contributes, unsupported, left }`: `contributes` is the extension's
  descriptors by public id, `unsupported` what only the app shows
  (panels, decorators, replacements, skills; a slot mount runs in its
  host page's run, so a fixture of the host page checks it); `render`
  prints it like `check`'s. Exit 0 / 1 / 2 as `check`. `check` with no
  running oxplow asks a throwaway (with no environment) for its command
  registry. `the_triage_example_reads_the_tree_the_layer_and_a_dataset`
  runs the triage example's job, event and component this way and holds
  its `contributes` equal to a real oxplow's over the same project.
- **The CLI** is `apps/desktop/src-tauri/src/extension_cli.rs`, dispatched
  by `main.rs` before Tauri boots exactly like `oxplow hook`. It takes a
  bare name (under `--root` or the cwd) or the extension folder's path
  (the project is read off `…/.oxplow/extensions/<name>`). `check` never
  opens the project's database (`check_never_reads_the_project_database`);
  only `--impact`, a review of what a change does to the person's pages,
  reads `<root>/.oxplow/local.sqlite` **read-only** (`Database::open_read_only`:
  no migrations, no writes). Exit 0 clean, 1 with errors (or an SDK error such as an unknown
  extension), 2 usage.
- **`oxplow_validate_extension`** returns the `CheckReport`, not
  the bare `Extension`; the extension is inside it.

The `oxplow-extension` skill requires `check` after every edit.

## Answerability

`crates/oxplow-sdk/src/answerability.rs` checks that an agent
reading the right skill can answer the questions oxplow, or an extension,
exists for. The questions live on the **pages** that answer them: a page's
`answers: [{ question, about?, params, skill? }]` (an entry with `about`
is phrased with "this" and offered on pages for that kind of ref). Core has
no questions files: oxplow-foundation's pages carry them, including small
hidden pages that exist to hold a question (`collectors`, `zones`,
`branch-sync`, `work-item-actions`, …), and a question that says "do this"
is held by a page with a whole-page action running that command.

- **`answerability::check_pages`** (with `live_pages` for the live half):
  the answer's `skill` names every dataset the page's `sql` loaders read,
  every command its `command` loaders run and every command its whole-page
  actions run (no `skill`, a user extension's, skips that); the skill's text
  is `skill_full_text`, so a dataset is named by the skill or any of its
  references, as a whole word in any case (`answerability::names`:
  `v_commit_file` doesn't name `v_commit`); a dataset is a table the query
  reads (`datasets_in`: after `FROM`, `JOIN` or a comma in a `FROM` list),
  never a `v_*` word in a string, a comment or an alias. An action's
  command must exist and its input fit the command's schema. The page runs
  with the answer's params with no loader failing (`RunTarget::Spec` over
  the throwaway `Services`), and `about` names a declared resource. A
  question whose skill doesn't lead there is a finding, which is the point:
  when a capability grows, its questions say which skill must learn about
  it.
- **In tree:** `every_page_answer_reaches_what_its_skill_names` runs it over
  oxplow's own extensions (and checks the kinds a person is offered
  questions about: file, commit, effort, work item);
  `a_page_answer_must_be_named_by_its_skill_and_run` the mechanism.
  `oxplow extension test` runs the second half for every extension's pages
  (`ran: answers`).
- **Live** (`OXPLOW_LIVE_ANSWERABILITY=1`): `answerability::live_pages` asks
  this machine's `decide` dataset (`AiService::for_this_machine`: the global
  `ai.yaml`, keychain read only), given only the skill text and the catalog
  (every `v_dataset` view, every command an agent may run and every tool it
  is offered), what it would reach for first; it must pick the page's first
  loader's dataset or command.
- **An extension's own** `questions.yaml` (`answerability::check`) runs in `oxplow extension test`:
  `skill` is a markdown file in the extension, SQL runs on the test's
  throwaway oxplow, and a command is one on its bus — core's or the
  extension's own `commands:` — or one of its providers' `<id>.<name>`
  (schema from its declarations).

**`oxplow extension test <name> [--only <example>,…]`** (`extension_test.rs`;
`--only` runs the check and just the named examples, nothing else — the
quick run while iterating; the text report sums up what ran with the
examples as one count, the JUnit form names each; an example's `expect`
compares numbers by value, so `2.0` holds for `2`) runs on
**a throwaway oxplow** (`Host`, "The SDK" above: activated as boot
activates, its entities created empty, the fixture model answering every
model call). It never reads what runs here — no deployment, no
database — so an extension is tested before its first deploy. **Which
extensions are on is what it declares**: the required
foundation, the one tested, and what each of those `requires:` or imports
from, in turn — and a fixture's own `requires: [<ext>…]` (a fixture key,
same meaning as the manifest's) adds more, on an oxplow of that fixture's
own (an unknown name is a finding). The default extensions are not on:
oxplow's own pages run complete without them. Oxplow's own work list,
effort policy and wiki are foundation's (always on), so a fixture that makes
work items, efforts or wiki pages needs nothing extra (`requires:
[oxplow-bundled]` only for what bundled adds: its review pages, verdict and
replacements), and an effort policy's conformance suite, which files work,
runs on an oxplow of its own — deployed in that order,
what one requires before it (`Host::start`). Never the project's list or
one person's `enableExtensions` / `disableExtensions`, and never a
sibling it doesn't name: a fixture passes or fails the same for everyone
and in CI, and a broken neighbour is no noise in its report. One named
but neither here nor shipped isn't turned on (an import of it reports it
isn't here). Each source is
copied to where the checkout keeps it
(`extensionSources`, so oxplow's own land at `extensions/<name>/`), and
every finding names that path — the file the author edits. It never opens
the project's database. It runs `check` there
(with that registry, so `commands:` examples dry-run), then — only if `check` is clean — each
`intent.examples[*]` by its fixture `fixtures/<name>.yaml` (the event,
job and component forms are in "The SDK"). A fixture's keys are `given`
(steps run first), `when` (what it does: one key names the kind, the rest
are that kind's), `expect`, `steps`, `view` and `model` (`{ replies }`, the
fixture model's answers); any other key is an error (`FIXTURE_KEYS`).
**The kinds are one list** (`extension_test::KINDS`: each kind's key, its
`when` and its `expect`): the runner's "fix:" hints and the skill's table
(`references/testing.md`, between `<!-- fixture-kinds -->` markers) are
made from it, and `the_skill_lists_every_fixture_kind_the_runner_runs`
fails when the table is stale (`OXPLOW_BLESS=1` writes it). A new kind is
a `KINDS` entry and a `dispatch` arm. The forms:

- `when: { page: <slug | ext/slug>, params? }` with optional `given: [..]`,
  `view: { stream?, thread? }` and `steps: [..]` beside `when`, and
  `expect: { loaders?: { <name>: { rows: n | $any | [ { col: val } | [ val,
  … ] ] | { contains: [..] }, columns? } }, nodes?: [{ id, kind?,
  actions? }], text?, alert?,
  badge?, inactive?, actions? }` — the page runs (`PageRunner`, as a person,
  from the `view`'s stream and thread) after the `given` steps, each `{
  command, input?, as?, confirmed?, bind? }` or `{ rows: { <entity>: [..] }
  }`, in order. Everything is compared by the one rule: a row is its
  columns by name (or its cells, where the example writes the row as a
  list), `rows: n` a count; `nodes` ids are in the
  run's tree (`<embed>/<id>` inside an embed); `alert`, `badge` and
  `inactive` are the run's own. `actions`
  (the page's, or a node's viz's) list ids, or objects compared on the
  fields they name, spelled as the page YAML spells them (`one_row`, which
  oxplow derives, included) — `actions_like`. A loader that fails
  is a finding naming it. `expect.text` is `{ contains: [..] }` or the exact
  text, against `page_text`.
  - **Bind and reuse.** `bind: <name>` on a step keeps its command's result;
    `$name.path` (`.key`, `[n]`) anywhere in a later step's `input`, the
    `params` or an `expect` is that value, its type kept (`$any` is never a
    name). A name nothing bound is a finding that says to bind it.
  - **Scenarios.** `steps:` runs after the first check (no top-level
    `expect`: no first run): `{ given: [..] }` changes the state, `{ expect,
    params?, bind? }` runs the page again — `params` over the example's,
    `bind: { <name>: loaders.<loader>.rows[0].<column> }` keeping a value of
    that run — and findings name the step (`steps[3]: …`).
  - **Materialized datasets are current when read.** The throwaway runs
    the change loop as boot does (`datasets_changed::spawn`), and before a
    read — a page run, a query, a command (`Host::current`) — after any
    step changed something, every asset recomputes at once, each after
    those it reads, a clocked one (`every: 1h`) too, without waiting out
    the coalesce window (`Assets::recompute_all`): a test has no clock to
    wait on. Once per read, not per step, so it costs the run nothing
    measurable.
  - **Inside embeds and mounts.** `loaders: { <embed>/<loader>: .. }`
    names a loader of the page an embed node or a slot mount runs (a
    mount's id is its page's with `/` as `.`: `beta.extra/words`), so a
    mounting extension's fixture runs the host page (`when: { page:
    host/items }`) and checks what it mounted (`loader_at`).
  - **A step acts as a person would** (`PageAt::act`): `{ press: { action,
    node?, row?: { <column>: value }, input?, confirmed? } }` presses one of
    the page's actions on an open run (`run_page_action`), `{ submit: {
    node, input, confirmed? } }` submits a form node (`submit_page_form`);
    both run for real and settle. What it gave — `{ ok: true, result }` or
    `{ ok: false, refused }` — is checked against the step's `expect`;
    without one, a no is a finding. `bind` reads it (`result.<key>`).
  - **Every kind has them** (`scenario`): a fixture that isn't a page's
    runs its `given` steps first, its `when` and `expect` read the values
    they bound, and each `{ when?, expect }` step runs the same kind
    again, its `when` keys over the fixture's. A step whose `when` names
    another kind (`kind_of`) runs that instead — an event, then a `sql`
    read of what it did — and in a page's scenario a step with a `when`
    runs it (another page, a query, a command) and the scenario goes on
    (`step_run`).
  - **`confirmed: true`** runs a command that asks a person first.
  - **What a step sets off lands before the next step.** After each
    command step the host settles (`Host::settle`): every async consumer
    hears what was logged (`EventPump::settle_all`) — the config reactors,
    registered as at boot, among them — and a generation one asked for is
    published (`ExtensionActivation::serve_pending`, what the owner does
    after its debounce). So `oxplow.capability.choose` is in effect for
    the next run, as it would be for a person.
  - **One oxplow per example that makes data.** An example with `given`,
    `steps`, `rows` or `setup`, or one that runs a job, logs an event or
    files a component's items (`makes_data`), runs on a fork of the
    throwaway
    (`Host::fork`: its files copied, and its database as the deploys left
    it — `Database::copy_in_memory` of a snapshot taken before anything
    ran — so the fork boots what was deployed and nothing is built or
    deployed again; ~0.8 s, most of it that boot's activation), so no
    example sees another's data. Being independent, they run side by side
    (spawned tasks, `side_by_side()`: half the cores, at most four), and
    their findings join the report in the examples' order after the
    shared host's; the CLI's runtime has threads for it. The rest — `check`, the other examples, the answers — share one.
  - **Answers.** After the examples every page's `answers:` entry runs on
    that shared, clean host (`ran: answers`): the page opens with its params
    and no loader fails, else the finding names the page file and the
    question.
  - **The report's cases.** `TestReport.cases` holds a `TestCase { name, ok,
    message, duration_ms }` per thing that ran (`check`, `example <name>`,
    `answers <page>`, `provider <id>`, `policy <id>`), and
    `render(report, Format::Junit)` writes them as one `<testsuite>`.
- `when: { command, input?, as?, confirmed? }`, `expect: { ok, result?,
  commands? }` or `{ ok: false, refused }` — any command run for real
  through the bus as a person (or `as: agent`), then settled
  (`command_run`): what it answered (a script's own answer, unwrapped from
  its composite's `{ result, children }`), and `commands`, the ones that
  ran because of it — the script's steps, then what reacted (audited after
  it). The extension's own command with `answers` is still the dry run
  below, until those fixtures make their data.
- `given: [{ observe: { turn: { prompt?, during?, tools?, write?, answer?, compactions?, context? } } }]`
  (`observe` in `extension_test.rs`; every kind takes `given` and `steps`) —
  what oxplow only learns from what happens around it, made through the API
  core makes it with, and a new kind is added to that `match`, red test
  first, one at a time. `turn` is one turn on the primary thread through
  `hook_ingest.ingest`: `UserPromptSubmit`, the commands made `during` it,
  one `PostToolUse` per tool (`ToolUse` as the harness maps it), the file
  written with a snapshot before and after (`SnapshotTrigger::Startup`,
  `enqueue_startup_diff`), then `Stop` with the answer
  (`last_assistant_message`). The log's projections are delivered between
  the calls and the end (the checkpoint counts the turn's writing tools
  from `v_tool_call`) and settled after it (the checkpoint, then what reacts
  to it). The other kinds: `event` (logged and delivered; effects react),
  `evidence` (`EffortEvidenceStore::replace`), `commit`
  (`SqliteGitStore::upsert_commit`), `change` (a change's files and
  functions, `SqliteChangeStore::store_results`; it never runs a deep
  analysis, and the materialized datasets over the commit index, like
  `co_change_pair`, fill on a recompute the throwaway doesn't start),
  `test_run` (a `tests` capture and its `oxplow.test_case` facts through
  `SqliteFactStore::record_facts`) and `snapshot` (one take in the
  snapshot log, `SqliteSnapshotStore::record_take`), `diagnostics`
  (`SqliteDiagnosticStore::replace_file`), `symbols` (`SqliteSymbolStore::record`
  at a snapshot taken with the file), `collect` (`run_effort_collectors`) 
  `run` (`extension_run_store::record_tx`; `days_ago` through
  `RunContext::began_at`), `health` (`disable_tx`), `token_usage`, `hint`
  and `nudge` (their stores), a `project:` given (keys of
  `.oxplow/project.yaml`, then `reload_config_from_disk`), and a `turn` that
  also takes `thread`, `session`, `prompt: null` + `subagent` (a hand-back)
  and `pre: true` on a tool; none can set the time of a hook (a tool call
  or a turn is as old as the run), only a `run`'s `days_ago`,
  `analyze` (`change_analysis::ensure_change`: a working or commit change
  analyzed to the end; real files come from `turn.write` and real commits from
  `oxplow.vcs.stage` / `oxplow.vcs.commit`, a throwaway's forks keep their
  streams in their own folder),
  `refs` (branches, tags and stream heads, `replace_branches` /
  `replace_tags` / `replace_stream_heads`),
  `visit` (a page the person opened, `PageVisitStore::record`),
  `facts` (findings of a declared measure through `record_facts`) and
  `retention` (`event_retention::sweep`, run `days` ahead). A step's `when: { sql }` reads the result, since a `steps` entry
  of a command, event or sql fixture takes the earlier kind in `KINDS`.
- `when: { advisory: <id>, thread? }`, `expect: { lines: [..] }` — the
  extension's advisory run for a thread (the primary one when none) as
  the app runs it on its `on:` (`advisories::for_thread`, over the
  thread's open effort), its text line by line, heading included; on an
  oxplow of its own, since it records what it said (`once_per`).
- `when: { sql, params? }`, `expect: { rows, columns? }` — a read-only query
  over the published datasets, run as a page loader would (an inline page of
  one `sql` loader, `sql_example`) and compared as a loader's: a dataset, a
  decorator's labels or a metric's `metric_grid` tested on its own.
- `when: { collector: <id>, rows?, event? }`, `expect: { entities: { <name>:
  n | $any } }` — a derived collector runs over `rows` (standing in for
  its `input` query, `oxplow_preview_collector(…, event, rows)`) and `event`
  (the event an `on:` collector reacts to, as the envelope), storing nothing, its
  rows typed against the declaration; a collector that calls a model
  (`ai.compute`) runs in the throwaway against the fixture
  model (`model: { replies }`, in the form the role expects, e.g. `{ answers:
  { label: { type: choice, choice } } }` for `ai_classify`); an exec or
  provider-reading collector's example is a warning, not run (it runs a
  program);
- `when: { collector: <id>, files: { <path>: <text> } }`, `expect: {
  facts: [...] }` — a **fact** collector (one with `facts:`) runs over
  `files` as its tree (`MetricsService::preview_fact_collector`, its other
  input keys — `report`, `rows`, `event` (made the envelope) — as its
  input), storing nothing;
  each fact is compared on the keys its expected fact names (it is not run as an
  entity collector);
- `when: { command, input }` — a provider's, run in its session
  (providers.md "The conformance kit");
Every example's `event:` is built by `EventView::fixture` through the
throwaway's vocabulary (core's plus the extensions' declared types,
synced at its start) — extensions-events.md "The event envelope" above: the script gets the
envelope the runtime would hand over, and a fixture that couldn't happen
is an error at it (``fixtures/x.yaml:1: example `x`: event: …``). A
review reads each fixture through that version's own declared types
(`Vocabulary::with_declared`), so a version that adds v2 carries its v1
fixtures to it.

- `when: { effect: <id>, event: { type, payload, subject? }, answers? }`,
  `expect: { commands: [names] } | { skip: <part of the reason> } | {
  reacts: false }` (P8.D12) — whether the effect reacts (`on`/`where`),
  then `effects::dry_run`: its script, its scope calls answered
  from `answers` (per scope, in call order) or for real, the
  composed calls checked against the registry; nothing runs. An
  extension command's is `when: { command: <id>, input, answers? }`,
  `expect: { commands?, result? }` (each compared on the keys it names:
  a read command's example checks what it answers) or `{ refuses }`;
  `{ skip: $any }` matches any skip (`reaction_problem`, shared with the
  policy's);
- `when: { implementation: <id>, event: { type, payload, subject?,
  anchors? }, answers? }`, the same `expect` but `reacts` — an
  **effort policy script** (`implementations:` with `capability:
  effort_policy`): the event envelope, `effects::run_over` with the answers, the
  calls checked against the registry; appending events is an error (a
  policy emits nothing). Naming no policy script of the extension is an
  error naming the id. For each such script `test` also runs the
  effort-policy suite (`policy_suite`): on a fork of the test's
  throwaway (as a provider's suite runs), the
  script chosen as `activeProviders.effort_policy`, then
  `effort_policy_conformance::suite`; a finding is an error at the
  manifest, `ran` shows `effort_policy suite`;
- `when: { event_type, v?, payload }`, `expect: { valid: true | false,
  upcast?: <payload> }` — the payload against the extension's declared
  schema (the newest version without `v`), and its upcast;
- `when: { wikilink }`, `expect: { uri: <canonical URI> | null }` — what
  `[[…]]` names with the extension's resources beside core's;
- no fixture is a warning; an `input` naming none of these is an error.

Then `questions.yaml`, each provider's conformance kit, and (P8.B5) each
`materialize: { incremental: <column> }` dataset on its fixture
`fixtures/dataset-<name>.yaml` — `before:` and `after:`, each `{ <table>:
[{ column: value }] }`: in a rehearsal (rolled back), `before` is written
and the dataset built whole into a temp table keyed like its `m_<view>`,
`after` is written, the rows past its watermark are appended, and what
it holds is compared with its SELECT's rows now
(`datasets::incremental_matches_full`). A difference is an error at the
dataset's line in `extension.yaml` ("misses n row(s) a full refill holds"
— a watermark that doesn't grow as rows arrive drops a row below it
unseen, which nothing at run time can tell); an append that hits the key
passes (the runtime refills then); no fixture is a warning.
