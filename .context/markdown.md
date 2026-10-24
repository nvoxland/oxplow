# Markdown (one component, the viewer and the editor)

What this doc covers: `Markdown` (`apps/desktop/src/components/Markdown/`),
the one component every surface draws or edits markdown with, its two
modes and their one feature set, who owns saving, how wikilinks, mermaid
and comments survive the editor's round-trip, and where it is used.

## One component, two modes

`<Markdown value={text} />` is the viewer (`MarkdownView.tsx`,
react-markdown + remark-gfm). Given `onChange` or `onCommit` it is the
editor (`RichTextField.tsx`, Tiptap). **Both modes have the one feature
set**, and a surface never builds its own:

- **Links** (`links.ts`): a wikilink or ref link (a wiki page, a file at
  a revision, a directory, a commit, a work item, an extension's
  resource) opens through the page's navigation (`openRef`);
  Cmd/Ctrl-click or middle-click opens it in a new tab; right-click
  offers Open, Open in new tab and a Copy of what it names (path, SHA,
  id, link). A web link opens in the app's web page when the allowlist
  takes it (`classifyExternalUrl`), else in the browser. An in-page
  anchor jumps. A broken `[[…]]` is inert and marked. A bare ref link
  (a wiki page, a work item, an extension's ref, written without a
  `|label`) shows its title (`useRefLinkTitle`, one rule for both modes;
  in the editor the stored text stays the slug, below).
- **Mermaid** fences draw as diagrams, always (pan, zoom, the
  lightbox; in the editor, the `MermaidBlock` node view, click to edit
  its source).
- **Code blocks** carry a Copy (`CopyCodeButton`, top-right on hover).
- **GFM tables**, images with the lightbox, `.oxplow-md` typography.
- **Headings** are `h2`/`h3` under `.oxplow-md`: a page's Contents rail
  lists them (either mode).
- **Comments**: the editor's own layer (`comments` prop, below); the
  viewer's text through its page's DOM layer (`data-ref`).

The editor reads and writes markdown **as stored**: the component runs
`preprocessWikilinks` on the way in and `postprocessWikilinks` on the
way out, so no owner sees the editor's spelling; an owner's echo of the
text it was handed maps back to the editor's own text, so typing is
never reset.

## Who saves: two owners

- **`onCommit`**: the text 300 ms after typing stops, on blur and on
  unmount (an edit is never lost to a closed tab). For an owner that
  saves as the person types: the task body (`TaskDetail`), the React
  wiki page. A value from outside is skipped while the editor has focus
  (a late echo mustn't clobber typing).
- **`onChange`**: every edit. For an owner that keeps a draft and saves
  on a button: a form's markdown field (`SchemaForm`, a schema property
  with `contentMediaType: text/markdown`), the comment composer, the
  lens editor. A value from outside is applied even while focused: the
  owner echoes exactly what it heard, so anything else is a reset it
  means (Discard, Escape).
- **Keys**: Cmd/Ctrl+Enter (`onSubmitShortcut`), Cmd/Ctrl+S
  (`onSaveShortcut`) and Escape (`onEscape`, when none of the editor's
  popovers is open) reach the owner with the text as it is now. Enter
  adds a line.

## Where it is used

The lens `markdown` viz (`lens/MarkdownViz.tsx`) is the owner a declared
page uses: with an `edit: true` action it keeps a draft in the tab's
history state (`markdown:<lens id>`, through `onChange`), enables Save
only when the draft differs from the saved text, runs the action with the
text laid over its edited field, and shows the text it saved until the
re-read brings it; Discard clears the draft. The page's rail lists the
headings (`lens/Contents.tsx`).

