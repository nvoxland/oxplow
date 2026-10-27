# Checking and testing an extension

A page's behavior is tested here, by fixtures in its extension — what its
loaders return after some steps, in which state — not by tests in another
language. `oxplow extension test` runs them in seconds on a throwaway
oxplow, so write the fixture first and watch it fail.

## Check after every edit

**Run `oxplow_validate_extension(name, stream_id)` after every edit**, not
once at the end. It is cheap, and each message names the file and line and
says what to change (`oxplow extension check <name>` is the same report).
It covers: manifest errors (YAML, unknown keys, a name/folder mismatch, a
missing `intent`, a Starlark script that doesn't parse or define
`transform`); cross-references that don't resolve (a slot mount naming a
page that isn't there, a command that doesn't exist or an input that doesn't
fit it, a loader reading one declared after it); the models compiled without
publishing (SQL errors, a `ref()` that doesn't resolve, columns that differ
from the declared contract); and a dry run of every advisory and every command
example. It **always** dry-runs: a collector's entities stand in empty. It
does not run a page's loaders: `oxplow_run_page` and `extension test` (which
opens each page with its `answers:` params) do, so a bad query in a page
shows there. `ok: true` means it works; fix every
`error`, and every `warning` unless you can say why not.

Then **`oxplow_run_page { page, draft?, params? }`** returns exactly what the
person will see. In a worktree stream pass `draft: true` (it reads your
source over live data; its commands are the deployed ones) and your own
`stream_id`. The source runs nothing until `oxplow.extension.deploy`.

