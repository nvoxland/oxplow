# Editor and Monaco patterns


What this doc covers: how `EditorPane` hosts Monaco, the conventions for
context menus / decorations / overlays, and the LSP bridge. For the
broader "Monaco at the core, custom shell around it" rationale, see
`architecture.md`.

## Single editor instance, models per file

`apps/desktop/src/components/EditorPane.tsx` mounts **one** Monaco editor and
swaps `editor.model` whenever `filePath` changes. Models are keyed by
URI via `streamFileUri(stream, path)` (see `apps/desktop/src/lsp.ts`), so opening
the same file across tabs hits the same model and edit history.

This avoids the cost (and visual flicker) of rebuilding the editor on
every tab switch — the parent `CenterTabs` keeps `EditorPane` mounted
in the same slot and only changes the `filePath` prop.

The Monaco host `<div>` carries `data-testid="monaco-host"` and
`data-file-path=<currentFilePath>` so test harnesses can assert which
file the editor is showing without relying on tab text. Keep the
attributes in sync if the mount structure changes.

## Save shortcut (Cmd/Ctrl+S) is double-bound by design

Save is registered TWICE on purpose:

1. The native Tauri menu (built by the renderer's menu service and
   pushed via `desktopBridge().setNativeMenu` →
   `commands::menu::set_native_menu`) binds the Cmd/Ctrl+S
   accelerator. This is what real users hit in day-to-day use — the
   OS menu catches the key before the webview sees it.
2. Inside `EditorPane`, right after the editor is created:
   `editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, onSave)`.
   Monaco owns its own keybinding service, so this makes the shortcut
   work when (a) the editor has focus, and (b) under synthetic
   keystrokes (Playwright, automation) that never reach the native
   menu.

These don't double-fire in normal use because the OS menu consumes the
keydown before it propagates to the webview. If you ever see save
firing twice, something changed in that dispatch order — investigate
before deleting either binding.

## Closing a dirty tab prompts before discarding

`handleCloseOpenFile` in `apps/desktop/src/App.tsx` checks `draftContent !==
savedContent` before calling `closeOpenFile`, and pops a
`window.confirm` when the tab is dirty. Cancelling the confirm leaves
the tab and its draft intact. The other call to `closeOpenFile` in
App.tsx is an error-path cleanup (open-file IPC failed) — that one
intentionally skips the prompt because there's nothing worth saving
yet. Auto-close via `enforceOpenFileLimit` already refuses to discard
dirty tabs, so all three close paths are now consistent.

## Custom context menu

Monaco's native menu is disabled (`contextmenu: false`). Right-click is
caught via `editor.onContextMenu`, which:

1. Computes the click's text position.
2. **Preserves any existing selection** if the click falls inside it
   (so actions like "Compare with Clipboard" still see the selected
   text). Only collapses the selection when the click lands outside.
3. Opens the shared `ContextMenu` component
   (`apps/desktop/src/components/ContextMenu.tsx`) at the cursor.

Menu items live in a per-render `MenuItem[]` array — `Cut`, `Copy`,
`Paste` (via `navigator.clipboard` + `editor.executeEdits`), `Save`,
`Find`, `Go to Definition`, `Format Document`, `Copy Path`,
`Annotate with Git Blame`, `Compare with Clipboard`, `Add Comment`,
`Open Comment`. Items are gated on `enabled` based on file path, language
LSP support, selection presence, and (for `Open Comment`) whether the
right-click landed on a commented range. The right-click handler
`preventDefault`s the native event (and the editor container has a
belt-and-suspenders `onContextMenu={e => e.preventDefault()}`) so the
webview's native menu never shows.

The `ContextMenu` component handles its own viewport-clamping and
submenu flip-up logic; never re-implement that per-call site.

## Blame overlay

The blame *state machine* — fetch, gutter scroll/line-height sync,
clear-on-file-change, and refresh-on-save — lives in
`apps/desktop/src/components/useBlame.ts` (a hook EditorPane mounts,
returning `{ blame, blameScrollTop, blameLineHeight, toggleBlame }`).
EditorPane renders the `BlameOverlay` from that state and wires the
"Annotate with Blame" menu item to `toggleBlame`. (`useMonacoEditor` was
intentionally NOT extracted: the editor-setup effect is the component's
core and is tightly coupled to its menu / comment-layer / focus-push /
LSP-provider wiring — a hook would need ~8 injected callbacks and read
*less* cohesively than leaving it inline.)

