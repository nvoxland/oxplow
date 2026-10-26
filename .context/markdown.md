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
- **`[[` completes** (the editor): typing `[[` lists what it could link
  to under the caret, filtered as the person types — the site search's
  hits (wiki pages, work items, commits, extensions' resources; not file
  content matches) and, on a page of a stream, its files by name
  (`refOptions.ts`; the owner's search is `useRefSearch` in
  `Markdown.tsx`). ↑/↓ move, Enter or Tab picks, Escape closes the list
  and keeps the text (the owner's Escape waits while it shows). The pick
  goes in as its link, saved as the wikilink a person would type
  (`refLinkOf`: `[[slug]]`, `[[path:12]]`, `[[tsk42]]`; a ref with no bare
  form as `[title](uri)`), and shows its title.
- **A format toolbar on a selection** (the editor, `SelectionToolbar.tsx`):
  Bold, Italic, Code, Link, Heading / Subheading (not in an `inlineOnly`
  field), and Add comment where the text takes comments. It floats at the
  selection's end after a mouse selection (on the mouseup) or a keyboard
  one; it hides when the selection collapses, a comment popover opens, or
  the focus leaves for anything but the toolbar. Buttons act on the press
  (a click would collapse the selection) and say whether they are on.
  Link opens a box: a web URL or an oxplow URI as typed, anything else
  read as a wikilink's target (`linkHref`: `local-snapshots`,
  `src/a.ts:12`, `tsk42`); Enter applies, an empty box removes the link,
  Escape leaves the text as it was.
- **Drop and paste** (the editor, `refDrops.ts`): an "Add to agent
  context" drag (`CONTEXT_REF_MIME`: a file, a wiki page, a page row's
  ref, a work item) or a work-item drag (`WORK_ITEM_DRAG_MIME`, several
  items) dropped on the field goes in where it lands as links
  (`insertRefLinks`; at the caret when the point names no place —
  `handleDOMEvents.drop`, ahead of ProseMirror's own drop, which gives up
  there and lets the browser paste the drag's text), and the field shows the dashed drop highlight while
  such a drag is over it. A pasted oxplow URI goes in as its link; a URI
  or web URL pasted over a selection links it; other text pastes as text.
- **A Source view** (the editor, `Markdown.tsx`): the corner toggle
  (where the pencil was, revealed the same way) or Cmd/Ctrl+Shift+M shows
  the text as stored — `[[…]]` and all — in a plain monospace box, and
  back. The owner hears the same `onChange` / `onCommit` from either view
  (the box commits as the editor does: 300 ms after typing, on blur, on
  unmount) and the same keys reach it; comments show only in the rich
  view. The mode is the field's own, not saved. A one-line field
  (`inlineOnly`) has none; a field with `hidePencil` keeps the shortcut
  without the button.
- **Mermaid** fences draw as diagrams, always (pan, zoom, the
  lightbox; in the editor, the `FencedBlock` node view, click to edit
  its source).
- **`oxplow-page` fences** draw a live read-only page in either mode
  (`InlineView`, below). In the editor, **Edit YAML** opens the original
  fenced source; Escape or leaving the block returns to its live view.
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
  with `contentMediaType: text/markdown`), the comment composer, a page's
  markdown editor. A value from outside is applied even while focused: the
  owner echoes exactly what it heard, so anything else is a reset it
  means (Discard, Escape).
- **A click in the text keeps its caret.** Clicking the field around the
  text focuses the end; a click in the text doesn't (Tiptap's focus lands
  a frame later and moved the caret under keys already typed).
- **Keys**: Cmd/Ctrl+Enter (`onSubmitShortcut`), Cmd/Ctrl+S
  (`onSaveShortcut`) and Escape (`onEscape`, when none of the editor's
  popovers is open) reach the owner with the text as it is now. Enter
  adds a line.

## Where it is used

The page `markdown` viz node is the owner a declared
page uses: with an `edit: true` action it keeps a draft in the tab's
history state (`markdown:<page id>`, through `onChange`), enables Save
only when the draft differs from the saved text, runs the action with the
text laid over its edited field, and shows the text it saved until the
re-read brings it; Discard clears the draft. The page's rail lists the
headings.

