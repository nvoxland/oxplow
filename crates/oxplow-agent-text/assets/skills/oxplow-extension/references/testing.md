# Testing the whole extension (`oxplow_preview_extension`, fixtures)

**`oxplow_preview_extension(name, stream_id)`** (MCP; `oxplow extension test
   <name>` from a terminal) runs your whole extension in a throwaway
   oxplow: activated as the app activates it (your entities empty, your
   models, commands, effects, jobs and components live), never the
   project's database, your programs seeing no credentials, and every
   model call answered by the fixture's `model.replies` — never a real
   provider. Run it after `oxplow_validate_extension` and before handing over.
   Its report says what it ran, the generation, what the extension
   `contributes`, and what it can't run here (`unsupported`: pages,
   panels, slots, decorators, replacements, skills — the app shows them).
   Give each example a fixture, `fixtures/<example>.yaml`, with `input`
   and `expect` (`$any` matches anything), and `model: { replies: [...] }`
   in `input` when it calls a model:
   - a lens: `input: { lens: <slug>, params?, rows? }`, `expect: {
     columns?, rows: n }` — `rows: { <entity>: [ {…} ] }` fills your
     collectors' entities first (they start empty);
   - an event: `input: { event: { type, payload, … } }` — logged in the
     throwaway and delivered: your effects (approved there) react for
     real and their commands run. `expect: { effects: [{ id, state }],
     commands: [names] }` (only the keys you name are compared);
   - a job: `input: { job: <id>, intentKey, request, cancel?, setup?,
     rows? }` — `setup: [{ command, input?, as? }]` first runs oxplow's
     own commands (`as: agent` for the agent of `thread:thr1`) to make
     what its tools read: a work item, an effort, a test run — run
     through your commands over `jobs.write/enqueue` and
     `jobs.read/snapshot` (declare both), approved there, its model rounds
     answered by `model.replies` (a string, or `{ text?, toolCalls?: [{
     name, arguments }], chunkDelayMs?, when? }` — a reply with tool calls
     makes the job run them and ask again; one with `when: "<text>"`
     answers only the round whose last message carries that text, such as
     the tool's result); `cancel: true` cancels it through
     your `jobs.write/cancel` command once it has written. `expect: {
     state, result?, text?, toolCalls?, output?: [{ kind }] }`;
   - a component: `input: { component: <id>, lens?, items?: [title],
     queries?: [{ asset, params? }], invokes?: [{ command, input?, approved? }] }` — what its
     frame would ask; `expect: { queries: [{ rows }], invokes: [{ ok,
     refused? }] }` (an invoke is refused until `approved: true`; `items` are work
     items filed first, `work_item:oxplow:<id>`, …);
   - a form lens (`viz: form`): `input: { form: <lens>, params? }` — opens
     it as a person would; `expect: { values?, fixed?, fields? }` (the
     values its fields start from, the read-only fields, the shown ones,
     `null` for all);
   - a collector: `input: { collector: <id>, rows: [...], event? }`
     (`rows` stands in for its `input` query; `event` is the event an
     `on:` collector reacts to), `expect: { entities: { <name>: n } }` —
     it runs your script and types its rows; an exec collector's example
     isn't run (a person approves it);
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
