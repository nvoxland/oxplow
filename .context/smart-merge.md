# Smart conflict auto-resolution

Oxplow tries to clear merge/rebase/cherry-pick/revert conflicts that
aren't *real* overlaps before the user ever sees them — the IntelliJ
"magic wand" / Mergiraf idea. Two tiers, layered weakest-assumption
first. **Invariant for every tier: we only ever *reduce* the conflict
count. Git has already produced its markers by the time we run, so the
worst case is identical to plain git; we never invent content and never
silently resolve a true semantic overlap.**

## Tier 1 — token-level diff3 (shipped)

`crates/oxplow-git/src/smart_merge.rs`. Language-agnostic, MIT, no new
deps. Splits base/ours/theirs into tokens (word runs, whitespace runs,
newlines, single punctuation — losslessly: `tokenize(s).concat() == s`)
and runs a classic diff3 over the token streams. Two edits to *different
words on the same line* land in non-overlapping token regions → clean
merge, where git's line-granular driver reported a conflict.

`auto_resolve_conflicts(repo)` walks `Index::conflicts()`, and for each
modify/modify entry that's UTF-8 and under 1 MiB runs `merge3_str`,
writing + `git add`ing the file **only when `merge3` returns `Ok`**
(zero residual token conflicts). add/add, delete/modify, binary, and
oversized files are left exactly as git produced them. This is wired
into `GitService` merge/rebase/cherry-pick/revert.

## Tier 2 — AST structural merge (core BUILT — tsk134+tsk135; not yet wired)

**Status:** the language-neutral core now ships as production code in
`crates/oxplow-git/src/ast_merge.rs`, in two layers:

- **Parse → items (tsk134):** `parse_top_level_items(src, Language) ->
  Option<Vec<Item>>` + `language_for_path` + the per-language `MergeSpec`
  kind tables. tree-sitter and the 6 first-slice grammars (rust,
  typescript, tsx, javascript, python, go) are real `oxplow-git` deps
  now. Each `Item` carries `{ key, text, byte_span }`: key = import's
  normalized text or a named decl's `kind+name` (Go methods + receiver);
  the byte span includes any directly-attached leading doc-comment. Parse
  failure (error/missing node) → `None` (never operate on an untrusted
  tree). Const/var/Go-type decls fall back to text identity (safe: add
  still commutes, same-item edit just refuses).
- **3-way merge (tsk135):** `merge_top_level(base, ours, theirs, Language)
  -> AstMerge` where `AstMerge` is `Resolved(String)` | `Conflict(keys)`
  | `Bail(BailReason)`. Per-key classify lifts the spike's rule (take the
  changed side, take agreement, refuse on divergent same-key edit /
  delete-vs-edit / add-add-different). **Ordering** is conservative +
  deterministic: base order for surviving base items, each side's
  additions inserted after their nearest surviving base anchor (ours
  before theirs; pre-anchor additions go to a front bucket, ours first).
  Reconstruction reuses each item's verbatim span text (no reflow), joins
  with single newlines, and is gated by a **re-parse guard** —
  reconstructed source that doesn't re-parse → `Bail(ReparseFailed)`. A
  side that doesn't parse → `Bail(SideParseFailed)`; a duplicate identity
  key within one side → `Bail(DuplicateKeys)`.

**Still not wired into `auto_resolve_conflicts` — that's tsk136.** Note a
useful invariant the ordering relies on: because we preserve each side's
relative order and only interleave *independent* additions, a
reconstruction is essentially always valid when the inputs are — the
re-parse guard is a belt-and-suspenders backstop. The text below is the
original scoped design.

## Tier 2 — AST structural merge (SCOPED — tsk121)

Where Tier-1 still leaves a conflict that is actually *commutative at the
syntax level* — both sides added different `use`/`import`s, or
independently added different top-level declarations — a syntax-aware
tier can resolve it. Tier-1 misses these whenever the two additions are
textually adjacent (same insertion gap) or interleave, because at token
granularity that's an ambiguous-order add/add.

### Licensing constraint (hard)

Mergiraf is the SOTA AST merge (Rust + tree-sitter, GumTree/PCS, 30+
languages) but is **GPLv3**. Oxplow is **MIT** (`license = "MIT"`, root
`Cargo.toml:37`). **Linking mergiraf as a library would virally GPL the
whole codebase — DISALLOWED.** The only GPL-safe use is invoking a
*separately-installed* `mergiraf` binary as an external subprocess (mere
aggregation, like we already shell out to `git`), never bundled or
distributed with oxplow. So Tier-2 is **build-our-own**, with an
optional opt-in external-binary escape hatch documented but off by
default.

### We already have the parsers

`tree-sitter 0.26` + 9 grammars are workspace deps today (root
`Cargo.toml:128-137`), used by `oxplow-code-metrics`: **rust,
typescript, tsx, javascript, python, go, java, c, cpp, clojure** (the
epic note says "7" — the real count is 9 grammars / 10 `Language`
variants). `oxplow_code_metrics::spec::language_for_path` already maps
extensions → grammar, and each `LanguageSpec` carries a
`grammar: || tree_sitter_*::LANGUAGE.into()` thunk. Tier-2 reuses this;
**no new dependency is required to build it.**

