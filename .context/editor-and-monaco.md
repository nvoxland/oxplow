# Editor and Monaco patterns

What this doc covers: how `EditorPane` hosts Monaco, the conventions for
context menus / decorations / overlays, and the LSP bridge. For the
broader "Monaco at the core, custom shell around it" rationale, see
`architecture.md`.

## Single editor instance, models per file

`src/ui/components/EditorPane.tsx` mounts **one** Monaco editor and
swaps `editor.model` whenever `filePath` changes. Models are keyed by
URI via `streamFileUri(stream, path)` (see `src/ui/lsp.ts`), so opening
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

1. The native Electron menu (via `commands.ts` → `setNativeMenu`) binds
   the Cmd/Ctrl+S accelerator. This is what real users hit in day-to-day
   use — the OS menu catches the key before the webview sees it.
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

`handleCloseOpenFile` in `src/ui/App.tsx` checks `draftContent !==
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
   (`src/ui/components/ContextMenu.tsx`) at the cursor.

Menu items live in a per-render `MenuItem[]` array — `Save`, `Find`,
`Go to Definition`, `Format Document`, `Copy Path`,
`Annotate with Git Blame`, `Compare with Clipboard`. Items are gated on
`enabled` based on file path, language LSP support, and selection
presence.

The `ContextMenu` component handles its own viewport-clamping and
submenu flip-up logic; never re-implement that per-call site.

## Blame overlay

When the user toggles `Annotate with Git Blame`, `EditorPane` fetches
blame via `gitBlame(stream.id, filePath)` and renders an absolutely-
positioned DOM overlay on the left gutter (the `BlameOverlay`
sub-component). Layout details:

- Reserves ~150px of left space by setting `lineNumbers: "off"` and
  `lineDecorationsWidth: 150` on the editor while blame is on.
- Syncs to scroll via `editor.onDidScrollChange` updating a
  `blameScrollTop` state.
- Reads `monaco.editor.EditorOption.lineHeight` so each row aligns with
  the corresponding text line.
- Ages each line via `blameColor(ageDays)` — bright blue for fresh
  commits, fading through gray for older ones. Uncommitted lines (sha
  all zeros) render blank.
- On click, calls the `onRevealCommit(sha)` prop (wired from `App.tsx`)
  which bumps two tokens: `historyReveal` (passed to `HistoryPanel` to
  select the commit) and `bottomActivate` (passed to the bottom
  `DockShell` to open the History tool window). Uncommitted lines
  (all-zero sha) intentionally pass `onClick={undefined}` so clicking
  them does nothing — there is no commit to reveal.

Refresh rule: the overlay re-fetches when the file is saved
(`isDirty` transitions true → false). It does **not** refresh on every
edit because blame is against `HEAD`, not the buffer.

## Uncommitted-change gutter markers

`EditorPane` fetches the file's HEAD content via `readFileAtRef(stream,
"HEAD", path)` on file open, caches it per-path, and diffs the buffer
against it on every content change. The line-level LCS diff runs in
`diffLineKinds` (capped at 5000 lines per side — larger files skip
diffing). Gutter bars render via Monaco `linesDecorationsClassName`:

- `newde-gutter-added` — green 3px inset bar (new line, no nearby delete).
- `newde-gutter-modified` — blue 3px inset bar (added line next to a delete).
- `newde-gutter-deleted` — red bottom bar on the surviving line next to a
  pure deletion.

Classes are defined in `public/index.html`. HEAD is re-fetched when
`filePath` changes; a subsequent commit won't invalidate the cache until
the file is reopened. Decoration ids live in `diffDecoIdsRef` and are
updated via `editor.deltaDecorations`.

## Diff editor

`src/ui/components/Diff/DiffPane.tsx` uses Monaco's `createDiffEditor`.
The `DiffSpec` type (`src/ui/components/Diff/diff-request.ts`) supports
two render modes:

- **Git-ref backed.** `leftRef` plus `rightKind: "working" | { ref }`.
  Each side fetched via `readFileAtRef` / `readWorkspaceFile`.
- **Inline content.** Optional `leftContent`/`rightContent` strings that
  bypass git/workspace reads. Used by the editor's "Compare with
  Clipboard" action — left = selection, right = clipboard text.

Tab labels honor an optional `labelOverride` so inline diffs can show
"selection vs clipboard" instead of generic "(diff)".

## LSP bridge

`src/ui/lsp.ts` defines `LspClient`, which talks to a per-language LSP
server through a runtime-managed socket (the runtime spawns the server
process via `LspSessionManager` and bridges its stdio to a WebSocket).
`EditorPane` registers Monaco providers (definition, hover, references)
that proxy to the client; the work of mapping LSP positions ↔ Monaco
positions and locations ↔ Monaco editor ranges happens in the editor
component.

The set of languages eligible for LSP is determined by
`isLspCandidateLanguage` (`src/ui/editor-language.ts`). The runtime
loads extra LSP servers from `newde.yaml` on startup
(`config.lspServers` → `registerLanguageServer` per server).

LSP is also exposed to **agents** via `buildLspMcpTools`
(`src/mcp/lsp-mcp-tools.ts`) so they can run definition/reference queries
without shelling out.

## Editor focus tracking

`EditorPane` pushes the user's current file/selection/caret to the
runtime via `window.newdeApi.updateEditorFocus`, debounced ~150ms. The
runtime relays it through `EditorFocusStore` and uses
`formatEditorFocusForAgent` to inject it as `additionalContext` on the
agent's `UserPromptSubmit` hook — so the agent automatically knows what
the user has open and selected when they start a turn.

## Related

- [agent-model.md](./agent-model.md) — how editor focus reaches the
  agent, and how MCP LSP tools work.
- [git-integration.md](./git-integration.md) — `gitBlame` + history
  panel that the blame overlay reveals into.
