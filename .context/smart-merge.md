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

## Tier 2 — AST structural merge (WIRED — tsk134+tsk135+tsk136+tsk137)

**Status:** the language-neutral core ships as production code in
`crates/oxplow-git/src/ast_merge.rs` and is wired into the auto-resolve
path as a Tier-1.5 fallback (tsk136 — see "Wiring" below). Two layers:

- **Parse → items (tsk134, languages extended in tsk137):**
  `parse_top_level_items(src, Language) -> Option<Vec<Item>>` +
  `language_for_path` + the per-language `MergeSpec` kind tables.
  tree-sitter and **ten grammars** are real `oxplow-git` deps now: the 6
  first-slice (rust, typescript, tsx, javascript, python, go) plus the 4
  second-slice (java, c, cpp, clojure — tsk137).

  > **One `Language` enum (tsk321).** `ast_merge` no longer declares its own
  > `Language` — it `pub use`s the canonical `oxplow_code_metrics::Language`
  > (the single source of truth) and keys its `MergeSpec` table off it via
  > the free fn `merge_spec(lang) -> Option<&MergeSpec>`. Canonical variants
  > the merge tier doesn't support (today only `CSharp`, which *is* analysed
  > for metrics) return `None` there — identity stays unified, support is a
  > per-spec property, not a separate enum. `ast_merge::language_for_path`
  > resolves through `oxplow_code_metrics::language_for_path` then drops
  > languages with no `MergeSpec` (so `.cs` ⇒ `None`, unchanged behavior).
  > The LSP↔analysis namespace bridge is `language_from_lsp_id` in the same
  > crate (maps `typescriptreact`→`Tsx`, etc.).

  Each `Item` carries `{
  key, text, byte_span }`. Identity key depends on the language's
  `KeyStrategy`:
  - **Generic** (rust/ts/tsx/js/python/go/java): import normalized text,
    else a named decl's `kind+name` (Go methods + receiver; Java
    class/interface/enum/record by name), else `kind + normalized-text`.
  - **CFamily** (c/cpp): `#include` by text; functions by their
    declarator (name + signature, so overloads stay distinct);
    structs/unions/enums/classes/namespaces by name; typedefs + globals by
    their declarator's leaf identifier; `template_declaration` seen
    through to the wrapped decl.
  - **Clojure**: top-level `(def…/defn…/ns …)` forms by `head-symbol +
    defined-name`; any other form by `kind + normalized-text`. (Two
    `defmethod`s on one multifn collapse to one key → `DuplicateKeys`
    bail — safe, not wrong.)

  The byte span includes any directly-attached leading doc-comment. Parse
  failure (error/missing node) → `None` (never operate on an untrusted
  tree). Items on the **Generic text fallback** (Rust/TS `const`/`let`/
  `var`, Go grouped `type`/`var`/`const`, unnamed items) commute on *add*
  but have a known soundness gap on *divergent same-item edit* — see
  "Text-identity divergent-edit gap" below (tracked as **tsk140**).
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

Note a useful invariant the ordering relies on: because we preserve each
side's relative order and only interleave *independent* additions, a
reconstruction is essentially always valid when the inputs are — the
re-parse guard is a belt-and-suspenders backstop. The text below is the
original scoped design.

### Wiring (tsk136)

`auto_resolve_conflicts` (`smart_merge.rs`) now runs the AST pass as a
**Tier-1.5 fallback, per file**: when Tier-1's `merge3_str` returns
`Err(Conflicted)`, it calls `ast_merge_resolve(path, base, ours, theirs)`,
which gates on `ast_merge::language_for_path` (the six first-slice
grammars) and accepts the result **only** when `merge_top_level` returns
`AstMerge::Resolved` — i.e. the re-parse guard has already passed. Any
`Conflict`, `Bail` (parse-fail / duplicate-keys / reparse-fail), or
unsupported extension → `None`, leaving git's markers untouched. Same
write + `git add`-only-on-clean discipline as Tier-1, so the pass can
still only *reduce* the conflict count.

`AutoResolveReport` gained an `ast_resolved: u32` counter: AST-resolved
paths go into the same `resolved` Vec (so `GitOpResult.auto_resolved =
resolved.len()` already surfaces them in the toast/HUD — no parallel
path), and `ast_resolved` records how many of those came from the AST
tier specifically. The flagship case the wiring unlocks: both sides add a
*different* import at the *same gap* (adjacent lines), which Tier-1 sees
as an ambiguous add/add but the AST tier unions. End-to-end test:
`auto_resolve_clears_ast_only_import_conflict` in `smart_merge.rs`; one
e2e auto-resolve test per second-slice language
(`auto_resolve_clears_{java,c,cpp,clojure}_ast_only_conflict`).

