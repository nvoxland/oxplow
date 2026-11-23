# Markdown (one component, the viewer and the editor)

What this doc covers: `Markdown` (`apps/desktop/src/components/Markdown/`),
the one component every surface draws or edits markdown with, its two
modes and their one feature set, who owns saving, how wikilinks and
mermaid survive the editor's round-trip, and where it is used.

## One component, two modes

`<Markdown value={text} />` is the viewer (`MarkdownView.tsx`,
react-markdown + remark-gfm). Given `onChange` or `onCommit` it is the
editor (`RichTextField.tsx`, Tiptap). **Both modes have the one feature
set**, and a surface never builds its own:

- **Links** (`links.ts`): a wikilink or ref link (a knowledge page, a file at
  a revision, a directory, a commit, a record of any shared model's store — a
  task, a knowledge page —, an extension's resource) opens through the page's navigation (`openRef`);
  Cmd/Ctrl-click or middle-click opens it in a new tab; right-click
  offers Open, Open in new tab, Open In ▸ (the file manager, the default
  app, a terminal, the browser: `openIn.ts`, [usability.md](./usability.md)
  "Open In ▸ on every path and URL") and a Copy of what it names (path, SHA,
  id, link). A web link opens in the app's web page when the allowlist
  takes it (`classifyExternalUrl`), else in the browser. An in-page
  anchor jumps. A broken `[[…]]` is inert and marked. A bare ref link
  (a record of a store, an extension's ref, written without a
  `|label`) shows its title (`useRefLinkTitle`, one rule for both modes;
  in the editor the stored text stays the slug, below).
- **`[[` completes** (the editor): typing `[[` lists what it could link
  to under the caret, filtered as the person types — the site search's
  hits (knowledge pages, tasks, commits, extensions' resources; not file
  content matches) and, on a page of a stream, its files by name, but
  not a store row's own file (`isStoreFile`: `<folder>/<id><suffix>` of a
  store's `files:` in `v_model_store`, as a page's markdown file: the row
  is what to link) (`refOptions.ts`; the owner's search is `useRefSearch` in
  `Markdown.tsx`). ↑/↓ move, Enter or Tab picks, Escape closes the list
  and keeps the text (the owner's Escape waits while it shows). The pick
  goes in as its link, saved as the wikilink a person would type
  (`refLinkOf`: `[[slug]]`, `[[path:12]]`, `[[tsk42]]`; a ref with no bare
  form as `[title](uri)`), and shows its title.
- **A format toolbar on a selection** (the editor, `SelectionToolbar.tsx`):
  Bold, Italic, Code, Link, Heading / Subheading (not in an `inlineOnly`
  field). It floats at the selection's end after a mouse selection (on the
  mouseup) or a keyboard one; it hides when the selection collapses or the
  focus leaves for anything but the toolbar. Buttons act on the press
  (a click would collapse the selection) and say whether they are on.
  Link opens a box: a web URL or an oxplow URI as typed, anything else
  read as a wikilink's target (`linkHref`: `local-snapshots`,
  `src/a.ts:12`, `tsk42`); Enter applies, an empty box removes the link,
  Escape leaves the text as it was.
- **Paste** (the editor, `refPaste.ts`): a pasted oxplow URI goes in as
  its link (`insertRefLinks`); a URI or web URL pasted over a selection
  links it; other text pastes as text. Nothing drops onto the field: no
  drag in the app carries a ref (`.context/usability.md` "Add to Context").
- **A Source view** (the editor, `Markdown.tsx`): the corner toggle
  (where the pencil was, revealed the same way) or Cmd/Ctrl+Shift+M shows
  the text as stored — `[[…]]` and all — in a plain monospace box, and
  back. The owner hears the same `onChange` / `onCommit` from either view
  (the box commits as the editor does: 300 ms after typing, on blur, on
  unmount) and the same keys reach it. The mode is the field's own, not saved. A one-line field
  (`inlineOnly`) has none; a field with `hidePencil` keeps the shortcut
  without the button.
- **Mermaid** fences draw as diagrams, always (pan, zoom, the
  lightbox; in the editor, the `FencedBlock` node view, click to edit
  its source).
- **`oxplow-page` fences** draw a live read-only page in either mode
  (`InlineView`, below). In the editor, **Edit YAML** opens the original
  fenced source; Escape or leaving the block returns to its live view.
- **Code blocks** carry a Copy (`CopyCodeButton`, top-right on hover),
  and are highlighted by their fence's language (```` ```sql ````):
  `codeHighlight.ts`'s one lowlight, through `rehypeHighlightCode` in the
  viewer and `CodeBlockLowlight` (decorations, the text unchanged) in the
  editor. No language, or one lowlight doesn't know: plain.