An extension's own `questions.yaml` (`{ question, skill, reaches: { sql } |
{ command, input }, shape: { columns } }`, `skill` a markdown file in the
extension) also runs under `oxplow extension test`: the skill must name every
model the SQL reads and the command it runs. Prefer `answers:` on pages.

## Testing the whole extension (`extension test`, fixtures)

**`$OXPLOW_BIN extension test <name>`**, from your terminal in your own
   checkout (a worktree's draft as it is; `OXPLOW_BIN` is set in your
   session, else run `oxplow`), runs your whole extension in a throwaway
   oxplow, in a process of its own: activated as the app activates it
   (your entities empty, your models, commands, effects, jobs and
   components live), never the project's database or what is deployed,
   with the project's extensions on (not the person's own choices), and
   every model call answered by the fixture's `model.replies` — never a
   real provider. It works before the extension's first deploy. Run it in
   the foreground after `oxplow_validate_extension` and before handing
   over; `--junit <file>` also writes a report for the Tests panel, `--json`
   the full report. It exits 1 when anything failed.
   Its report says what it ran, the generation, what the extension
   `contributes`, and what it can't run here (`unsupported`: panels, decorators,
   replacements, skills — the app shows them).
   **A test runs your extension with what it declares**, plus oxplow's
   own (foundation, and the extensions on by default): another
   extension you read, mount at or run commands of must be in
   `requires: [name]` (or imported) — `check` says so when it isn't.
   **How `expect` is compared, everywhere:** an object only on the keys it
   names (a key the result lacks matches only `null`); a list every item,
   in order; `{ contains: [..] }` items a list has, in any order, among
   others; `$any` any value that's there; `{ $exact: v }` the whole value;
   `refused: <text>` a reason containing the text (anything that refuses
   or fails says `refused`). So a row `{ title: A }` checks its title only.
   Give each example a fixture, `fixtures/<example>.yaml`: `given` (steps
   run first), `when` (what it does — one key names the kind), `expect`
   (`$any` matches anything), and `model: { replies: [...] }` when it calls
   a model. Any other key is an error:
   The kinds of fixture (generated from the runner's own list):

<!-- fixture-kinds -->
| kind | `when` | `expect` |
|---|---|---|
| page | `{ page: <slug \| ext/slug>, params? }` | `{ loaders?, nodes?, text?, alert?, badge?, inactive?, actions? }` |
| form | `{ form: <page>, node?, params? }` | `{ values?, fixed?, fields? }` |
| job | `{ job: <id>, intentKey, request, cancel?, setup?, rows? }` | `{ state, result?, text?, toolCalls?, output?, effects?, commands?, pages? }` |
| component | `{ component: <id>, items?, invokes?: [{ command, input? }] }` | `{ invokes: [{ ok, result? } \| { ok: false, refused }] }` |
| event | `{ event: { type, v?, payload, subject?, anchors? } }` | `{ effects?: [{ id, state }], commands?: [names] }` |
| effect | `{ effect: <id>, event, answers? } (a dry run)` | `{ commands } \| { skip } \| { reacts: false }` |
| implementation | `{ implementation: <id>, event, answers? } (a dry run)` | `{ commands } \| { skip }` |
| event_type | `{ event_type, v?, payload }` | `{ valid, upcast? }` |
| advisory | `{ advisory: <id>, thread? }` | `{ lines: [text] }` |
| sql | `{ sql, params? }` | `{ rows, columns? }` |
| wikilink | `{ wikilink }` | `{ uri }` |
| collector | `{ collector: <id>, rows?, event? } \| { collector: <id>, files }` | `{ entities?, events? } \| { facts }` |
| command | `{ command, input?, as?, confirmed? } (with answers: the extension's own, a dry run)` | `{ ok, result?, commands? } \| { ok: false, refused }` |
<!-- /fixture-kinds -->

   - a page: `when: { page: <slug | ext/slug>, params? }`, with `given:` —
     steps run first, in order: `{ command, input?, as?,
     confirmed?, bind? }` or `{ rows: { <entity>: [..] } }`.
     `expect: { loaders: { <name>: { rows: n | $any | [ { col: val } ] |
     { contains: [ { col: val } ] }, columns? } }, nodes: [{ id, kind?,
     actions? }], text: { contains: [..] } | "<exact>", alert?, badge?,
     inactive?, actions? }` (a loader of an embedded or slot-mounted page
     is `<node id>/<loader>` — a mount's node id is its page's with `/` as
     `.`, so test a mount from the host page: `when: { page: host/items }`,
     `loaders: { beta.extra/words: { rows: 1 } }`; `text` is the page's text, as `oxplow_run_page`
     shows it; `actions`, the page's or a node's, list ids, or objects
     compared on the fields they name, spelled as in the page YAML:
     `{ id: rename, form: true, one_row: true }`).
     `view: { stream: str1, thread: thr1 }` runs it from that stream and
     thread. `bind: <name>` on a step keeps the command's result, and
     `$name.uri` (any `.key` / `[n]` path) in a later input, `params` or
     `expect` is that value. `confirmed: true` runs a command that asks a
     person first. Example:
     `given: [{ command: oxplow.work_item.create, input: { title: T }, bind: t }]`,
     `when: { page: items, params: { uri: $t.uri } }`, `expect: { loaders:
     { items: { rows: [{ title: T }] } } }`.
     **Scenarios:** `steps: [ { given: [..] } | { expect, params?, bind? } ]`
     after it — each `expect` runs the page again (`params` over the
     example's; `bind: { c: 'loaders.items.rows[0].uri' }` keeps a value of
     that run), so one fixture can change the state and check the page
     between changes. **A custom component is tested by a `ui:` step**:
     `- ui: [ { expect: { count: { li: 1 } } }, { click: "#go" }, { expect: {
     invoked: [{ command: <yours>, ok: true }] } } ]` runs the page's component
     in the oxplow app's own webview against this test's data — `click`,
     `fill: { "#t": text }`, `key: { on, key }`, `drag: { from, to }`, `press:
     <nav-bar action id>`, `view_params`, `given`, `confirm: false`, `form: {
     submit: {..} }`, and `expect: { visible, count, text, title, actions,
     view_params, invoked, navigated, confirmed, forms, shown }` (waited for,
     5 s). It needs the app (`OXPLOW_SHELL`, or the `oxplow` beside
     `$OXPLOW_BIN`); without it the report warns that the steps didn't run.
     Write the `ui` fixture first, then the component. A step can also act as a person would: `{ press: {
     action, node?, row?: { uri: $t.uri }, input? } }` presses one of the
     page's actions, `{ submit: { node, input } }` submits a form node —
     for real; its `expect` is `{ ok: true, result? }` or `{ ok: false,
     refused: <part of the reason> }` (none: a no fails the step), and the
     next `expect` step shows what it changed. What a step sets off (a reaction to its event, a
     setting taking effect) has happened before the next step. Each
     example that makes data gets an oxplow of its own: no fixture sees
     another's data, so a fixture makes everything it reads. Every kind
     below takes `given:` and `steps:` the same way; a step of a kind
     that isn't a page is `{ when?, expect }`, its `when` keys over the
     fixture's (`when: { payload: { .. } }` changes just the payload). A
     step's `when` that names another kind or page runs that instead:
     `- when: { sql: "SELECT …" }` reads what an event or command did,
     `- when: { page: other }` checks another page, and the scenario goes on. Every page's `answers:` entry
     also runs (`ran: answers`), on a clean oxplow: the page must open with
     its params and no failing loader;
   - a `given` step can also **observe** what oxplow only learns from what
     happens around it, made through the API core makes it with:
     `{ observe: { turn: { prompt?, during?: [{ command, input?, as? }],
     tools?: [{ name, kind, paths?, command?, detail?, ok?, exit_code? }],
     write?: { path, body }, answer? } } }` — one turn on the thread, as its
     harness's hooks report it: the prompt, the commands made `during` it
     (an effort opened mid-turn), each tool call (a finished call; a shell
     `cargo test` also records a test run), the file the turn wrote (with a
     snapshot before and after, so the turn's checkpoint says `changed`) and
     the turn's end with the agent's last message. Read the result with
     `when: { sql }` (`v_tool_call`, `v_agent_turn`, `v_effort`, …).
     Other kinds: `{ observe: { event: { type, payload, … } } }` (an event
     oxplow logs on its own, your effects reacting), `{ observe: { evidence:
     { effort, observations: [{ kind, metric_value?, payload? }] } } }` (an
     effort's `test-run` and `diff-coverage` evidence) and `{ observe: {
     commit: { sha, at, subject, files?: [{ path, status?, additions?,
     deletions? }] } } }` (a commit the git indexer read), `{ observe: {
     change: { kind?, target, files?: [{ path, status?, additions?,
     deletions?, zone?, is_test? }], functions?: [{ path, symbol, status?,
     body_changed?, length?, params_before?, params_after?,
     complexity_delta?, … }] } } }` (what the change analysis stored for a
     change: `kind` is `commit` by default, an effort's `target` is its id
     like `eff1`; the models over a change read it, and the
     materialized models that read the commit index need a step of their
     own to fill), `{ observe: { test_run: { effort, at?, cases?: [{ name,
     status }], asserted_failed? } } }` (a test run an effort recorded: a
     capture with a fact per case) and `{ observe: { snapshot: { trigger?,
     elapsed_ms?, budget_ms?, files?: [{ path, hash }] } } }` (one take in
     the stream's snapshot log; its parent is the take before it,
     `{ observe: { diagnostics: { path, items: [{ severity, message, line?,
     col? }] } } }` (what a language server reported for a file),
     `{ observe: { symbols: { path, items: [{ name, kind, container?, line? }] } } }`
     (the definitions the index read from it), `{ observe: { collect: {
     effort, collectors?: [key] } } }` (the fact collectors an effort's end
     runs) and `{ observe: { retention: { days } } }` (the log's retention
     sweep as it runs that many days from now: payloads expire, envelopes
     stay);
   - `rows: { <entity>: [ {…} ] }` in a page or job fixture fills your
     collectors' entities first (they start empty);
   - an event: `when: { event: { type, payload, … } }` — logged in the
     throwaway and delivered: your effects react for
     real and their commands run. `expect: { effects: [{ id, state }],
     commands: [names] }` (`commands` are the ones an effect's run
     made, its steps in order, not the sequence that carried them; only the keys you name are compared);
   - a job: `when: { job: <id>, intentKey, request, cancel?, setup?,
     rows? }` — `setup: [{ command, input?, as? }]` first runs oxplow's
     own commands (`as: agent` for the agent of `oxplow:oxplow/thread?id=thr1`) to make
     what its tools read: a work item, an effort, a test run — run
     through your commands over `jobs.write/enqueue` and
     `jobs.read/snapshot` (declare both), its model rounds
     answered by `model.replies` (a string, or `{ text?, toolCalls?: [{
     name, arguments }], chunkDelayMs?, when? }` — a reply with tool calls
     makes the job run them and ask again; one with `when: "<text>"`
     answers only the round whose last message carries that text, such as
     the tool's result); `cancel: true` cancels it through
     your `jobs.write/cancel` command once it has written. `expect: {
     state, result?, text?, toolCalls?, output?: [{ kind }] }`;
   - a component: `when: { component: <id>, items?: [title], invokes?: [{
     command, input? }] }` — what its frame would ask; `expect: { invokes: [{
     ok, result? } | { ok: false, refused }] }` (an invoke is `refused` when the
     command refuses or fails; `items` are work
     items filed first, `oxplow:oxplow/work_item?id=<id>&provider=oxplow`, …);
   - a page's form (a `viz: form` node): `when: { form: <page>, node?,
     params? }` — opens it as a person would (`node` names the form node,
     else the page's first); `expect: { values?, fixed?, fields? }` (the
     values its fields start from, the read-only fields, the shown ones,
     `null` for all);
   - a command, run for real: `when: { command, input, as?, confirmed? }`,
     `expect: { ok: true, result?, commands?: [names] }` or `{ ok: false,
     refused: <part of the reason> }` — `commands` are the ones it ran (a
     script's) and what reacted. Make what it needs with `given:` first.
   - a query: `when: { sql: "SELECT … FROM v_<ext>_<model>", params? }`,
     `expect: { rows: [ { col: val } ] }` — reads the published models as a
     page loader would, for a model, a decorator's labels or a metric
     (`metric_grid`) on its own.
   - a collector: `when: { collector: <id>, rows: [...], event? }`
     (`rows` stands in for its `input` query; `event` is the event an
     `on:` collector reacts to), `expect: { entities: { <name>: n } }` —
     it runs your script and types its rows; an exec collector's example
     isn't run (it is a program under `permissions.process`);
   - a fact collector (one with `facts:`): `when: { collector: <id>,
     files: { "src/a.ts": "…" } }` (the tree its `files()` sees),
     `expect: { facts: [{ measure, value, path? }] }` — each fact is
     compared on the keys you name, in order;
   - a command: `when: { command: <id>, input: {...}, answers? }`,
     `expect: { commands: [names], result: {...} }` (either or both;
     `result` is compared on the keys you name, so a read command's
     example checks its answer) or `{ refused: <part of the reason> }` —
     a dry run; nothing changes;
   - an effort policy script (`implementations:`): `input: {
     implementation: <id>, event: { type, payload, anchors? }, answers?
     }`, `expect: { commands: [names] }` or `{ skip: <part of the
     reason> | $any }` — a dry run; the test also runs the effort-policy
     conformance suite over the script;
   - a provider: `when: { command, input }` (`references/providers.md`).

   An `event:` in an example is `{ type, v?, payload, subject?, anchors?
   }`, with a payload valid for its type (the whole payload, as oxplow
   would log it). The script gets every event field, `null` where the
   fixture gave none and `source: fixture`; a `v` older than the newest
   is carried forward. A fixture that couldn't happen (unknown type,
   newer `v`, a payload that doesn't match, `thread_id` not like `thr1`,
   an extra key) fails with the reason.
   Give a command's own `examples:` `answers:` too, so they don't depend
   on the project's data.