When the user toggles `Annotate with Blame`, the hook fetches a merged
per-line attribution via `localBlame(stream.id, filePath)` and EditorPane
renders an absolutely-positioned DOM overlay on the left gutter (the
`BlameOverlay` sub-component). The merge is computed server-side in
`crates/oxplow-git/src/blame.rs` (`computeLocalBlame`) — it walks closed
task efforts newest-first (`TaskEffortStore.listEffortsForPath`),
diffs each effort's start/end snapshot content to figure out which lines
the effort introduced, and falls back to `gitBlame` for any line the
local walk can't attribute. Snapshots pruned by the 7-day retention
window degrade gracefully — the effort is skipped and git blame picks
up the line. The IPC is a single round-trip (`oxplow:localBlame`) so the
UI never has to reconcile two streams.

Layout details:

- Reserves ~150px of left space by setting `lineNumbers: "off"` and
  `lineDecorationsWidth: 150` on the editor while blame is on.
- Syncs to scroll via `editor.onDidScrollChange` updating a
  `blameScrollTop` state.
- Reads `monaco.editor.EditorOption.lineHeight` so each row aligns with
  the corresponding text line.
- Two hue tracks share one overlay: **local** (task) lines use
  `--blame-local-*` (warm amber age ramp) with a 2px
  `--blame-local-border` left stripe; **git** lines use `--blame-git-*`
  (cool blue age ramp) with a 2px `--blame-git-border` left stripe.
  Uncommitted lines render with `--blame-uncommitted` and a transparent
  border. All variables live in `public/index.html` — see
  `.context/theming.md`.
- Labels: local rows show the truncated task title; git rows show
  `yyyy-mm-dd  author`; uncommitted rows are blank.
- On click, local rows call `onRevealTask(itemId)` (wired to
  `handleRequestEditTask` in `App.tsx` — pops the Plan pane and
  opens the task edit modal). Git rows call
  `onRevealCommit(sha)` (same path as before: bumps `historyReveal`
  and `bottomActivate` tokens).
- Right-click on a git row still opens the three-item menu (Copy SHA,
  Reveal commit, Copy author email). Local and uncommitted rows skip
  the menu — there is no commit to act on.

Refresh rule: the overlay re-fetches when the file is saved
(`isDirty` transitions true → false). It does **not** refresh on every
edit because attribution is relative to the last closed effort / HEAD,
not the buffer.

## Uncommitted-change gutter markers

