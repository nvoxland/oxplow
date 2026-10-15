# Checking an extension


**Run `check` after every edit, before anything else.** Not once at the
end: after each file you write or change. It is cheap, and the message
names the file and line and says what to change.

1. `validate_extension(name, stream_id)` (MCP), or `oxplow extension check
   <name>` from the worktree, is the same check and the same report:
   - manifest errors (YAML, unknown keys, a name/folder mismatch, a
     missing `intent`, a `shared` extension without `engine` or `version`, a
     Starlark collector script that doesn't parse or define `transform`);
   - cross-references that don't resolve (a slot mount naming a lens
     that doesn't exist, a link kind nobody registered, a command that
     doesn't exist or an input that doesn't fit it);
   - its models compiled without publishing (SQL errors, a `ref()` or
     `source()` that doesn't resolve, columns that differ from the
     declared contract, a changed contract at a published version);
   - a dry run of every lens, advisory and command `input` with its
     default params (SQL errors, and `columns` keys the query doesn't
     return), and of every command example.

   It **always** dry-runs, and what you declared but haven't run yet is
   there for it: a collector's entities stand in empty, and your own
   models are compiled for the check. So collector → model → lens checks
   clean before the first sync — don't sync just to make `check` pass.
   (The CLI uses the project's database when it has one, else an empty
   one.) `ok: true` (exit 0) means it works. Fix every `error`; fix a
   `warning` unless you can say why not.
2. `run_lens(id, params?, stream_id, thread_id?)` returns exactly the
   rows the user will see. Check that they answer the question. It reads
   your extension's models as your worktree has them, published or not.
3. Pass **your own `stream_id`** to both when you're in a worktree
   stream. Extensions are read from the stream's worktree, so the primary
   stream won't see yours until it's merged.

4. `oxplow extension test` (see `references/testing.md`) also runs the extension's `questions.yaml`, if it
   has one: questions an agent should be able to answer with it, each
   `{ question, skill, reaches: { sql } or { command, input }, shape:
   { columns } }`. `skill` is a markdown file in the extension (a
   README); it must name every view the SQL reads and the command it
   runs (core's, the extension's own, or a provider's), the SQL must
   return exactly `shape.columns`, and the input must fit the command.
   Write them for what the extension is *for*, whatever its kind.

An extension with an external provider is tested as `references/providers.md`
says; the whole extension is run in a throwaway oxplow as
`references/testing.md` says.
