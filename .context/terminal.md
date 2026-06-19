# Terminal (xterm.js)

What this doc covers: the xterm.js Terminal that hosts the agent
session, plus the file-path link provider that turns `path:line` text
in terminal output into clickable links that open files in oxplow's
editor.

## Single component

`apps/desktop/src/components/TerminalPane.tsx` is the **only** xterm.js
consumer. It's mounted by two page renderers:

- `apps/desktop/src/pages/AgentPage.tsx` — the agent tab, `paneTarget`
  `"working"` / `"talking"`. The backend spawns the thread's assigned
  agent CLI (Claude or Codex).
- `apps/desktop/src/pages/TerminalPage.tsx` — the "Terminal" Page (rail
  entry + `indexRef("terminal")`). Hosts **multiple** shells: it mounts
  one `TerminalPane` per terminal, stacked and toggled `display:none`
  (keep-warm), with a vertical initials strip
  (`components/Terminal/TerminalTabStrip.tsx`) on the left to select
  between them. The strip mirrors the far-left `Navigator`: a thin
  always-visible glyph column, and on hover an overlay slides out with
  full titles + a per-row right-click menu (Rename… / Close terminal). Each pane's `paneTarget` is `"shell"` for the first
  (default) terminal and `shell:<id>` for the rest. The backend
  (`commands/terminal.rs`) early-branches on `pane_target == "shell" ||
  starts_with("shell:")` to spawn the user's `$SHELL -l` (fallback
  `/bin/sh`) rooted at `stream.worktree_path` — no agent command,
  plugin, or system prompt. One persistent shell per (stream, terminal
  id) (session key `<stream>|<pane_target>|<mode>`, so the bare-`shell`
  default reproduces the old `<stream>|shell|<mode>` key and reattaches
  its existing PTY). No `onUserInterrupt`: Escape is an ordinary shell
  keystroke here. See "Multiple terminals" below.

Both go through the same component; only the `paneTarget` (and thus the
server-side spawn) differs.

Agent sessions key their backend PTY on `(stream, thread, agent, pane)`
only — see `agent_session_key` in `commands/terminal.rs`. The thread's
`agent` is in the key, so Claude and Codex threads on the same stream/pane
never reattach to the same PTY. The **transport mode is deliberately NOT in
the agent key** (tsk138): a re-attach that negotiated a different transport
(e.g. a second daemon/browser client) must resume the one live agent PTY,
not spawn a duplicate `claude` in the same worktree. (The shell path keeps
transport in its key via `shell_session_key` — shell sessions may
legitimately differ by transport.) Agent-specific runtime files are
generated under `.oxplow/runtime/` by `oxplow-plugin`; shell terminals skip
that path entirely.

**Read-only session lookup (tsk139).** `lookup_terminal_session({ threadId,
pane? })` (core in `crates/oxplow-rpc/src/commands/terminal.rs`; Tauri shim +
`bindings.ts` `lookupTerminalSession`) returns the live agent `sessionId` for
a thread's pane **without spawning** — `pane` defaults to `"working"`, only
`working`/`talking` are valid. It rebuilds the same `agent_session_key`
`open_terminal_session` uses (resolving the thread's stream + agent from
`thread_store.get`), then calls the registry's read-only
`TerminalSessionRegistry::session_id_for_key` (reads `by_key`, validates the
id still lives in `inner`; a killed-but-stale entry reads as `None`). Returns
`None` when no live session exists (or the thread is unknown). This is the
spawn-free path a second client / automation uses to resolve a thread's agent
PTY before `forward_terminal_input` (the human-keystroke transport), instead
of the spawn-capable `open_terminal_session` — which, since tsk138, is safe to
call twice but still *can* spawn. Like `forward_terminal_input`, the lookup is
UI/second-client only (`ui(...)` in the surface-parity manifest); it is **not**
exposed to the agent (no-automation guard, see `.context/agent-model.md`).

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

## Paste path and input ordering

