# Terminal (xterm.js)

What this doc covers: the xterm.js Terminal that hosts the agent
session, plus the file-path link provider that turns `path:line` text
in terminal output into clickable links that open files in oxplow's
editor.

## The CSP carve-out that keeps colour working (tsk247)

`tauri.conf.json` sets
`app.security.dangerousDisableAssetCspModification: ["style-src"]`.
**Do not remove it** — the name oversells the risk, and dropping it breaks
every terminal in packaged builds while dev stays fine.

Tauri hardens the CSP for packaged apps in two steps: it injects
`nonce="__TAURI_STYLE_NONCE__"` into every `<style>` at build time
(`tauri-utils/src/html.rs`), then swaps in a real nonce and **appends
`'nonce-…'` to `style-src`** at serve time (`tauri/src/manager/mod.rs`).
Per CSP3, a directive carrying a nonce-source makes **`'unsafe-inline'`
ignored** — so the `style-src 'self' 'unsafe-inline'` we declare silently
becomes nonce-only once packaged.

xterm.js builds its palette and row font rules into a `<style>` element it
creates at runtime, which has no nonce and is therefore blocked. That single
cause produces both halves of the symptom:

- **every ANSI colour disappears** — 16-colour, 256, and truecolour alike,
  because `.xterm-fg-*` / `.xterm-bg-*` never apply; text falls back to
  `theme.foreground`, i.e. uniform white;
- **the font stops being monospace**, because the same sheet sets the row
  font-family, so rows inherit the page's proportional UI font.

The rest of the app is unaffected, which is what makes this confusing to
diagnose: React writes inline styles through the CSSOM (`el.style.x = …`),
and CSP governs only `<style>` elements and markup `style=` attributes.

The carve-out does **not** weaken the declared policy — it makes the
*effective* policy match the one already written in `csp`. `script-src`
keeps its nonce hardening.

Ruled out by measurement during the original diagnosis; don't re-tread these
if the symptom reappears: a `CI` env var reaching the agent (absent),
missing `LANG`/locale (banner output byte-identical with and without),
fonts missing from the bundle (all four woff2 are embedded in the binary),
and an xterm font-metrics race at mount (a resize changes nothing).

## The terminal viz and its host

The terminal is drawn by one host in `apps/desktop/src/terminal/`, in
three pieces:

- **`TerminalSurface.tsx`** — the **only** xterm.js consumer: the screen,
  the font it opens in, the keys and gestures oxplow adds, the file-path
  links. It knows no session: what the person types comes out of
  `onData` / `onBinary`, a fit out of `onResize`, and `onOpened(term)`
  hands the opened terminal over.
- **`useTerminalSession(uri, …)`** — the PTY its URI names, opened at the
  surface's size and replayed into it, followed (output, exit), and opened
  again when it must be. What the person types it encodes and writes at
  once to the terminal's URI (`terminalInput`, see "Paste path and input
  ordering"): it keeps no queue and no buffer. It is the one place the
  `{type:"input"}` messages are built (the no-automation guard names it).
  Unmounting detaches; the process keeps running.
- **`TerminalView.tsx`** — the two together, its visibility measured
  (`useShown`: a hidden tab is `display: none`, so its box has no size),
  and what the session says when it isn't running: stopped (Start), ended
  or failed to start (Start Again / Retry), unable to resume (Retry,
  Start Fresh). An agent's parts (`agent`: stopped, its conversation,
  Start, Start Fresh, the Escape interrupt) are given only for an agent
  session; a shell has none, and Escape is an ordinary keystroke there.

Two pages draw it:

- **The Terminal page** is oxplow-foundation's `terminal` page over the
  kit's **`terminal` viz** (`viz/TerminalViz.tsx`, `terminal: { of,
  title?, stopped?, conversation? }`): one `TerminalView` per row, all
  mounted and kept warm (`display: none`), the picked one shown. With
  several, the strip picks — the page's selection (`select` on its
  `title` column), so Back un-picks — and its row menu renames (the page's
  `rename` action, with the typed title) or closes (its `close` action,
  which asks first). `+` and **New Terminal** run `oxplow.terminal.open`;
  a stream with none opens one the first time the page shows (the viz is
  its own empty state: a terminal page with no rows isn't "No rows."). A
  clicked file path opens in its own tab. An agent reads the page as the
  terminals' names, never their output.
- **An agent session's page** is oxplow-foundation's `agent-session`
  document: its `agent-session-terminal` section is a `terminal` viz of one
  row — the session's URI, `stopped`, `conversation` and `thread` — so the
  viz gives its `TerminalView` the `agent` parts (Start, Start Fresh, the
  Escape interrupt in the row's thread); its `agent-session-waiting`
  section above is the thread's `approvals`. Its actions (Rename…, Switch /
  Fork Conversation…, Stop / Start Agent) are the nav bar's and the tab's
  right-click menu.

The backend (`commands/terminal.rs`, `open_terminal_session { uri, cols,
rows }`) resolves the URI — a terminal, or an agent session; anything
else is refused — and for a terminal spawns the user's `$SHELL -l`
(fallback `/bin/sh`) rooted at the terminal's stream's `worktree_path`,
with no agent command, Claude Code plugin or system prompt. One persistent
shell per terminal: the registry key is the URI, so a re-attach (another
window, a browser client) resumes it.

An agent PTY is keyed by its **agent session** only (`agent_pane_key`,
`session|ses3`; the URI names the session): `open_terminal_session` reads
the session, then its thread and stream (never the person's selection),
refuses a closed session,
resumes from the session's resume id and passes `OXPLOW_SESSION`. Two
sessions in one thread are two PTYs; a re-attach from another window or
transport resumes the one live agent PTY, never a duplicate `claude` in
the same worktree. The spawn binds the PTY to its `AgentPane { thread, session }`:
its output stamps the session's liveness and its exit ingests a
`SessionEnd` for the session. Agent-specific runtime files are generated
under `.oxplow/runtime/` by the session's harness (`oxplow-harnesses`: the Claude MCP config and the
opencode prompt file per session); shell terminals skip that path
entirely.

**Restoring tabs starts nothing.** Every tab body stays mounted
(`display:none`), so an agent session's session opens with
`startOnActivate`: until its view is shown it opens no session — except
that it attaches, through the spawn-free `lookup_terminal_session`, to an
agent that already runs. The first time the tab is shown it opens the
session (resuming its saved conversation). Leaving the tab never stops the
process, and nothing is typed on restore. Shell terminals open at once.
The deciding rule is `mayStartNow` in `terminalSession.ts`.

**A reattach takes the pane's size.** `attach_or_create` resizes a live
PTY to the cols/rows the attaching pane measured before it replays. The
pane's first `ResizeObserver` callback fires before its session id is
back, so it sends no resize; without this, an agent that outlived an app
restart kept the old window's size and drew its input and status lines
past the pane's bottom, on top of each other, until the window was
resized. Locked by `reattaching_resizes_the_live_session_to_the_new_pane`.

**Stopped stays stopped.** `oxplow.agent_session.stop` (a person's only)
sets `agent_session.stopped_by_user_at` and stops the process; the slot
and its saved conversation stay. While it is set `open_terminal_session`
refuses to spawn and answers the `STOPPED` code, the pane (given
`stopped` from the session row, or finding out from that code) shows
"Stopped" with **Start**, and `agent_session.start` clears it — the row
change then opens the session, resuming the conversation. A crash or
backend restart never sets it, so those resume when the tab is shown.
Only the person stopping sets it: we do not infer a stop from an exit,
because no harness tells us the person typed `/exit`. The agent-tab
context menu has **Stop Agent** / **Start Agent**.

**Resume outcomes.** A launch that resumes a saved conversation never
falls back to a fresh one (`Resume::Saved` is `exec <agent> <resume args>`,
no `||`). The pane is told what happened:

- The pane shows no note about how it connected (joined a live process,
  started fresh, or resumed the saved conversation): the replayed screen
  or the agent's own resume output says it.
- The `exit` message carries an `outcome` (`session_outcome.rs`): a
  resume that exited nonzero **before the session recorded a turn**
  (`turn_since`: no `agent_turn` at or after the launch) is
  `unable_to_resume` — `transcript_missing` when the harness flagged it,
  else `exited_early`, with the exit code. A nonzero exit after a turn, a
  clean exit and a stop by the person are an ordinary `ended`. We don't
  parse the output for auth versus config versus transport; the terminal
  shows it.
- The ended banner for `unable_to_resume` keeps the saved conversation and
  offers **Retry** (resume the same id), **Choose another…** (where the
  harness declares `conversation_list`; "Choosing another conversation"
  below) and **Start Fresh** (a second click confirms; runs
  `agent_session.start_fresh`, the only way to a new conversation).
  Another end offers **Start Again**, which also resumes.

**Choosing another conversation.** A harness that declares
`conversation_list` lists its saved conversations for the session's
worktree (`AgentHarness::conversations`: Claude's transcripts under its
config dir, named by their last `custom-title` / `ai-title` record with the
first thing the person typed; opencode's `session list --format json`;
Codex's daemon's `thread/list`), read as `v_agent_conversation`. The
agent session page's **Switch Conversation…** / **Fork Conversation…**
(offered `when` the harness declares `conversation_list` /
`conversation_fork`) are forms whose conversation is a select from the
`agent-session-conversations` page — title, else first prompt, else id,
saying "this session's" and "open in another session" (from
`v_agent_session_binding`) — and run
`oxplow.agent_session.resume_conversation` (refused for one open in
another session, naming it; Undo goes back) or `fork_conversation` (the
next launch forks it: `--resume <id> --fork-session`, Codex's `thread/fork`,
`opencode -s <id> --fork`). Both stop the running process; the session's
`conversation` (its resume id and `fork_next`, one value) changes, and its
terminal opens again once that process has exited.

**Earlier sessions: the same lists, with no session to ask.** The New
Session page lists the thread's earlier sessions under its Start row, so a
person can bring one back without first opening a session to switch. One
listing (the `new-session-earlier` page, oxplow-foundation's, over
`v_agent_session` and `v_agent_conversation`) merges the
stream's closed oxplow sessions with every harness's saved conversations
for the worktree (`harness_conversations`, what `v_agent_conversation`
reads; each harness is asked at once, one
that fails or answers `Unavailable` becomes a row with its reason and the
rest still list), on (harness, native id), newest activity first, and
decides each row's action itself: **reopen** (this thread's session a
person closed: `oxplow.agent_session.reopen`, whose toast offers Undo),
**resume** (a free conversation, an outside one or one from a session
closed in another thread or by a close of its thread: a new session here,
`oxplow.agent_session.open { resume }`, recorded `chosen`), **switch** (open
in a session on this thread: show its tab) or **none** (open in another
thread's session; the only row with nothing to take). A closed slot that
never ran anything, or whose action would be none (another thread's, with
no conversation to resume), is left out. A session's name is
`AgentSession::display_name` (title, else the harness's name, else harness
and day: the one rule, mirroring `v_agent_session.display_name`); the page
labels a row with the given name, else a prompt.
Nothing is adopted until a person picks it. The page filters in the client
(title, prompts, thread, agent; ↑/↓, Enter, Escape clears the filter).
A closed session's ref no longer opens nothing: it opens the
`session-transcript` page, which has a **Reopen** action.

**Read-only session lookup (tsk139).** `lookup_terminal_session({ uri })`
(core in `crates/oxplow-rpc/src/commands/terminal.rs`; Tauri shim +
`bindings.ts` `lookupTerminalSession`) returns the live PTY `sessionId` of
a terminal or an agent session **without spawning**: the registry's read-only
`TerminalSessionRegistry::session_id_for_key` under the same key
(`terminal_key`, or `agent_pane_key`) (reads `by_key`, validates the id still lives in
`inner`; a killed-but-stale entry reads as `None`). Returns `None` when no
live PTY exists. This is the spawn-free path a hidden agent tab uses to
find out whether its agent runs, instead of the spawn-capable
`open_terminal_session` — which is safe to call twice but still *can*
spawn. Like terminal input, the lookup is UI/second-client only (`ui(...)` in the surface-parity manifest); it is **not**
exposed to the agent (no-automation guard, see `.context/agent-model.md`).

The surface owns:

- The xterm `Terminal` instance + `FitAddon`. **The inner gutter (~8px)
  goes on a wrapper *around* the `hostRef` mount, never on the mount
  itself** — `FitAddon`/`ResizeObserver` measure `hostRef` directly, and
  padding the mount makes the computed row count oscillate (the text
  visibly jumps). The wrapper's `background` matches `XTERM_THEME.background`
  so the gutter reads as the terminal's own padding.
- **It opens xterm only once the mono web font has loaded**
  (`document.fonts.load`, tsk1042). xterm measures its cells when it opens
  and again only *after* a resize, so a terminal opened while JetBrains
  Mono was still loading measured the fallback's 15px rows. Its next fit —
  a strip appearing above it, which changes the pane but not the window —
  sized the rows by those, then re-measured at 17px and overflowed the
  pane, input line below the fold. (A window resize re-measured on its
  own, which is why it looked fine after one.) The e2e holds the font back
  and checks the face was loaded when `.xterm` appeared;
  `document.fonts.check` can't, as it answers true while a face is still
  loading.
- A custom keydown handler (Cmd+V paste, Shift+Enter, PageUp/Down as a
  page of xterm's own scrollback, Escape interrupt detection).
- Fast scroll is **Alt**+wheel (`fastScrollSensitivity: 4`). xterm 6
  dropped the `fastScrollModifier` option and hardwired Alt, so it is
  no longer configurable (it was Shift before the v6 upgrade).
- A custom wheel handler that leaves a Ctrl/Cmd-wheel to the page.
  Scrollback is xterm's own; there is no tmux history mode (tsk1018).
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
The surface's `onData` (through the session's `input`) ships that as a single
`terminalInput(uri, {type:"input", bytes:base64})`: one frame on the
daemon's socket. The terminal's input queue writes it with a single
`TerminalSessionRegistry::send` → one `pty.write` → one `write_all` on the
PTY owner task's FIFO mpsc.

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

**Between messages, one stream keeps the order.** Every message a view
writes — each keystroke's `onData`, a resize, a history move — is one
frame on the daemon's `/events` WebSocket (`sendSocketFrame` in
`tauri-bridge/transport.ts`): `{"channel":"terminal","uri":…,"message":…}`.
Every project window drives a daemon, a local one too
(`.context/remote-daemon.md`), so the socket is the one route. Frames on
one socket arrive in the order they were written, and the daemon reads
each (`terminal_frame` → `terminal_input`, sync) before the next. Separate
`/ipc` requests didn't keep the order — a later keystroke could overtake
an earlier one (the browser suite typed `$((6*7))` and the shell got
`$((67*))`) — and cost a round trip each. The window doesn't wait for an
answer; none comes. Frames written while the socket is still opening are
sent first once it opens; with the socket closed they are dropped (the
daemon is gone, and reconnecting opens the terminal again).

**The backend holds keys until the PTY is up.** Input is addressed by the
terminal's **URI** (known before its PTY exists), never by session id. In
the daemon, `TerminalSessionRegistry::enqueue_input(key, message)` puts it
on the key's queue (created on first use); the queue's drain task is the
only caller of `send`. Per message, in order: find the key's live session;
with none yet (still spawning), wait on the registry's `ready` signal
(raised as each session registers) up to `INPUT_WAIT` (10 s); then write
it. Input whose PTY never comes (a launch failure, which the view shows)
is dropped with the messages queued behind it, never written later into
another process. A session that ends under a write is waited for once
more. The queue goes with its session (`close`, or the exit unregister),
so keys typed after an exit wait in a fresh queue for the next one. The
window keeps no buffer: a fast typist's first keys wait in the daemon.
Locked by `input_written_before_the_session_exists_arrives_once_it_does`,
`many_inputs_arrive_in_order` and `input_for_a_key_that_never_opens_is_dropped`
(`terminal_sessions.rs`), `a_terminal_frame_on_the_socket_reaches_the_pty_in_order`
(`oxplow-daemon`), and the e2e burst typed as the terminal appears.

Each `onData` is its own message, so a bare Escape and the Enter after it
are two PTY writes, as they were typed. Nothing merges keys any more.

## When a session ends or goes away (tsk1026)

A session's process exiting sends `{type:"exit", exitCode}`; the registry
then unregisters it (`TerminalSessionRegistry`'s forwarder removes it from
both maps), so attaching to its key spawns a fresh one instead of replaying
the dead one. The pane shows **Session ended (exit N)** with **Start Again**
and takes no keys meanwhile (they must not reach the next session).

A session that never got a process shows the same banner. When the open
itself fails for a reason other than a stop — Codex not installed where
oxplow looks, any launch error raised before a PTY exists — the pane
takes the ended state with a `launch_failed` outcome: **Couldn't start the
session: <the error>** and **Retry** (`failedLaunch` in `terminalSession.ts`;
the same reopen, so the person can fix the cause and retry without
reopening the tab). It takes no keys, and a reconnect tries the open again.

The pane opens its session through one `openSession` path — first mount,
Start Again, and `onRemoteReconnect` (the daemon came back). Opening again returns the same id
for a session that survived (nothing changes) or a new one: the screen is
reset and the new session's replay is written under a dim "the previous
session ended; this one was started again" line. Helpers in
`components/terminalSession.ts`.

**Ending a session's processes.** portable-pty makes each pane's child the
leader of its own session and process group, outside the daemon's group, so
the group is everything the pane started: an agent's workers, its tool
processes, a shell's jobs. Killing a pane (`PtyManager::kill`, which every
stop goes through: `TerminalSessionRegistry::close`, an agent session's
stop or close, a thread's close) ends that group, not just the child. The
group gets a SIGHUP, as a closed terminal gives it, and a SIGKILL after
`HANGUP_GRACE` (500 ms) if anything ignored it (`end_tree`). The kill runs
off the manager's owner loop, so one slow kill doesn't hold the other panes.
The daemon's exit ends every pane at once (`Services::shutdown` →
`TerminalSessionRegistry::close_all` → `PtyManager::shutdown`): its
`process::exit` runs no destructors, and no signal to the daemon's own
group reaches the panes' groups.

## Terminals are rows

A stream's terminals are rows of the `terminal` table (`id`, `stream_id`,
`title`, `position`, `created_at`; `SqliteTerminalStore`), read as the model
`v_terminal` (with its `uri`, `oxplow:oxplow/terminal?id=trm3`) and named by
the `oxplow/terminal` resource. Three commands (`commands/terminal.rs`, scope
`terminals.write`) write them: `oxplow.terminal.open { stream, title? }`
(auto-numbered "Terminal N": one past the highest numbered title and never
below the count, so a generated title never repeats after renames; answers
`{ id, uri, title }`; an agent opens only in its own stream),
`oxplow.terminal.rename { terminal, title }` (undoable) and
`oxplow.terminal.close { terminal }` (`External`: the shell the registry keeps
under the URI ends, then the row goes; a person's — an agent is refused — and
confirmed; the last terminal may close). They log `terminal.opened@1`,
`renamed@1` and `closed@1`. A stream's archive ends its terminals' shells and
deletes their rows with it.

The Terminal page is a page over the rows (above). A stream with none when
the page first opens gets one (once per page: closing the last leaves the
page empty with **New Terminal**). The old pane-target strings (`shell`, `shell:<id>`,
`ses3`) and the page-memory list of terminals are gone; a shell is its URI.
Not migrated: terminals the page memory held (shells re-spawn after a restart
anyway; a renamed title is lost once). Terminal output takes no comments:
a buffer that scrolls, wraps and evicts has nothing stable to anchor to (a
session's transcript is what to comment on).

The strip (`TerminalTabStrip.tsx`) runs on the shared
`useSlideoutStrip` hook (`.context/usability.md` → "Hover reveals, click
rearranges"), so it behaves exactly like the `Navigator`: **hovering a
glyph shows its title as a tooltip and nothing else**; the panel expands
only on a click (the bottom-pinned `terminal-tab-expand` chevron, or the
strip's dead space) and closes on a background click, pointer-leave,
Escape, or an outside press. Clicking a glyph activates that terminal;
rename and close are right-click menu items on the panel row (rename
opens an inline input there, opening the panel again — pressing the menu,
outside the panel, closed it; the last terminal may close). Close is **not** an inline `InlineConfirm` `×` anymore — it
matches the stream/thread nav.

Two behaviors the hook fixed here (tsk270), both previously hand-rolled
and wrong: the strip had **no outside-press dismissal at all** (only
Escape), so the panel could sit over the xterm surface swallowing clicks;
and its close timer cleared `renamingId` outright, so drifting the
pointer away mid-rename **silently discarded the edit**. An open rename
now passes the hook's `guard`, which suppresses the passive closes
(pointer-leave, background click) while leaving the explicit ones
(Escape, outside press, the collapse chevron) working.

- **Each pane is keyed by its terminal's URI**, so a stream switch mounts
  the other stream's panes against their own worktrees.
- **Close ends, switch detaches.** Closing is the command; a pane's unmount
  only detaches (`closeTerminalSession`), so switching terminals, streams or
  closing the whole Terminal page tab leaves the shells running and a
  re-attach replays the buffer. The open one is history state
  (`terminalActive`, `Record<streamId, terminalId>`).

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
  raw path text plus optional line/column. **The shape scan is memoized
  per row text** (`createMemoizedScanner`, a bounded LRU — a hit refreshes
  recency)
  so xterm's per-visible/hovered-row `provideLinks` calls don't re-run
  the regex over unchanged rows on every scroll/hover (tsk39) — the key
  is the row text, so a content change auto-invalidates. The
  `validatePath` gate is **not** cached (the workspace index can change);
  it's applied fresh on each call over the cached shape matches. `TOKEN_RE`
  is module-level (compiled once).

The surface wires `onActivate` to its `onOpenFile` prop, resolving
relative paths against the prop `worktreePath` first (`/`-prefixed
paths pass through; `~/` paths are dropped — the frontend doesn't
know HOME).

## Path resolution and caveats

- Relative paths resolve against the session's **live cwd**, falling
  back to `stream.worktree_path`. On click, `resolveClickedPath`
  (`terminal/TerminalSurface.tsx`, through the session's `cwd`) calls the `terminal_session_cwd` IPC, which reads
  the child process's cwd by pid (the pty exposes `PaneHandle.pid`; the
  `TerminalSessionRegistry` stashes it; `read_process_cwd` reads
  `/proc/<pid>/cwd` on Linux / `lsof -d cwd` on macOS — no extra crate).
  This makes a path printed after `cd`ing into a subfolder open
  correctly. **Caveat:** only the `shell` pane benefits — its child *is*
  the shell, so the pid's cwd tracks `cd`; an agent pane's child is the
  agent, which doesn't `cd`, so its cwd is the worktree anyway.
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
- Added a new `TerminalView` mount site (today: the `terminal` viz and
  the agent session's page) → call out the new host.
- Made another URI open a PTY (today: a terminal, an agent session) → note what
  the backend spawns for it.
- Changed how terminals are opened, listed or closed → update "Terminals are rows".
- Changed the tab strip's expand/collapse behavior → change it in
  `components/useSlideoutStrip.ts` (shared with the `Navigator`), not
  here; update "Terminals are rows" only if the strip stops using it.