- **GFM tables**, images with the lightbox, `.oxplow-md` typography.
- **A file's relative links and images** (`base`, the URI of the file the
  markdown came from; the `markdown` viz's `markdown: { base: <column> }`,
  foundation's Markdown page passes `content.uri`): `parseMarkdownLink(href,
  base)` reads a relative href as a file in that file's folder
  (`relativeFilePath`: `./`, `../`, `/` from the root), on its filesystem
  and revision — `docs/guide.md` is a file, not an oxwiki slug, which it
  stays with no base, as oxwiki's own pages mean it. A relative image is
  `FileImage`: `readBytesAt` on the viz's stream (`useVizStream`), drawn
  from a blob URL given back when it goes; a missing one keeps its alt.
  `followLink` and `linkMenu` take the same `base`. A link to a file of
  the base's kind (the same extension) opens with the page the base's
  view names (`#oxplow.renderer=`), so `DEV.md` from a README read as a
  document opens as a document; `MarkdownViz` puts that key on the base
  (`baseAsRead`) only when the document is the tab's own file and the
  page drawing it isn't the default (the tab's address has no key then),
  so a README shown inside a commit page carries nothing of that page.
- **Headings** are `h2`/`h3` under `.oxplow-md`: a page's Contents rail
  lists them (either mode).
- **Add to Context** on a selection: the editor's right-click menu has
  Add Selection to Context, the text from the URI the owner gives
  (`contextUri`; a page's `markdown` viz gives the page's, from
  `PageViewContext`). The viewer's text is page text: the page's own
  right-click menu over a selection (`useSelectionContextMenu`, mounted by
  `PageHost`) offers Add Selection to Context. See `.context/agent-model.md` "The
  context basket".

The editor reads and writes markdown **as stored**: the component runs
`preprocessWikilinks` on the way in and `postprocessWikilinks` on the
way out, so no owner sees the editor's spelling; an owner's echo of the
text it was handed maps back to the editor's own text, so typing is
never reset.

## Who saves: two owners

- **`onCommit`**: the text 300 ms after typing stops, on blur and on
  unmount (an edit is never lost to a closed tab). For an owner that
  saves as the person types: a task's description, the React
  knowledge page. A value from outside is skipped while the editor has focus
  (a late echo mustn't clobber typing).
- **`onChange`**: every edit. For an owner that keeps a draft and saves
  on a button: a form's markdown field (`SchemaForm`, a schema property
  with `contentMediaType: text/markdown`), a page's markdown editor. A value from outside is applied even while focused: the
  owner echoes exactly what it heard, so anything else is a reset it
  means (Discard, Escape).
- **A click in the text keeps its caret.** Clicking the field around the
  text focuses the end; a click in the text doesn't (Tiptap's focus lands
  a frame later and moved the caret under keys already typed).
- **Keys**: Cmd/Ctrl+Enter (`onSubmitShortcut`), Cmd/Ctrl+S
  (`onSaveShortcut`) and Escape (`onEscape`, when none of the editor's
  popovers is open) reach the owner with the text as it is now. Enter
  adds a line.

## One line

`<Markdown value={text} inline />` draws one line of markdown in a `span`
that flows in its text: only `INLINE_ELEMENTS` (`inlineMarkdown.ts`:
emphasis, strikethrough, code, links, a line break) survive, every block
unwrapping into the line (react-markdown's `allowedElements` +
`unwrapDisallowed`), with the same link behavior and titles as the
viewer. A page column's `format: markdown` draws its cell this way (a
description's `first_sentence(body)`, which keeps its inline markdown and
cuts outside code spans and links). A component's frame draws the same
line through the kit (`renderMarkdown`: extensions-components.md
"Markdown in a frame"), from the same parse and link rules.

