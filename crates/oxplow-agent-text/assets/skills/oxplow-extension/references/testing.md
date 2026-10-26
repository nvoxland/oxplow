# Checking and testing an extension

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
from the declared contract); and a dry run of every page's loaders with their
default params, every advisory and every command example. It **always**
dry-runs: a collector's entities stand in empty, so collector → model → page
checks clean before the first sync. `ok: true` means it works; fix every
`error`, and every `warning` unless you can say why not.

Then **`oxplow_run_page { page, draft?, params? }`** returns exactly what the
person will see. In a worktree stream pass `draft: true` (it reads your
source over live data; its commands are the deployed ones) and your own
`stream_id`. The source runs nothing until `oxplow.extension.deploy`.

An extension's own `questions.yaml` (`{ question, skill, reaches: { sql } |
{ command, input }, shape: { columns } }`, `skill` a markdown file in the
extension) also runs under `oxplow extension test`: the skill must name every
model the SQL reads and the command it runs. Prefer `answers:` on pages.

## Testing the whole extension (`oxplow_preview_extension`, fixtures)

**`oxplow_preview_extension(name, stream_id)`** (MCP; `oxplow extension test
   <name>` from a terminal) runs your whole extension in a throwaway
   oxplow: activated as the app activates it (your entities empty, your
   models, commands, effects, jobs and components live), never the
   project's database, your programs seeing no credentials, and every
   model call answered by the fixture's `model.replies` — never a real
   provider. Run it after `oxplow_validate_extension` and before handing over.
   Its report says what it ran, the generation, what the extension
   `contributes`, and what it can't run here (`unsupported`: panels, decorators,
   replacements, skills — the app shows them).
   Give each example a fixture, `fixtures/<example>.yaml`, with `input`
   and `expect` (`$any` matches anything), and `model: { replies: [...] }`
   in `input` when it calls a model:
   - a page: `input: { page: <slug | ext/slug>, params? }`, with `given:` beside
     `input` — steps run first, in order, on the extension's throwaway
     database: `{ command, input?, as? }` or `{ rows: { <entity>: [..] } }`.
     `expect: { loaders: { <name>: { rows: n | $any | [ { col: val } ],
     columns? } }, nodes: [{ id, kind? }], text: { contains: [..] } | "<exact>" }`
     (a row map compares only the columns it names; `text` is the page's
     text, as `oxplow_run_page` shows it). Every page's `answers:` entry also runs: the page
     must open with its params and no failing loader. Example:
     `given: [{ command: oxplow.work_item.create, input: { title: T } }]`,
     `input: { page: items }`, `expect: { loaders: { items: { rows: [{
     title: T }] } } }`. `answers:` double as fixtures: each runs after the
     examples (`ran: answers`), and a page with no `expect` still has to
     open. One throwaway oxplow serves every fixture of the extension, and
     `given` steps mutate its database in order, so a later fixture sees what
     an earlier one made;
   - `rows: { <entity>: [ {…} ] }` in a page or job fixture fills your
     collectors' entities first (they start empty);
   - an event: `input: { event: { type, payload, … } }` — logged in the
     throwaway and delivered: your effects react for
     real and their commands run. `expect: { effects: [{ id, state }],
     commands: [names] }` (only the keys you name are compared);
   - a job: `input: { job: <id>, intentKey, request, cancel?, setup?,
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
   - a component: `input: { component: <id>, page?, items?: [title],
     queries?: [{ asset, params? }], invokes?: [{ command, input? }] }` — what its
     frame would ask; `expect: { queries: [{ rows }], invokes: [{ ok,
     refused? }] }` (an invoke is `refused` when the command refuses it; `items` are work
     items filed first, `oxplow:oxplow/work_item?id=<id>&provider=oxplow`, …);
   - a page's form (a `viz: form` node): `input: { form: <page>, node?,
     params? }` — opens it as a person would (`node` names the form node,
     else the page's first); `expect: { values?, fixed?, fields? }` (the
     values its fields start from, the read-only fields, the shown ones,
     `null` for all);
   - a collector: `input: { collector: <id>, rows: [...], event? }`
     (`rows` stands in for its `input` query; `event` is the event an
     `on:` collector reacts to), `expect: { entities: { <name>: n } }` —
     it runs your script and types its rows; an exec collector's example
     isn't run (it is a program under `permissions.process`);
   - a fact collector (one with `facts:`): `input: { collector: <id>,
     files: { "src/a.ts": "…" } }` (the tree its `files()` sees),
     `expect: { facts: [{ measure, value, path? }] }` — each fact is
     compared on the keys you name, in order;
   - a command: `input: { command: <id>, input: {...}, answers? }`,
     `expect: { commands: [names], result: {...} }` (either or both;
     `result` is compared on the keys you name, so a read command's
     example checks its answer) or `{ refuses: <part of the reason> }` —
     a dry run; nothing changes;
   - an effort policy script (`implementations:`): `input: {
     implementation: <id>, event: { type, payload, anchors? }, answers?
     }`, `expect: { commands: [names] }` or `{ skip: <part of the
     reason> | $any }` — a dry run; the test also runs the effort-policy
     conformance suite over the script;
   - a provider: `input: { command, input }` (`references/providers.md`).

   An `event:` in an example is `{ type, v?, payload, subject?, anchors?
   }`, with a payload valid for its type (the whole payload, as oxplow
   would log it). The script gets every event field, `null` where the
   fixture gave none and `source: fixture`; a `v` older than the newest
   is carried forward. A fixture that couldn't happen (unknown type,
   newer `v`, a payload that doesn't match, `thread_id` not like `thr1`,
   an extra key) fails with the reason.
   Give a command's own `examples:` `answers:` too, so they don't depend
   on the project's data.
