# Rich-text editor (shared WYSIWYG surface)

What this doc covers: the Tiptap-based `RichTextField` editor used for
inline-editable prose on task and (future) wiki pages, how mermaid +
internal URL schemes survive round-trip, and the composition model
("one page, many editors, mixed editability") it was built for.

## Composition model

Pages that read as a single document are composed at the React level
from multiple independent blocks:

- **Editable blocks** — one `RichTextField` per field. On the task
  page, title → `task.title` (via the simpler `TitleField` textarea)
  and description → `task.description`. Each block runs its own
  `onCommit` and saves to its own destination.
- **Read-only blocks** — rendered with plain React + `MarkdownView`,
  no editor instance. Effort cards in `ActivityTimeline` are the
  current example; future "referenced files" footers on wiki pages
  follow the same pattern.

The visual continuity ("it feels like one document") comes from
shared prose typography (`.oxplow-md` styles applied to both the
editor surface and `MarkdownView`), not from a single underlying
editor.

To telegraph editability, every editable block carries a faint
`Pencil` icon (lucide-react, ~12px) in its top-right, opacity ~0.35
by default and ~0.85 on hover/focus. Read-only blocks must **not**
show the pencil — that's the consistent signal "this is for reading."

## What's in `apps/desktop/src/components/RichText/`

- **`RichTextField.tsx`** — the surface component. Configures Tiptap
  with `StarterKit` + `Markdown` (round-trip) + `Placeholder` +
  `MermaidBlock` + `InternalLink`. Debounced 300ms save while typing,
  immediate commit on blur. The editor's outer `<div>` wears
  `.oxplow-rt-field` (hover tint + pencil reveal) and the inner
  ProseMirror element wears `.oxplow-md .oxplow-rt-editor` so prose
  inherits the same typography as `MarkdownView`. Passes
  `immediatelyRender: false` to `useEditor` — the default (`true`)
  renders the initial document during React's render phase, and the
  `MermaidBlock` React NodeView flushes via `flushSync`, which trips
  React's "flushSync was called from inside a lifecycle method"
  warning. Deferring the first render to a post-commit effect silences
  it; harmless here since there's no SSR.
- **`MermaidBlock.tsx`** — extends Tiptap's `CodeBlock` (same node
  name, `codeBlock`) with a React NodeView that paints rendered SVG
  via `renderMermaidInto` when the caret is outside, and a raw
  editable `<pre><code>` when the caret enters. Round-trips as a
  ` ```mermaid …``` ` fenced code block, so storage is unchanged.
- **`InternalLink.ts`** — extends Tiptap's standard `Link` mark to
  allow `file:`, `dir:`, `gitcommit:` URL schemes through the URL
  sanitizer. `openOnClick: false` — click handling is owned by the
  React layer: the `RichTextField` wrapper's `onClick` /
  `onAuxClick` intercepts clicks on `<a>` descendants, parses the
  href via `parseMarkdownLink` (reused from `MarkdownView`), and
  routes through `useOptionalPageNavigation`. Plain click → in-tab
  navigate; Cmd/Ctrl/middle/right click → new tab. Cursor placement
  inside link text is sacrificed — arrow in from adjacent text.

## What's in `apps/desktop/src/components/Wiki/mermaidRender.ts`

Shared mermaid rendering pipeline used by `MermaidBlock` (editor
NodeView). `MarkdownView` still has its own inline copy of the same
logic; consolidating them is a small follow-up. Exports
`loadMermaid`, `loadSvgPanZoom`, `attachPanZoom`,
`renderMermaidInto`. Lazy-loads mermaid + svg-pan-zoom on first use;
waits for the host element to have a non-zero layout box before
initializing pan-zoom (otherwise `getCTM().inverse()` throws on
zero-size SVGs — see `MarkdownView.tsx` for the original guard).

## Storage model

Markdown stays the on-disk format. `tiptap-markdown` parses on mount
and serializes on save via `editor.storage.markdown.getMarkdown()`.
Fenced code blocks round-trip; lists, headings, bold, italic, code,
blockquotes, GFM tables — all standard. The custom `MermaidBlock`
hijacks rendering of `language: "mermaid"` fences without changing
the serialized form.

