# Terminal (xterm.js)

What this doc covers: the xterm.js Terminal that hosts the agent
session, plus the file-path link provider that turns `path:line` text
in terminal output into clickable links that open files in oxplow's
editor.

## Single component

`apps/desktop/src/components/TerminalPane.tsx` is the **only** xterm.js
consumer. It's mounted by two page renderers:

- `apps/desktop/src/pages/AgentPage.tsx` — the agent tab, `paneTarget`
  `"working"` / `"talking"`. The backend spawns the agent CLI.
- `apps/desktop/src/pages/TerminalPage.tsx` — the "Terminal" Page (rail
  entry + `indexRef("terminal")`), `paneTarget` `"shell"`. The backend
  (`commands/terminal.rs`) early-branches on `"shell"` to spawn the
  user's `$SHELL -l` (fallback `/bin/sh`) rooted at
  `stream.worktree_path` — no agent command, plugin, or system prompt.
  One persistent shell per stream (session key `<stream>|shell|<mode>`).
  No `onUserInterrupt`: Escape is an ordinary shell keystroke here.

Both go through the same component; only the `paneTarget` (and thus the
server-side spawn) differs.

The component owns:

- The xterm `Terminal` instance + `FitAddon`.
- A custom keydown handler (Cmd+V paste, Shift+Enter, PageUp/Down
  routing for tmux history mode, Escape interrupt detection).
- A custom wheel handler (mousewheel → tmux history scrolling when
  appropriate).
- The PTY session lifecycle via `desktopBridge().openTerminalSession`
  / `sendTerminalMessage` / `closeTerminalSession`.
- Drag-drop "Add to agent context" support (see
  `.context/usability.md` for the convention).
- The file-path link provider (see below).

## File-path link provider

`apps/desktop/src/terminal-link-provider.ts` exports two pieces:

- **`findFilePathMatches(line)`** — pure scanner. Given a line of
  text, returns ranges + optional `:line` / `:line:col` for every
  string that looks like a file reference. Unit-tested in
  `terminal-link-provider.test.ts`. Detection rules:
  - Tokens are runs of `[\w./@~+-]+` optionally followed by `:N` or
    `:N:M`.
  - Leading `'"`(\[<` and trailing `.,;!?)\]}>'"\`` punctuation are
    trimmed (so `(see foo.ts:42).` extracts `foo.ts:42`).
  - Tokens preceded by `scheme://` (or any scheme-followed-by-colon-
    and-slashes prefix) are rejected — URLs stay URLs.
  - Email-shaped tokens are rejected.
  - With no slash, the token must look like `name.ext` where the
    extension starts with a letter and stem contains a letter (rules
    out version strings like `1.5`).
- **`installFilePathLinkProvider(term, { onActivate })`** — registers
  an xterm `ILinkProvider` that scans each visual line, calls
  `findFilePathMatches`, and yields ILink ranges. Wrapped lines are
  coalesced (only the wrap-start row is scanned; continuation rows
  return `undefined`). On click, `onActivate(match)` fires with the
  raw path text plus optional line/column.

TerminalPane wires `onActivate` to its `onOpenFile` prop, resolving
relative paths against the prop `worktreePath` first (`/`-prefixed
paths pass through; `~/` paths are dropped — the frontend doesn't
know HOME).

## Path resolution and caveats

- Relative paths resolve against **`stream.worktree_path`**, not the
  pty's actual cwd. The frontend doesn't track the pty's current
  directory; if the user `cd`s into a subfolder and prints a path
  relative to that, the link will mis-resolve. Acceptable v1
  limitation.
- Absolute paths outside the worktree open as-is — the App.tsx
  callback strips the worktree prefix when present and otherwise
  forwards the absolute path; oxplow's `readWorkspaceFile` will
  surface a missing-file error if the path isn't readable.
- Wide CJK characters can misalign link ranges by a cell because the
  provider assumes one cell per char — clicks still work because the
  match `text` is passed to `onOpenFile` directly, not derived from
  cell coordinates.

## Commenting on terminal output

`TerminalPane` accepts an optional `comments` prop (`{ streamId,
threadId, targetKind, targetId }`); `AgentPage` passes
`{ targetKind: "agent", targetId: thread.id }` and `TerminalPage` passes
`{ targetKind: "terminal", targetId: stream.id, threadId: null }`. When
set, `components/Comments/TerminalCommentLayer.tsx` mounts against the
live `Terminal` (exposed via a `term` state set right after `term.open`).
The terminal is in the editor/terminal carve-out, so the app-level
`DomCommentLayer` ignores it — this layer is its dedicated comment
surface, the xterm analog of `MonacoCommentLayer`.

- **Anchoring is against the serialized buffer, not the DOM.** Only the
  visible viewport is in the DOM, so `components/Comments/terminalAnchor.ts`
  flattens every `buffer.active` line (`translateToString(true)`) into one
  string + per-line char offsets. A selection's quote is the *serialized
  slice* (from `getSelectionPosition()` buffer coords via `coordToOffset`),
  NOT `term.getSelection()` — the latter de-wraps, which wouldn't match the
  search text. `selectors_json` is the shared W3C array plus a
  `TerminalBufferSelector` (the surface coordinate refinement). Re-anchor
  runs the shared `resolveAnchor` over the serialized buffer, then maps the
  offset back to `(line, col)`.
- **Highlights are xterm decorations**, repainted on `onWriteParsed` /
  `onScroll`. For each comment, `registerMarker(absoluteLine - cursorAbs)`
  + `registerDecoration({ x, width })` overlays `.oxplow-terminal-comment`
  on the line; a click reopens the thread. Placement is wrapped in
  try/catch so a coordinate edge case degrades to "no highlight" rather
  than disturbing the terminal.
- **Orphaning is expected.** Scrollback wraps and evicts (5000-line cap),
  so terminal anchors orphan far more readily than editor anchors —
  acceptable; the comment still lists in the inbox and the agent still
  answers it. No `set_comment_anchor` self-heal loop here (unlike the
  editors): the buffer is too volatile to persist a re-resolved hint.

## Adding a new link kind

Compose a separate xterm link provider rather than expanding
`findFilePathMatches`. xterm allows multiple registered providers;
each scans the same line. Examples for the future:

- A WebLinks-style URL provider (currently no `WebLinksAddon`
  installed; URLs in terminal output are not clickable).
- Stack-trace formats from non-rust/non-js languages
  (`File "x.py", line 42`, `at com.foo.Bar(Bar.java:42)`).

## When to update this doc

- Added a new xterm addon (WebLinks, search, image, …) → list it.
- Changed how the link provider resolves paths (e.g. picked up the
  pty's live cwd) → update the resolution section.
- Added a new TerminalPane mount site (today: AgentPage + TerminalPage)
  → call out the new host so future work doesn't assume one consumer.
- Added a new `pane_target` (today: working / talking / shell) → note
  what the backend spawns for it.
- Changed terminal comment anchoring (buffer serialization, the
  `TerminalBufferSelector`, or the decoration painting) → update the
  commenting section.