### Second-slice languages (tsk137)

java, c, cpp, clojure are now supported (grammars were already workspace
deps, promoted into `oxplow-git`). Each got top-level-item kind tables +
identity keys (see `KeyStrategy` above), per-language parse/key/merge unit
tests, and an end-to-end auto-resolve test. **Java caveat:** methods and
fields are *nested in a class body*, not top-level, so the AST tier
matches at type-declaration granularity — intra-class method/field merges
are Tier-1's (textual) job, consistent with the rest of the design.

### Mergiraf escape hatch — intentionally dropped

The original tsk137 also proposed an *optional, opt-in, off-by-default*
external `mergiraf` subprocess as an escape hatch. **This half is
intentionally not built.** Mergiraf is GPLv3 and oxplow is MIT; even the
"mere aggregation" subprocess route adds a runtime dependency on a
copyleft tool we'd have to document, gate, and support, for marginal value
over our own AST tier. The project decision (this doc's "Licensing
constraint (hard)" section) is to **build our own and stop there** — no
mergiraf integration, library *or* subprocess. If a future need arises,
re-open it as a fresh decision rather than reviving the dropped task half.

### Text-identity divergent-edit gap (tsk140)

Items on the `Generic` **text-fallback** key (`kind + normalized-text`)
encode their body into the key. When *both* sides edit the same such item
differently, `classify` reads it as base-deleted-by-both + two independent
adds and keeps **both** divergent versions rather than refusing — the
re-parse guard misses it (duplicate decls parse fine). This violates the
reduce-only "never silently resolve a true overlap" invariant for those
items only (named decls, C-family declarator keys, and Clojure head+name
keys are safe). Pre-existing since tsk135; tracked + scoped in **tsk140**.

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

## Candidate scope: REGULAR BLOBS ONLY

Both tiers only ever see conflicts whose three stages are all mode
`100644`/`100755` (`is_regular_blob` in `smart_merge.rs`). This is a
safety gate, not an optimization.

A symlink's blob **is its target path**, so it reads back as ordinary
small UTF-8 and merges like any other text file — and then
`std::fs::write` FOLLOWS the link and overwrites whatever it points at.
tsk158 reproduced this destroying an unrelated tracked file while
reporting `resolved: ["link"], remaining: 0`. Gitlinks (`160000`,
submodule commit OIDs) are excluded for the same reason: no sane
textual 3-way merge exists for them.

Note this also bounds the "can only reduce the conflict count" claim
above: it holds *within* the candidate set. Widening the set to
non-regular modes breaks it.

**Languages, in value order:** rust, typescript+tsx+javascript, python,
go — cover these first (they're the repo's own stack and the bulk of
user code). java/c/cpp/clojure fall out of the same machinery once the
per-language item-kind tables exist (model them like
`oxplow-code-metrics`'s `LanguageSpec` kind tables).

**Explicitly out of the first slice:** sub-item / intra-body merge
(that's what Tier-1 already does textually inside each unchanged-vs-
changed item), GumTree tree-edit-distance matching, and rename detection.
The external-`mergiraf` escape hatch is **dropped entirely** (not merely
deferred) — see "Mergiraf escape hatch — intentionally dropped" above.

### Build estimate

| Piece | Est. |
|---|---|
| ✅ `LanguageSpec`-style top-level-item kind tables (reuse metrics pattern) for rust/ts/tsx/js/py/go (tsk134) | done |
| ✅ Generic parse → ordered-items extractor + identity keys (tsk134) | done |
| ✅ `merge_items` 3-way classify + ordering/reconstruction + re-parse guard (tsk135) | done |
| ✅ Wire into `auto_resolve_conflicts` behind the per-language gate + report counts (tsk136) | done |
| ✅ Tests (per language: commutative-win, divergence-refusal, parse-fail-bail, ordering) | done |
| **Total first slice (6 languages)** | **shipped** |
| ✅ java/c/cpp/clojure kind tables + tests (tsk137) | done |
| ~~Optional opt-in external `mergiraf` subprocess~~ | **dropped** — GPL boundary, see "Mergiraf escape hatch — intentionally dropped" above |

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