## Wikilink round-trip

Wiki pages use `[[ ]]` syntax extensively (`[[path/to/file]]`,
`[[dir:src/components|the folder]]`, `[[git:<sha>]]`, etc.).
`MarkdownView`'s `preprocessWikilinks` converts these to standard
markdown links (`[label](file:path)`) for read-rendering; the new
`postprocessWikilinks` helper is the inverse — it collapses standard
markdown links carrying our internal schemes (`file:`, `dir:`,
`gitcommit:`) back into `[[ ]]` form on save.

`WikiPageTab` applies `preprocessWikilinks` to the body before
handing it to `RichTextField`, and applies `postprocessWikilinks` to
the markdown it gets back on `onCommit` before calling
`writeWikiPageBody`. The on-disk shape is preserved across edits,
including the bare vs. labeled forms (`[[path]]` vs.
`[[path|label]]`). Plain http/https links and image links are left
alone — collapsing those into `[[ ]]` form would be lossy.

## What does NOT round-trip yet

- **Per-link right-click menus / wiki title resolution.** `MarkdownView`
  attaches a right-click menu to every link (copy URL, open in
  new tab, etc.) and swaps bare `[[slug]]` text for the resolved
  wiki page title via `useWikiTitle`. Neither is implemented in the
  editor surface yet.

## CSS surfaces

In `apps/desktop/index.html`:

- `.oxplow-md` — shared prose typography. Applied to both
  `MarkdownView` wrappers and the editor's inner ProseMirror element.
- `.oxplow-rt-field` — outer wrapper. Hover/focus add a
  `--surface-card` background + `--border-strong` outline; this is
  where the pencil-affordance reveal triggers from.
- `.oxplow-rt-editor.ProseMirror` — kills the default focus outline,
  sets `min-height: 1.4em`, and implements the placeholder
  pseudo-element (Tiptap's `Placeholder` extension drives the
  `data-placeholder` attribute).
- `.task-section-heading`, `.task-title-field` — task page specifics.

## Where the editor is used today

- `apps/desktop/src/components/Plan/TaskDetail.tsx` — description +
  acceptance fields on the task details page. Title uses the simpler
  `TitleField` (auto-sizing textarea, no rich formatting).
- `apps/desktop/src/components/Wiki/WikiPageTab.tsx` — wiki page body.
  Always-on editing (no view/edit toggle); the Edit / Save / Revert /
  Done-editing buttons were removed from `WikiPageRail`. Save fires
  via the RichTextField debounce + blur, calling `writeWikiPageBody`
  directly (with `postprocessWikilinks` first). Errors surface via
  `recordOpError`. The `useWikiPageController` hook still backs
  page-summary loading, refresh-on-event, and notFound/loadError
  state — it just no longer owns the editor's draft.

## Comments