All paste gestures (Cmd+V, the native `paste` event, drag-drop "Add to
context") funnel through xterm's `term.paste(text)`. xterm's clipboard
helper does two things: normalizes `\r?\n` → `\r`, and — when the
program has bracketed-paste mode on (`\x1b[?2004h`, tracked by xterm
from the PTY output stream) — wraps the text in `\x1b[200~`…`\x1b[201~`.
It then fires **one** `onData` event with the whole payload (verified in
`@xterm/xterm`'s `triggerDataEvent`, which calls `_onData.fire(e)` once).
`TerminalPane`'s `onData` ships that as a single
`sendTerminalMessage({type:"input", bytes:base64})`. In remote/daemon
mode this is one `POST /ipc/send_terminal_message`; the backend
`TerminalSessionRegistry::send` does a single `pty.write` → one
`write_all` on the PTY owner task's FIFO mpsc.

**A single paste is therefore one contiguous, in-order write to the PTY
child — there is no oxplow-side chunking or reordering.** This is
locked by `multi_paragraph_paste_reaches_pty_in_order` and
`large_multi_chunk_paste_preserves_marker_order` in
`crates/oxplow-app/src/terminal_sessions.rs`, which spawn `cat > file`,
send a bracketed multi-paragraph paste through the real `send` path, and
assert the child received the paragraph markers in order (the tests
capture the *child's stdin*, not the renderer event stream — the PTY's
ECHO would otherwise mix a second racing copy of the input into the
output, testing the wrong direction).

> **tsk93 finding (multi-paragraph paste paragraphs reorder in the web
> UI).** The reorder is **not** in oxplow's web→PTY path (proven by the
> tests above). It surfaces downstream in the embedded agent CLI's
> handling of the bracketed paste — the paragraph separators arrive as
> `\r` (xterm's `\n→\r` normalization), and the agent reassembles them
> into one prompt with scrambled order. Confirming/fixing that needs the
> live remote flow with a shim capturing the agent process's stdin
> (info not obtainable statically). A possible oxplow-side mitigation —
> delivering agent-pane pastes with `\n` separators instead of `\r` — is
> tracked as a follow-up but unverified, so it was not shipped.

## Multiple terminals (Terminal page)

`TerminalPage` owns a per-stream list of terminals and renders one
`TerminalPane` per entry. The pure list logic lives in
`apps/desktop/src/components/Terminal/terminalTabs.ts` (unit-tested):
`addTerminal` (auto-numbers "Terminal N", collision-safe), `closeTerminal`
(picks a neighbor as the new active; re-seeds a default when the last one
closes), `renameTerminal`, `normalizeTerminalList`, plus
`paneTargetFor(id)` / `commentTargetFor(streamId, id)`.

The strip (`TerminalTabStrip.tsx`) follows the `Navigator` hover-expand
pattern: clicking a glyph activates; rename and close are right-click
menu items on the hover-overlay row (rename opens an inline input in the
overlay row; close is disabled when only one terminal remains). Close is **not**
an inline `InlineConfirm` `×` anymore — it matches the stream/thread nav.

- **The first terminal uses the sentinel id `DEFAULT_TERMINAL_ID`
  (`"default"`)** → bare `"shell"` pane target + `stream.id` comment
  target. This is the back-compat hinge: the pre-multi-terminal single
  shell and any comments anchored to it keep working with zero migration.
  Additional terminals get random ids → `shell:<id>` pane target +
  `<streamId>:<id>` comment target.
- **Persistence** mirrors `App.tsx`'s `oxplow.layout.v1.*` blobs:
  `oxplow.layout.v1.terminalTabs` is `Record<streamId, {id,title}[]>`,
  `oxplow.layout.v1.terminalActive` is `Record<streamId, terminalId>`.
  The backend session registry is in-memory, so after an app restart the
  list (titles/order/active) is restored but the shells re-spawn fresh.
- **Each pane is keyed `${stream.id}:${id}`** so a stream switch remounts
  it against the right stream's worktree.
- **Close kills, switch detaches.** `TerminalPane` gains a
  `terminateOnUnmount?` prop: when set, unmount calls
  `terminateTerminalSession` (kill PTY) instead of `closeTerminalSession`
  (detach). The page only sets it on a terminal that the user explicitly
  closed, via a two-phase `closingIds` set: the pane first re-renders
  with `terminateOnUnmount` true, then the effect removes it from the
  list so the resulting unmount kills the shell. Stream switches and
  closing the whole Terminal page tab unmount with the flag false →
  detach, so those shells survive and reattach.

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

- Relative paths resolve against the session's **live cwd**, falling
  back to `stream.worktree_path`. On click, `resolveClickedPath`
  (`TerminalPane.tsx`) calls the `terminal_session_cwd` IPC, which reads
  the child process's cwd by pid (the pty exposes `PaneHandle.pid`; the
  `TerminalSessionRegistry` stashes it; `read_process_cwd` reads
  `/proc/<pid>/cwd` on Linux / `lsof -d cwd` on macOS — no extra crate).
  This makes a path printed after `cd`ing into a subfolder open
  correctly. **Caveat:** only the direct `shell` pane benefits — its
  child *is* the shell, so the pid's cwd tracks `cd`. For tmux-backed
  panes the pid is the tmux client, so the read returns `None`/the root
  and we fall back to the worktree (agent panes don't `cd` anyway).
- A missing target is benign: `handleOpenFile` (App.tsx) shows a
  friendly "File not found: <path>" at `warn`, not an error with the
  raw OS code — so a stale link doesn't look like a crash.
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
`{ targetKind: "terminal", targetId: commentTargetFor(stream.id, id),
threadId: null }` — i.e. `stream.id` for the default terminal (preserving
pre-multi-terminal comments) and `<streamId>:<id>` for the rest, so each
terminal's buffer comments stay isolated. When
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
- Added a new `pane_target` (today: working / talking / shell /
  `shell:<id>`) → note what the backend spawns for it.
- Changed how the Terminal page manages its terminal list, persistence,
  or the kill-vs-detach close path → update "Multiple terminals".
- Changed terminal comment anchoring (buffer serialization, the
  `TerminalBufferSelector`, or the decoration painting) → update the
  commenting section.