Every markdown surface: the lens `markdown` viz and document/dashboard
markdown parts (`LensResultView`, `PartsViz`), effort summaries
(`TaskDetail`, `DiffViewPage`), the wiki body (the `wiki-page-body` lens's `markdown` viz), a work
item's body (`TaskDetail`), comment messages (`CommentPopover`) and the
comment composer (`CommentComposer`), a form's markdown field
(`SchemaForm`: stream and thread prompts, a handoff's body). A
dashboard's `TextTile` is plain text by its own decision (its file says
why).

## What's in `apps/desktop/src/components/Markdown/`

- **`richTextExtensions.ts`** — the one extension list every field
  mounts (plus `getMarkdown(editor)` and the `Storage` type augmentation
  `tiptap-markdown` doesn't ship). Kept out of the component so tests
  (`richTextExtensions.test.ts`, `tableRoundTrip.test.ts`) run the real
  config rather than a copy. **Tiptap 3's `StarterKit` bundles `Link`
  and `Underline`**: `link: false` because `InternalLink` replaces it,
  `underline: false` because markdown has no underline syntax (with
  `html: false` it would vanish on save). Re-check what `StarterKit`
  bundles on every Tiptap major.
- **`RichTextField.tsx`** — the surface component. Configures Tiptap
  with `StarterKit` + `Markdown` (round-trip) + `Placeholder` +
  `MermaidBlock` + `InternalLink` + **GFM tables**
  (`Table`/`TableRow`/`TableHeader`/`TableCell`, all from
  `@tiptap/extension-table` since Tiptap 3, `resizable`,
  block-only so they're off when `inlineOnly`). `tiptap-markdown` ships
  the GFM table serializer and markdown-it parses tables, so adding the
  standard table nodes is all the round-trip needs — without them the
  always-on wiki editor flattened any `| … |` table into run-on text on
  autosave. Editor-only table CSS (cell-selection overlay, column-resize
  handle, `table-layout: fixed`) lives in `index.html` next to the
  `.oxplow-md table` display styles (which the editor inherits via
  `.oxplow-md`). Round-trip covered by `Markdown/tableRoundTrip.test.ts`. Debounced 300ms save while typing,
  immediate commit on blur. The editor's outer `<div>` wears
  `.oxplow-rt-field` (hover tint + pencil reveal) and the inner
  ProseMirror element wears `.oxplow-md .oxplow-rt-editor` so prose
  inherits the same typography as `MarkdownView`. Passes
  `immediatelyRender: false` to `useEditor` — the default (`true`)
  renders the initial document during React's render phase, and the
  `MermaidBlock` React NodeView flushes via `flushSync`, which trips
  React's "flushSync was called from inside a lifecycle method"
  warning. Deferring the first render to a post-commit effect silences
  it; harmless here since there's no SSR. The same rule covers **every
  later content swap**: once mounted, Tiptap 3 creates each new React
  NodeView under `flushSync`, so the upstream-`value` sync effect runs
  `setContent` in a `queueMicrotask`, re-checking focus/destroyed at
  apply time (`RichTextField.render.test.tsx`). Don't call `setContent`
  synchronously from a React effect.
- **`RefTitles.tsx`** — the title widget a bare ref link gets in the
  editor ("A ref link's title in the editor").
- **`MermaidBlock.tsx`** — extends Tiptap's `CodeBlock` (same node
  name, `codeBlock`) with a React NodeView that paints rendered SVG
  via `renderMermaidInto` when the caret is outside, and a raw
  editable `<pre><code>` when the caret enters. Round-trips as a
  ` ```mermaid …``` ` fenced code block, so storage is unchanged.
- **`InternalLink.ts`** — extends Tiptap's standard `Link` mark to
  allow oxplow URIs (the `oxplow:` scheme) through the
  URL sanitizer — Link's `isAllowedUri`, which every parse, render and
  set-link check goes through — and `oxplow-invalid:`, the link an
  unresolvable wikilink is kept as, so it saves back as `[[…]]` instead
  of plain text; a link Tiptap hands over with a null href is passed to
  the default check, never `.trim()`med (tsk1007). **Never as Link's `protocols`**: Tiptap
  registers those with linkify (for autolinking, off here), and linkify
  throws on a scheme it can't tokenize — `work_item`'s underscore broke
  every rich-text field (tsk974; `InternalLink.test.ts`). `openOnClick:
  false` — click handling is owned by the
  React layer: the `RichTextField` wrapper's `onClick` /
  `onAuxClick` intercepts clicks on `<a>` descendants, parses the
  href via `parseMarkdownLink` and names its ref via `linkRef`
  (both from `MarkdownView` — one resolution for every kind: a task, an
  extension's ref, a file; tsk976), and routes through
  `useOptionalPageNavigation`. Plain click → in-tab
  navigate; Cmd/Ctrl/middle/right click → new tab. Cursor placement
  inside link text is sacrificed — arrow in from adjacent text.

## `mermaidRender.ts`

Shared mermaid rendering pipeline used by `MermaidBlock` (editor
NodeView), `MarkdownView` (read-only bodies — it keeps its own
parse/render/stray-sweep loop but loads mermaid and pan-zoom from
here) and `MediaLightbox`. The one `mermaid.initialize` lives in
`loadMermaid`; it pins `layout: "dagre"` because Mermaid 12 switched
the default to ELK, which would re-lay-out every existing diagram.
Exports
`loadMermaid`, `loadSvgPanZoom`, `attachPanZoom`,
`renderMermaidInto`. Lazy-loads mermaid + svg-pan-zoom on first use;
waits for the host element to have a non-zero layout box before
initializing pan-zoom (otherwise `getCTM().inverse()` throws on
zero-size SVGs — the `waitForVisible` guard, originally from
`MarkdownView`).

## Storage model

Markdown stays the on-disk format. `tiptap-markdown` parses on mount
and serializes on save via `editor.storage.markdown.getMarkdown()`.
Fenced code blocks round-trip; lists, headings, bold, italic, code,
blockquotes, GFM tables — all standard. The custom `MermaidBlock`
hijacks rendering of `language: "mermaid"` fences without changing
the serialized form.

## Wikilink round-trip

Wiki pages use `[[ ]]` syntax extensively (`[[path/to/file]]`,
`[[dir:src/components|the folder]]`, `[[git:<sha>]]`, `[[tsk<id>]]`, etc.).
`MarkdownView`'s `preprocessWikilinks` converts these to standard
markdown links (`[label](oxplow:oxplow/file?path=…)`) for read-rendering; the
`postprocessWikilinks` helper is the inverse — it collapses standard
markdown links to oxplow URIs back into `[[ ]]` form on save.

**Work item wikilinks** — a full URI (`[[oxplow:oxplow/work_item?id=ENG-1&provider=issues]]`) or a
bare id of the active list's (`[[tsk42]]` with oxplow's tasks), matched
by the id pattern the list declares (`v_capability_provider.id_pattern`,
`workItemRefOfMention`), as the backend ref extractor does — route to the
item's `oxplow/work_item` URI link and **resolve to its title** at
render time via `useWorkItemTitle` (`apps/desktop/src/workItemTitleCache.ts`,
a lazy per-ref sibling of `wikiTitleCache`) — same display-swap rule wiki
slugs use (bare `[[tsk42]]` shows the title; `[[tsk42|label]]` keeps the
label). An item that isn't on the active list renders broken. Click
routes through `workItemTabRef` via the page-nav chokepoint.

The `Markdown` component applies `preprocessWikilinks` to the value it
is given and `postprocessWikilinks` to the text it hands back. A wiki
page's `[[slug]]` goes in as its URI (`oxplow:oxplow/wiki?slug=…`), as
every other ref does, so the editor keeps it as a link and it comes back
as `[[slug]]` (a bare slug as the href was once kept as a relative link
and saved as `[slug](slug)`). The on-disk shape is preserved across edits,
including the bare vs. labeled forms (`[[path]]` vs.
`[[path|label]]`). Plain http/https links and image links are left
alone — collapsing those into `[[ ]]` form would be lossy.

## A ref link's title in the editor

The editor can't change a link's text: the text is what it saves. So
`RefTitles.tsx` draws the title beside it: a React widget decoration
(`addDecorations` + `ReactWidgetRenderer`) before each link that names a
titled ref. Once the title resolves for a bare link, the widget shows the
kind's icon and the title (`[data-ref-title]`, an `<a>` with the same
href, so a click opens it as any link does), and CSS hides the link
beside it (`.oxplow-ref-title:has([data-ref-title]) + a` in
`index.html`). A broken ref shows marked in its place. With the caret
strictly inside the link there is no widget, and the slug shows to edit
(arrow in: a click opens the link). Decorations only, so nothing of it is
saved. `RichTextField.titles.test.tsx`.

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

## Comments

The editor becomes comment-enabled when given a `comments`
config (`{ streamId, threadId, target, author? }`, `target` a URI).
The `wiki-page` page entry says `comments: true`, so the page's URI is the comment target (a wiki page isn't thread-bound) and the page wraps its body in the URI as a context node
(`contextNodeProps`), so plain-DOM selections around the editor
resolve to the page too; WorkItemPage passes
`{ target: <the item's URI>, threadId:
item.threadId }`.

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
  `relocateComment` (`oxplow.knowledge.relocate_comment`; the stored anchor
  then equals the recomputed one, so the re-read stops); fuzzy matches get the dashed
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
  quote re-attaches its highlight promptly instead of waiting for the
  blur/commit that updates `value`.
- **Anchors are stored when the document settles** (tsk902): every
  re-anchor queues the moved anchors (`pendingAnchorsRef`), and
  `flushAnchors` stores them — at once when the editor isn't focused, on
  blur, and on unmount — never per typing pause, since each
  `oxplow.knowledge.relocate_comment` is a command run (an audit row and events)
  on the person's behalf. A store that fails (the comment deleted
  meanwhile) is caught; the bus audited it.
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
  (`Markdown/selectionToolbar.ts`); the toolbar hides when the
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
  When the comment navigator (`CommentNavigator`) fires
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