### Spike (done — `crates/oxplow-git/tests/ast_merge_spike.rs`)

A dev-only integration test (no production code, tree-sitter as a
dev-dep) parses Rust top-level items via the existing grammar and runs a
per-item 3-way classify. It proves the risky core works and that the
safety model falls out for free. All 6 cases pass:

- top-level items parse via the existing `tree_sitter_rust` dep;
- both-add-different-`use` → structural union (order-insensitive);
- independently-added top-level `fn`s → union;
- one side edits a body while the other adds a fn → clean;
- **same fn edited differently by both → refused (semantic overlap)**;
- add/add of one name with different bodies → refused.

Identity heuristic in the spike: a `use` is keyed by its normalized
text; every other named item by `kind + name` (so a body edit reads as
"same item, changed", not delete+add). The merge is the same
take-the-changed-side / refuse-on-divergence classify as Tier-1, lifted
to item granularity.

### Recommended slice (smallest valuable)

**Top-level-item commutative merge, run as a Tier-1.5 pass after Tier-1,
inside `auto_resolve_conflicts`, per supported language.** For a still-
conflicted file whose extension maps to a grammar:

1. Parse base/ours/theirs. **If any of the three fails to parse, bail
   for that file** (leave git's markers) — never merge a tree we can't
   trust.
2. Reduce each to its ordered top-level items (`use`/`import`, `fn`,
   `struct`/`class`/`type`, `const`/`static`, top-level `impl`/`trait`).
3. Per-item 3-way classify (the spike's `merge_items`). Refuse the whole
   file if *any* item is a divergent same-key edit, an add/add with
   different text, or a delete-vs-edit — i.e. only resolve when every
   item is independently resolvable.
4. Reconstruct source. **Ordering policy is the real design risk** (the
   spike normalizes to base-order-then-new; shipping needs to preserve
   each side's relative order and interleave deterministically — a
   trimmed PCS, or "keep base order, append new-ours then new-theirs at
   their nearest surviving anchor"). Re-validate by re-parsing the
   output; if it doesn't parse, discard and leave git's markers.
5. Same write+`git add`-only-on-clean discipline as Tier-1, so it can
   only reduce the conflict count.

**Languages, in value order:** rust, typescript+tsx+javascript, python,
go — cover these first (they're the repo's own stack and the bulk of
user code). java/c/cpp/clojure fall out of the same machinery once the
per-language item-kind tables exist (model them like
`oxplow-code-metrics`'s `LanguageSpec` kind tables).

**Explicitly out of the first slice:** sub-item / intra-body merge
(that's what Tier-1 already does textually inside each unchanged-vs-
changed item), GumTree tree-edit-distance matching, rename detection,
and the external-`mergiraf` escape hatch.

### Build estimate

| Piece | Est. |
|---|---|
| ✅ `LanguageSpec`-style top-level-item kind tables (reuse metrics pattern) for rust/ts/tsx/js/py/go (tsk134) | done |
| ✅ Generic parse → ordered-items extractor + identity keys (tsk134) | done |
| ✅ `merge_items` 3-way classify + ordering/reconstruction + re-parse guard (tsk135) | done |
| Wire into `auto_resolve_conflicts` behind the per-language gate + report counts | ~0.5 day |
| Tests (per language: commutative-win, divergence-refusal, parse-fail-bail, ordering) | ~1.5 days |
| **Total first slice (6 languages)** | **~1 week** |
| java/c/cpp/clojure kind tables + tests (follow-up) | ~1–2 days |
| Optional opt-in external `mergiraf` subprocess (off by default, config-gated, never bundled) | ~1 day |

### Risks / open questions

- **Ordering determinism** is the one genuinely hard sub-problem; a
  wrong ordering policy produces valid-but-surprising diffs. Mitigation:
  always re-parse the reconstruction and bail on any change in semantics
  the classify didn't authorize; start with the most conservative
  ordering and only relax with tests.
- **Comments/doc-comments** attach as `extra` nodes; the spike skips
  them. Shipping must decide whether a moved decl carries its doc
  comment (it should) — handle by treating leading attached comments as
  part of the item's text span.
- **Whitespace/formatting** between items: reconstruction must not
  reflow the file. Keep each surviving item's original byte span; only
  the inter-item joins are synthesized.

## Cross-references

- Integration point + conflict-state plumbing: `.context/git-integration.md`.
- Tier-1 source + tests: `crates/oxplow-git/src/smart_merge.rs`.
- Tier-2 core (parse→items + kind tables + 3-way merge/reconstruct/guard): `crates/oxplow-git/src/ast_merge.rs`.
- Spike (per-item 3-way classify, not yet productionized): `crates/oxplow-git/tests/ast_merge_spike.rs`.
- Parser/grammar reuse: `crates/oxplow-code-metrics/src/spec.rs`.
