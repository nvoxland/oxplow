# Refs: one identity for everything

What this doc covers: the canonical ref grammar, the kind registry, and
how refs replace tab ids, `page_ref` kinds and `[[…]]` wikilink shapes.
Target design: [target-architecture.md](./target-architecture.md) §4.
Built so far: the grammar (P1.1, tsk403). The registry, the Rust `[[…]]`
translation and the TS side land in P1.2–P1.3 and are described here as
they arrive.

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
(`apps/desktop/src/refs/ref.test.ts`, P1.3) asserts the same file, so the
two parsers can't drift. Add a case there when you extend the grammar;
never change an existing case's expectation without changing both parsers.

## What refs replace (coming in P1.2–P1.3)

| Today | Canonical |
|---|---|
| `task:tskN` (kind `task`) | `work_item:oxplow:tskN` |
| `git-commit:<sha>` | `commit:<sha>` |
| `dir:<path>` (kind `directory`) | `dir:<path>` |
| `file:<p>:@<frag>` | `file:<p>@<rev>` |
| `metric-detail:<key>` | `metric:<key>` |
| `task-note` | `task_note` |
| bare page ids (`agent`, `settings`) and shell routes (`diff:`, `dup:`, `diff-view:`) | `page:<name>[?params]` |

Decided 2026-09-28: there is no compatibility layer for old ids. Only
Nathan uses oxplow; stale `page_ref`/`page_visit` rows are wiped in V92
and regenerated, and the UI starts new localStorage keys.