Every markdown surface: the `markdown` viz and markdown nodes of a
page's view, and `oxplow-page` fenced blocks (an inline page, read-only,
drawn live in the wiki body), effort summaries
(`TaskDetail`, `DiffViewPage`), the wiki body (the wiki page's `markdown` viz), a work
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
  `FencedBlock` + `InternalLink` + **GFM tables**
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
  `FencedBlock` React NodeView flushes via `flushSync`, which trips
  React's "flushSync was called from inside a lifecycle method"
  warning. Deferring the first render to a post-commit effect silences
  it; harmless here since there's no SSR. The same rule covers **every
  later content swap**: once mounted, Tiptap 3 creates each new React
  NodeView under `flushSync`, so the upstream-`value` sync effect runs
  `setContent` in a `queueMicrotask`, re-checking focus/destroyed at
  apply time (`RichTextField.render.test.tsx`). Don't call `setContent`
  synchronously from a React effect.
- **`RefCompletion.tsx`** — `[[` completion: a `@tiptap/suggestion`
  plugin (debounced, a stale search aborted, the list mounted and placed
  by the plugin) and its list. Mounted only when the field is given a
  `refSearch`; `editor.storage.refCompletion.open` says the list shows, so
  `RichTextField`'s own keys stand aside. `refOptions.ts` is what it lists
  (pure), `refLinkOf` (`MarkdownView.tsx`) what a pick puts in, tested
  against `postprocessWikilinks` for each kind.
- **`SelectionToolbar.tsx`** — the selection toolbar, its gate
  (`selectionToolbarVisible`) and the link box's reading (`linkHref`).
  The plain-DOM comment layer keeps its own one-button
  `SelectionCommentToolbar` (`components/Comments/`).
- **`refDrops.ts`** — what a drop names and what a paste is (pure);
  `RichTextField`'s drop handler and `handlePaste` act on them. A real drop
  needs layout (`posAtCoords`), so it is covered in the browser suite
  (`knowledge/wiki.spec.ts`), the rest in unit tests.
- **`RefTitles.tsx`** — the title widget a bare ref link gets in the
  editor ("A ref link's title in the editor").
- **`FencedBlock.tsx`** — extends Tiptap's `CodeBlock` (same node
  name, `codeBlock`) with a React NodeView that paints rendered SVG
  via `renderMermaidInto` when the caret is outside, and a raw
  editable `<pre><code>` when the caret enters. Round-trips as a
  ` ```mermaid …``` ` fenced code block, so storage is unchanged.
  Its `oxplow-page` branch draws `InlineView` in a non-editable island,
  with a pencil to edit the YAML. Both languages remain ordinary fenced
  code in storage, with the one `codeBlock` node registered.
- **`InlineView.tsx`** — draws an inline spec through the shared page
  run store and `PageTree`, without page chrome or a rail. A rejected
  spec shows its source and the daemon's validation error under it.
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

Shared mermaid rendering pipeline used by `FencedBlock` (editor
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
blockquotes, GFM tables — all standard. The custom `FencedBlock`
hijacks rendering of `language: "mermaid"` and `"oxplow-page"` fences without changing
the serialized form.

## Live page blocks

A fence whose language is `oxplow-page` holds the same page YAML the
daemon accepts for an inline spec. `MarkdownView` routes the fence to
`InlineView`; `FencedBlock` does the same in the editor. The daemon parses
and checks the spec, including read-only restrictions (no command loaders,
actions, forms, components, editors, terminals or slot mounts), before it
opens a run. The client does not parse YAML or substitute a separate query
runner. Loader invalidations use the shared page subscription store, and
unmounting a block releases its run.

The block inherits its containing page's `PageViewContext` scope, including
when that tab is hidden. A caller may supply explicit scope; outside a page
body it falls back to the host stream and selected thread. Links use the
containing page's navigation. A picked row
updates the inline run and the host's view params; inline node IDs never
register as the wiki page's own nodes.

Rendering budgets: at most 64 KiB of UTF-8 source per block and four nested
inline blocks (page markdown may itself contain page fences). A limit is
shown inline before another run opens; result limits remain the daemon's.
The normal reading line appears only after 150 ms. These budgets do not
change the markdown saved on disk.

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
- **Authoring: selection toolbar + right-click.** A selection floats the
  editor's toolbar (above), whose Add comment opens the composer —
  mirroring the plain-DOM surfaces' button, which this contenteditable
  region is deliberately carved out of (`useDomAnnotations` skips
  contenteditable). *Opening* an
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