`EditorPane` fetches the file's HEAD content via `readFileAtRef(stream,
"HEAD", path)` on file open, caches it per-path, and diffs the buffer
against it on every content change. The line-level LCS diff runs in
`diffLineKinds` (capped at 5000 lines per side — larger files skip
diffing). Gutter bars render via Monaco `linesDecorationsClassName`:

- `oxplow-gutter-added` — green 3px inset bar (new line, no nearby delete).
- `oxplow-gutter-modified` — blue 3px inset bar (added line next to a delete).
- `oxplow-gutter-deleted` — red bottom bar on the surviving line next to a
  pure deletion.

Classes are defined in `public/index.html`. HEAD is re-fetched when
`filePath` changes; a subsequent commit won't invalidate the cache until
the file is reopened. Decoration ids live in `diffDecoIdsRef` and are
updated via `editor.deltaDecorations`.

## Diff editor

`apps/desktop/src/components/Diff/DiffPane.tsx` uses Monaco's `createDiffEditor`.
The `DiffSpec` type (`apps/desktop/src/components/Diff/diff-request.ts`) supports
two render modes:

- **Git-ref backed.** `leftRef` plus `rightKind: "working" | { ref }`.
  Each side fetched via `readFileAtRef` / `readWorkspaceFile`.
- **Inline content.** Optional `leftContent`/`rightContent` strings that
  bypass git/workspace reads. Used by the editor's "Compare with
  Clipboard" action — left = selection, right = clipboard text.

Tab labels honor an optional `labelOverride` so inline diffs can show
"selection vs clipboard" instead of generic "(diff)".

The pane's top toolbar carries three buttons: Prev / Next (which call
`editor.goToDiff("previous" | "next")` — Monaco's built-in diff
navigation on the diff editor instance) and Open file (fires the
`onJumpToSource` prop). `App.tsx` wires `onJumpToSource` to
`handleOpenFile(spec.path)` followed by `closeDiffTab(diff.id)`, so
jumping to source opens the working copy of the right-side path in the
regular editor pane and closes the diff tab. For literal-content diffs
(e.g. Compare with Clipboard) this still opens the workspace file,
which is the useful action.

## Comment overlay

`MonacoCommentLayer` (`apps/desktop/src/components/Comments/MonacoCommentLayer.tsx`)
is mounted inside `EditorPane`'s relative container once `monacoReady`
and `filePath` are set (target `file:<repo-relative path>`, stream-scoped
— files aren't thread-bound, so `threadId` is null). It mirrors the
rich-text integration:

- **Highlights** are inline `deltaDecorations`
  (`inlineClassName: "oxplow-comment-highlight"`, its own id ref —
  separate from `diffDecoIdsRef`), recomputed when the thread list
  changes. A parallel `decoId → commentId` map drives hit-testing.
- **Re-anchoring** tries the stored `{ startLine, startColumn, endLine,
  endColumn }` first (quote still matches → reuse), else runs the shared
  `resolveAnchor` (`components/Comments/anchor.ts`) over
  `model.getValue()` — exact (disambiguated by stored `prefix`/`suffix`
  context + `textOffset` proximity) then bounded fuzzy — and maps the
  offset back via `model.getPositionAt`. `buildAnchorJson` re-persists the
  enriched anchor (line/col + textOffset + prefix/suffix + `approx`) from
  the resolved location so hints/context self-heal and old comments
  upgrade in place; fuzzy matches paint the dashed
  `.oxplow-comment-highlight--approx` variant. Corrected/orphaned anchors
  persist through `setCommentAnchor` (no event → no loop).
- **Relinking.** The `MonacoCommentHandle` exposes `relinkTargets()` +
  `relinkToSelection(id)`; EditorPane's context menu lists "Relink
  orphaned: …" entries that re-attach an orphaned comment to the current
  selection via `relink_comment`.
- **Right-click-driven**, not click/selection (those fight cursor
  placement). `MonacoCommentLayer` exposes a `MonacoCommentHandle`
  (`forwardRef` + `useImperativeHandle`): `commentIdAt(position)`,
  `addCommentForSelection()`, `openComment(id)`. `EditorPane`'s existing
  `onContextMenu` captures the comment-at-position into a ref and adds
  "Add Comment" (enabled when there's a selection) + "Open Comment"
  (enabled when over a commented range) to its `contextMenuItems`,
  delegating to the handle. The layer renders `NewCommentPopover` /
  `CommentPopover` from its own state.
- **Popover/composer coords:** Monaco renders its own selection (not the
  native DOM one), so the rect comes from
  `editor.getScrolledVisiblePosition` + the editor DOM node's bounding
  rect — NOT `window.getSelection()`.
- **Cross-page reveal:** the layer subscribes to `comment-reveal-bus.ts`.
  When the Comments Dashboard's "Go to location" button fires
  `requestCommentReveal(id)` and navigation lands on this file, the layer
  (once its decoration is painted) calls `editor.revealRangeInCenter` on
  the matching decoration range, opens the popover, and clears the bus.
  The request survives the async mount + threads fetch because it stays
  pending until consumed.

## LSP bridge

The backend (`LspSessionManager`, see `.context/lsp.md` for the full
subsystem) owns every language-server process — one shared session per
`(stream, language)`, used by both the editor and the MCP tools.
`apps/desktop/src/lsp.ts` defines `LspClient`, a thin facade per
`(stream, language)`: `request`/`notify` ride the `lsp_request` /
`lsp_notify` RPCs (payloads as JSON strings — specta can't type
`serde_json::Value`), and one module-level `lsp:event` subscription
demuxes `LspSessionEvent` payloads (publishDiagnostics, session
status) to the live clients.

Monaco provider registration lives in
`apps/desktop/src/components/editor-lsp-providers.ts`
(`registerLspProviders(monaco, deps)`): definition, hover, references,
**completion**, **rename**, **code actions**, and **document symbols**
(outline). Providers register ONCE for every language id in
`editor-language.ts`'s extension map (`allKnownLanguageIds()`) and
no-op per call via `hasLspServer` — re-registering on server-list
changes would race Monaco's open widgets. Every provider calls
`flushDoc(model)` before its request (didChange debounce race). LSP ↔
Monaco shape conversion is pure and unit-tested in
`apps/desktop/src/lsp-monaco-mapping.ts` (completion kinds map by enum
*name*, symbol kinds by `-1` offset; `normalizeWorkspaceEdit` handles
`changes` + `documentChanges` and counts skipped file create/rename/
delete ops). Workspace edits apply across open AND non-open files via
`apps/desktop/src/lsp-workspace-edit.ts`
(`applyNormalizedWorkspaceEdit`): open Monaco models through
`pushEditOperations` (draft + undo intact), non-open files through the
workspace file IPC read-modify-write; file create/rename/delete ops
are still skipped + surfaced on the status banner. Rename rides
Monaco's own rename UI (`editor.action.rename`, F2 + context-menu
item): the open-model subset returns to Monaco's bulk-edit service,
the non-open subset (`partitionByOpenModel`) is written at accept
time. Code actions whose edits stay within open models keep Monaco's
native application; anything touching a non-open file (and any LSP
`command`) routes through the registered Monaco command
`oxplow.lsp.applyCodeAction`, which applies the edit then forwards
the command to `workspace/executeCommand`. Server-initiated
`workspace/applyEdit` arrives as an `applyEditRequest` event and is
applied by the stream's mounted editor (see `.context/lsp.md`).

The **client lifecycle** itself lives in
`apps/desktop/src/components/useLspClients.ts` (a hook EditorPane mounts):
the per-language `LspClient` cache + `ensureLspClient`, diagnostics →
Monaco markers (`markerOwnerRef`), the document sync (`didOpen` /
`didChange` / `didClose` / `didSave`), the install-suggestion state +
`installSuggested` flow, and disposal on both stream-switch and unmount.
`didChange` rides `DocumentSyncTracker`
(`apps/desktop/src/lsp-document-sync.ts`): per-path **monotonic** LSP
versions (Monaco's `getVersionId` resets on model swaps — don't use it)
with a ~200ms debounce; the hook's `flushPendingChanges(path)` must be
called before `didSave` and before any positional request so the server
never answers against stale text. EditorPane keeps the Monaco editor +
provider registration and calls the hook's `ensureLspClient`; the shared
status banner (`lspStatus`) stays in EditorPane because blame +
go-to-definition also write to it, so the hook takes `setLspStatus` as
a param.

Language eligibility is **data-driven**: `hasLspServer(languageId)`
(`apps/desktop/src/lsp-servers-store.ts`, backed by `list_lsp_servers`,
refreshed on `lspServersChanged` / `configChanged`) replaces the old
hardcoded `isLspCandidateLanguage`. Any language with a yaml-configured
or Mason-installed server lights up.

When a request fails with "no language server configured", the status
banner surfaces the self-describing error and `EditorPane` checks
`apps/desktop/src/lspSuggestions.ts` for a Mason package mapping. If a
suggestion exists, the banner shows an "Install <package>" button that
calls `installLspPackage` (an IPC into `oxplow-app::lsp_installer`).
The install runs against the Mason registry
(`mason-org/mason-registry`) and lands the binary in
`.oxplow/lsp/<name>/`; on success the server list refreshes and the
cached `LspClient` for that language is dropped so the next request
retries with the new binary.

LSP is also exposed to **agents** via the `lsp_*` MCP tools
(`crates/oxplow-mcp/src/lib.rs`) so they can run definition/reference
queries without shelling out — same backend sessions as the editor.

## Monaco workers

`apps/desktop/src/main.tsx` installs `self.MonacoEnvironment.getWorker`
that returns Vite-bundled `?worker` modules for the editor core plus
the JSON / CSS / HTML / TypeScript language services. Without this,
Monaco logs "You must define a function MonacoEnvironment.getWorkerUrl
or MonacoEnvironment.getWorker" and runs language services on the main
thread, blocking input during heavy parsing. New Monaco language
contributions need a matching `case` here.

## Editor focus tracking

`EditorPane` pushes the user's current file/selection/caret to the
runtime via `window.oxplowApi.updateEditorFocus`, debounced ~150ms. The
runtime relays it through `EditorFocusStore` and uses
`formatEditorFocusForAgent` to inject it as `additionalContext` on the
agent's `UserPromptSubmit` hook — so the agent automatically knows what
the user has open and selected when they start a turn.

## Related

- [agent-model.md](./agent-model.md) — how editor focus reaches the
  agent, and how MCP LSP tools work.
- [git-integration.md](./git-integration.md) — `gitBlame` + history
  panel that the blame overlay reveals into.