**Where the rules live.** `markdownLinks.ts` is a link's meaning with no
React — wikilinks in and out (`preprocessWikilinks`,
`postprocessWikilinks`, `refLinkOf`), `parseMarkdownLink`, `linkRef`,
the title rule (`refLinkTitle`, which `useRefLinkTitle` runs over
`useRefTitle`'s lookup) and the URLs a link may hold (`urlTransform`).
Nothing in that path names a model, a store or a resource (`oxplow/task`,
oxtask, oxwiki): see "Records and titles".
`MarkdownView.tsx` draws with them; `markdownTree.ts` builds a frame's
tree with them.

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
drawn live in the knowledge page body), foundation's `markdown` page (a `.md` file
read as a document, View as → Markdown), the knowledge page body (the knowledge page's `markdown` viz), a task's body (the task page's `markdown` viz), a form's markdown field
(`SchemaForm`: stream and thread prompts). A
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
  always-on knowledge page editor flattened any `| … |` table into run-on text on
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
- **`SelectionToolbar.tsx`** — the editor's format toolbar and the link
  box's reading (`linkHref`). Page text outside the editor has the page's
  selected-text right-click menu (`components/Context/SelectionContextMenu.tsx`).
- **`refPaste.ts`** — what a paste is (pure); `RichTextField`'s
  `handlePaste` acts on it.
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
  the default check, never `.trim`med. **Never as Link's `protocols`**: Tiptap
  registers those with linkify (for autolinking, off here), and linkify
  throws on a scheme it can't tokenize — `task`'s underscore broke
  every rich-text field (`InternalLink.test.ts`). `openOnClick:
  false` — click handling is owned by the
  React layer: the `RichTextField` wrapper's `onClick` /
  `onAuxClick` intercepts clicks on `<a>` descendants, parses the
  href via `parseMarkdownLink` and names its ref via `linkRef`
  (both from `MarkdownView` — one resolution for every kind: a task, an
  extension's ref, a file), and routes through
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
register as the knowledge page's own nodes.

Rendering budgets: at most 64 KiB of UTF-8 source per block and four nested
inline blocks (page markdown may itself contain page fences). A limit is
shown inline before another run opens; result limits remain the daemon's.
The normal reading line appears only after 150 ms. These budgets do not
change the markdown saved on disk.

## Wikilink round-trip

Knowledge pages use `[[ ]]` syntax extensively (`[[path/to/file]]`,
`[[dir:src/components|the folder]]`, `[[git:<sha>]]`, `[[tsk<id>]]`, etc.).
`MarkdownView`'s `preprocessWikilinks` converts these to standard
markdown links (`[label](oxplow:oxplow/file?path=…)`) for read-rendering; the
`postprocessWikilinks` helper is the inverse — it collapses standard
markdown links to oxplow URIs back into `[[ ]]` form on save.

**Record wikilinks** — see "Records and titles" below: a bare id, `[[store:id]]`
or a whole URI of any shared model's store links to that record, and a link
to it **resolves to its title** at render time (a bare `[[tsk42]]` shows the
title; `[[tsk42|label]]` keeps the label). A record no row holds renders
broken. Click routes through the page-nav chokepoint like every ref.

The `Markdown` component applies `preprocessWikilinks` to the value it
is given and `postprocessWikilinks` to the text it hands back. A record's
`[[id]]` goes in as its URI (`oxplow:oxplow/knowledge?id=…&store=oxwiki`), as
every other ref does, so the editor keeps it as a link and it comes back
as `[[id]]` (`[[<store>:<id>]]` when no bare id names it) (a bare slug as the href was once kept as a relative link
and saved as `[slug](slug)`). The on-disk shape is preserved across edits,
including the bare vs. labeled forms (`[[path]]` vs.
`[[path|label]]`). Plain http/https links and image links are left
alone — collapsing those into `[[ ]]` form would be lossy.

## Records and titles

A task, a knowledge page and a row of a model an extension adds are all
**records of a store of a shared model** (`.context/models.md`): the desktop
treats them one way, with no model, store or resource named in the link and
title code.

- **`recordStores.ts`** holds the stores, read from `v_model_store`
  (`recordStoresLoader.ts`, started at boot, re-read when it changes): per
  store its model, its **resource** and the params that name the store
  (`store_param`) and the record (`key`), its `id_pattern` and its `loose`
  flag — the declarations core's one record recognizer reads
  (`ResourceRegistry::with_record_ids`). From them: `recordOfStoreId`
  (`[[store:id]]`, any store), `recordOfBareId(id, loose)` (the first store,
  in the order read, whose pattern matches the id whole; **loose** stores'
  ids are tried before a path or a slug, the others after, as core does),
  `recordOf(uri)` (is this URI a store's record), `recordLinkText` (written
  as its id when a bare id names it, else `store:id`). Tests and the
  harness set them with `setRecordStores`.
- **A link to a record is a `ref` link** (`ParsedLink` has no `task` or
  `knowledge` kind): the one kind for any resource's URI. Its icon is its
  resource's (`parsedLinkIconKind`: the resource name, the `PageKindIcon`
  table's key). Its right-click menu adds Copy id for a record.
- **`refTitleCache.ts` is the one title cache**: `useRefTitle(ref)` /
  `loadRefTitle(ref)` read the title from the ref's resource's `resolve`
  dataset and title column (`v_resource`, `extensionTitleQuery`), lazily per
  ref, re-read (coalesced) when a dataset the read touched changes.
  `status` is `loading`, `found`, `missing` (the dataset has no such row) or
  `untitled` (the resource names no dataset). `refLinkTitle` turns it into
  the link's title and, for a **record** whose row is missing, the broken
  reason; a missing row of any other ref is never flagged.
- A model an extension adds links and titles with no desktop change: the
  tests prove it with a made-up `acme/ticket` store.
- What stays per store is the task board's: stages, buckets and fields
  (`useTaskTrackerProfiles`), none of it in the link path.

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

## The right-click menu

The wrapper's `onContextMenu` **always** fires and `preventDefault`s, so
the native webview menu never appears in the editor. It builds a shared
`ContextMenu`: on a link, the viewer's link menu first; then Cut / Copy /
Paste (via `navigator.clipboard` + ProseMirror commands, using positions
captured at menu-open so they survive the click moving focus) and, on a
selection, Add Selection to Context (the text, from `contextUri`).

## Related

- [theming.md](./theming.md) — semantic CSS variable tokens.
- [usability.md](./usability.md) — Enter/Escape contracts inline-edit
  fields must follow. RichTextField uses Cmd/Ctrl+Enter for explicit
  commit (Enter inserts a paragraph) + blur for implicit commit.