`RichTextField` becomes comment-enabled when given a `comments`
config (`{ streamId, threadId, targetKind, targetId, author? }`).
WikiPageTab passes `{ targetKind: "wiki", targetId: slug, threadId:
null }` (wiki pages aren't thread-bound) and additionally wraps the
body scroll host in a `wiki:<slug>` context node
(`contextNodeProps`), so plain-DOM selections around the editor
resolve to the page too; TaskPage passes
`{ targetKind: "task", targetId: String(item.id), threadId:
item.thread_id }`.

- **`CommentDecorations.ts`** is a ProseMirror **plugin extension**,
  NOT a stored mark. This is the load-bearing rule: a stored mark
  would serialize into the markdown that round-trips to disk and
  pollute the file. Highlights are inline **decorations** computed
  from the comment list and pushed in via a transaction meta
  (`commentDecorationsKey`); between pushes the set maps through doc
  edits so highlights track typing. (`InternalLink` is a stored mark
  — use it as a structural reference only, not the comment mechanism.)
- **Re-anchoring.** `findCommentRange(doc, quote, { hintFrom, hintTo,
  prefix, suffix })` re-resolves each comment's `quote` to a
  `{ from, to, approx }`: stored-hint fast path, then `flatten(doc)` +
  the shared `resolveAnchor` (`components/Comments/anchor.ts`) — exact
  (disambiguated by context + proximity) then bounded fuzzy — mapped back
  to doc positions. `flatten` now inserts a `BLOCK_SEP` between blocks
  (with a synthetic map entry so `map` stays 1:1 with text), so
  **cross-block selections match** (capture uses the same flatten, so
  stored quote/context agree). `buildAnchorJson` re-persists the enriched
  anchor (from/to + textOffset + prefix/suffix + `approx`) via
  `setCommentAnchor` (no event → no loop); fuzzy matches get the dashed
  `--approx` highlight.
- **Typed context on create.** When composing a comment,
  `startCommentForSelection` captures `referencedRefs` from the live DOM
  selection via `refsInRange` (`components/Comments/domAnchor.ts`) — any
  rendered `<a>` inside the quote (e.g. an `InternalLink` wikilink)
  becomes a canonical `(kind,id)` ref so the agent sees what the
  highlighted prose links to. The backend additionally unions refs it can
  parse out of the quote *text* at create time, so plain mentions are
  covered too. `context_chain` stays empty: a wiki/task editor is its own
  root, with no typed ancestors above it. The in-content `{from,to,…}`
  anchor stays the surface coordinate fast-path (not migrated to the W3C
  selector array — the resolver tolerates both, and the legacy shape is
  what the re-anchor fast path reads).
- **Live un-orphan.** The re-anchor effect also keys on a debounced
  `docVersion` bumped from `editor.on("update")`, so retyping a deleted
  quote re-attaches promptly instead of waiting for the blur/commit that
  updates `value`.
- **Relinking.** When a selection is active, the editor context menu
  lists "Relink orphaned: …" entries that re-attach an orphaned comment
  to the current selection via `relink_comment` (rewrites quote +
  anchor, clears orphaned).
- **Authoring: selection toolbar + right-click.** On mouseup with a
  non-collapsed selection, the field floats the shared
  `SelectionCommentToolbar` ("Add comment") at the selection end —
  mirroring the plain-DOM surfaces, which this contenteditable region
  is deliberately carved out of (`useDomAnnotations` skips
  contenteditable). Gating is the pure `selectionToolbarVisible`
  (`RichText/selectionToolbar.ts`); the toolbar hides when the
  selection collapses or a composer/popover is open. *Opening* an
  existing comment stays right-click-only (click-to-open would fight
  cursor placement). The wrapper's `onContextMenu` **always** fires and
  `preventDefault`s, so the native webview menu never appears in the
  editor. It builds a shared `ContextMenu` with Cut / Copy / Paste (via
  `navigator.clipboard` + ProseMirror commands, using positions
  captured at menu-open so they survive the click moving focus) plus,
  when comment-enabled, "Add Comment" (selection non-empty) and "Open
  Comment" (right-click target's `closest("[data-comment-id]")` hits a
  decoration). "Add Comment" opens `NewCommentPopover` (composer
  anchored to the selection-end caret via `coordsAtPos`); "Open
  Comment" opens `CommentPopover`. Both live in `components/Comments/`
  and `stopPropagation` their pointer events so the wrapper's
  editor-focus `onClick` doesn't steal focus. The highlight CSS class
  is `.oxplow-comment-highlight` (`--comment-highlight*` tokens).
- **Cross-page reveal.** The field subscribes to `comment-reveal-bus.ts`.
  When the Comments Dashboard's "Go to location" button fires
  `requestCommentReveal(id)` and navigation lands on this wiki page, the
  field finds the decoration's rendered `[data-comment-id]` node,
  `scrollIntoView`s it, opens its `CommentPopover`, and clears the bus.
  The request stays pending until the node exists, so the async mount +
  threads fetch after navigation still resolves.

## Related

- [theming.md](./theming.md) — semantic CSS variable tokens.
- [usability.md](./usability.md) — Enter/Escape contracts inline-edit
  fields must follow. RichTextField uses Cmd/Ctrl+Enter for explicit
  commit (Enter inserts a paragraph) + blur for implicit commit.
