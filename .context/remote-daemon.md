# Remote daemon mode

Every project backend is an `oxplow-daemon`, including local ones — a
local project window is a client talking to a daemon on 127.0.0.1 (see
[architecture.md](./architecture.md)). "Remote mode" is therefore not a
separate mechanism, just the same client pointed at a daemon on another
machine (e.g. an EC2 dev box). Single-user by design; SSH is the auth
layer. User-facing setup lives in the user docs — this note is the
developer-facing mechanics.

## Pieces

- **`crates/oxplow-rpc`** — transport-neutral command cores +
  `rpc_dispatch!` registry (`dispatch(name, camelCase JSON args,
  &Services) -> JSON`). Single source of truth for the command set;
  both hosts call it. No `tauri` deps, ever — the daemon builds
  headless.
- **`crates/oxplow-daemon`** — headless binary. `--project <dir>`
  (or `OXPLOW_PROJECT_DIR`) + `--bind 127.0.0.1:7420`. Boots
  `Services::boot` → `oxplow_app::boot::run_boot_orchestration`
  (shared with the Tauri shell — recovery, ensure_primary, all
  watchers/indexers) → control-plane spawn (hooks/MCP for the agents
  on that box). Routes: `POST /ipc/:name` (tauri-specta result
  envelope — built by the shared `oxplow_rpc::ipc_envelope`, the single
  Rust owner of the `{status, data|error}` shape; the Tauri path reaches
  the byte-identical shape via the TS `typedError` wrapper + the same
  `IpcError`, so the two hosts can't diverge on error mapping),
  `GET /events` (WebSocket multiplexing
  `{channel:"oxplow"|"lsp"|"terminal"|"acp", payload}` with the exact
  payload shapes the Tauri bridges emit; the `lsp` frame carries
  `LspSessionEvent` from `LspSessionManager` — see `.context/lsp.md`;
  the `acp` frame carries `AcpEvent` from `Services.acp` — see
  `.context/agent-model.md` → "ACP agents"; every channel is subscribed
  before the upgrade is answered, so a client that writes once its socket
  opens never misses the event its write caused — tsk995),
  `GET /health`. Same per-project instance lock as the shell.
  `/ipc` and `/events` require the UI token (see "Auth" below);
  `/health` doesn't. CORS stays permissive so the frontend can run in a
  plain browser (Playwright, a served `dist/`): a page can't obtain the
  token and no cookies are involved, so it exposes nothing.
- **`crates/oxplow-daemon-sim`** — the browser suite's daemon (P11,
  tsk948): the same `oxplow_daemon::run_main` (arguments, boot, server),
  with its secrets in memory (`MemorySecrets`) instead of the keychain,
  so a headless CI runner needs none and a local run writes no test key
  into the person's. They go with the process — a restarted sim no
  longer trusts an approval it recorded. Dev-only: nothing that ships
  depends on it (`no_test_double_is_a_production_dependency`), and the
  shipped `oxplow-daemon` always passes `KeychainSecrets` —
  `Services::boot(layout, secrets)` takes the store, no flag skips it.
  Isolate its global config (approvals, `ai.yaml`) with `OXPLOW_HOME`.
- **Facade guard** — `@tauri-apps/*` may only be imported under
  `apps/desktop/src/tauri-bridge/`; everywhere else funnels native
  access through a bridge module (e.g. `nativeDialog.ts` wraps the OS
  folder picker). `tauri-bridge/no-tauri-imports.test.ts` fails `bun
  test` on any violation, so a native assumption can't leak past the
  switchable transport and break the browser path silently. (The repo
  has no ESLint; this source-scan guard delivers the invariant in the
  existing test step.)
- **`apps/desktop/src/tauri-bridge/transport.ts`** — the frontend
  switch. With no daemon base it delegates to `@tauri-apps/api`; with
  one it fetches `/ipc/:name` and demuxes the `/events` WS with backoff
  reconnect.

  **Which daemon is a per-window question** (tsk258). The shell injects
  `window.__OXPLOW__ = { base, kind, projectDir }` into every window it creates
  (`src-tauri/src/windows.rs`, `initialization_script` — built through
  `serde_json` so a URL can't break out of the literal), so two project
  windows in one shell process can drive two different daemons.
  `resolveBase` takes it from, in order: localStorage
  `oxplow.remoteBase` (the launcher's manual connect — an explicit user
  action outranks the default), the injected base, then
  `VITE_OXPLOW_REMOTE` (dev override). All three are read once at module
  load — switching is a window reload.

  **Routing is per command.** `invokeRoute(name, base, tauriAvailable)`
  sends shell commands (windowing, native menus, clipboard, project
  lifecycle — no daemon serves them) to Tauri IPC and everything else to
  this window's daemon. The table is
  `generated/shellCommands.ts`, emitted from
  `oxplow_tauri_ipc::SHELL_ONLY_COMMANDS` by the `export_ts_bindings`
  test and asserted by the surface-parity test, so the TS side can't
  drift from the Rust definition. With **no Tauri host at all** (a
  plain browser over a tunnel) a shell command has nowhere to go: the
  transport refuses it itself (`"none"`, a `NOT_FOUND` error), with no
  request — no daemon serves one (tsk996). `shellAvailable()` is the one
  check for a Tauri host every bridge uses, and the app doesn't ask for
  what only a shell has (the native menu, recent projects) without one.

  **Signing in to a provider works the same with a remote daemon** (P10):
  the service sends the browser back to a loopback port on the person's
  machine, so the shell listens there (`listen_for_oauth_redirect`,
  `await_oauth_redirect`, `answer_oauth_redirect`, `stop_oauth_redirect` —
  shell commands) and
  the renderer hands each redirect to the daemon
  (`complete_oauth_sign_in`). The core never listens for one. A plain
  browser on a daemon has no shell, so its Sign in is off with the
  reason (`.context/providers.md` → "Credentials and sign-in").

  Every channel
  `listen()` accepts is declared in `channels.ts`'s `CHANNEL_ROUTING`
  registry with a routing class (`multiplexed` = daemon WS in remote /
  Tauri bus locally; `shellLocal` = Tauri bus only). `listen()`'s
  channel arg is the `ListenChannel` union of those keys, so a new
  channel can't be subscribed without being classified (compile error),
  and `listenRoute` switches on the class. Shell-local channels (e.g.
  `menu:command`) still use the Tauri bus in remote mode, but go inert
  (`listenRoute` → `"none"`) when no Tauri host exists — i.e. the
  frontend running in a plain browser for Playwright-driven testing or
  a served `dist/`. The backoff loop keeps the registered channel
  handlers across a drop, so it auto-re-subscribes
  on reconnect. A reconnect *after* a drop (not the first connect) also
  fires the `onRemoteReconnect` handlers, so consumers re-hydrate the
  snapshot they hold and catch up on events missed while the socket was
  down. `triggerRemoteResync()` fires the same handlers manually (used by
  the daemon health-probe recovery path in `App.tsx`). The backoff loop
  is the WS-transport half; the in-place store recovery it drives is the
  next bullet.
- **Auto-resync on reconnect.** Recovery re-hydrates the client stores
  in place — *no* manual full-page reload. The top-level loader
  (`loadInitialAppState` in `App.tsx`: streams, current stream + its
  threads, workspace context, selected-thread work), the core-store
  subscriptions (`useBackendSubscriptions`: backlog, config, agent
  statuses), and comment threads (`useCommentsForTarget`) all register
  `onRemoteReconnect` handlers. The daemon health probe (App.tsx, 2s
  `/ipc/ping` poll) used to `window.location.reload()` on recovery;
  it now `triggerRemoteResync()`s instead, so an HTTP-level recovery
  takes the same in-place resync path (a reload would drop unsaved
  editor drafts). To make a new surface live-again after a drop,
  register an `onRemoteReconnect` re-fetch alongside its event
  subscription.
- **Launcher connect flow** — `launcher/Launcher.tsx`
  `RemoteConnectSection` + `launcher/remoteRecents.ts`. Probes
  `/ipc/ping` before committing. `Root.tsx` renders the full app
  shell for any window without a shell-assigned `kind` — which is what a
  plain browser over a tunnel is.
- **`components/RemoteConnectionBanner.tsx`** — fixed top strip in
  remote mode: red "reconnecting…" while the WS is down (with
  Disconnect). Once it recovers, state auto-resyncs (see above), so the
  banner shows only a brief, non-blocking "Connection restored — state
  resynced" confirmation that auto-dismisses (`RESTORED_AUTO_DISMISS_MS`)
  — no reload prompt. Agents on the daemon box run through the gap.
  (A genuine version/schema skew after a backend upgrade would still
  warrant a reload prompt; there's no skew detection yet, so nothing
  surfaces one today.) **A refused token is not a drop** (tsk971): an
  `/ipc` 401 marks the transport refused (`isTokenRefused`), each call
  fails `UNAUTHORIZED` with what to do, the socket is closed and never
  retried, and the banner shows `remote-banner-refused` — "this window's
  token was refused… reconnect from the launcher" — sticky, since no
  retry fixes it. The health probe counts a refusal as the daemon
  answering, so the "Backend daemon disconnected" overlay doesn't show;
  background reads fail through `readFailed` (logged), never uncaught.

## Lifecycle, logs and stalls

- **A daemon doesn't outlive its app.** The supervising app keeps the
  daemon's stdin open; end-of-file means the app is gone (quit, crashed,
  killed) and the daemon stops (`stop_when_app_goes`).
- **A second app defers.** `.oxplow/daemon.json` names the running
  daemon. Opening a project whose daemon is alive waits a few seconds
  (one whose app just quit is on its way out), then refuses: "already
  open in another Oxplow". It never kills it — a second launch used to
  take the first app's backend down that way (`live_daemon`).
  `oxplow --version` / `--help` print and exit rather than launch.
- **Logs.** The daemon logs to its app's stderr and to
  `.oxplow/logs/daemon.<date>.log` (daily, a week kept, written
  synchronously): a packaged app's stderr goes nowhere. Every exit says
  why — the server stopping, SIGTERM/SIGINT/SIGHUP, the app going, a
  panic (with its thread and backtrace).
- **Stalls.** A plain OS thread watches a heartbeat the runtime bumps
  every second (`diagnostics::spawn_watchdog`). 20 s of no progress —
  every worker busy or blocked — logs a warning and, on macOS, saves every
  thread's stack (`sample`) to `.oxplow/logs/stall-<unix secs>.txt` (the
  newest five kept); the recovery is logged too.

## Deployment model (v1)

Daemon binds loopback only; reach it with
`ssh -L 7420:127.0.0.1:7420 <host>`. No TLS. Multi-user is explicitly
out of scope.

Logging: the daemon logs to stderr at `info,rmcp=warn` unless `RUST_LOG`
says otherwise (`run.rs::run_main`). A line that repeats per event or per
connection is a bug to fix at its source, not noise to filter: in
tsk1077, 91% of an hour's log was two such bugs (tsk1071, tsk1078).

## Auth (tsk345)

Loopback is not a boundary: the agents the daemon runs, the sources it
sandboxes (localhost is allowed) and any web page the person visits can
all reach 127.0.0.1. So `/ipc` and `/events` require a per-launch **UI
token**; only the person's renderer holds it.

- **Minting:** the shell's `DaemonSupervisor` mints 256 random bits per
  daemon and hands them over on **stdin** (`--token-stdin`). Never argv,
  the environment or a file: all are readable by other processes of the
  user, the daemon's own agents included.
- **Lifeline (tsk1073):** after the token the shell keeps that stdin open
  for as long as it lives. End-of-file means the app is gone (quit,
  crashed, SIGKILLed), and the daemon stops with its agents
  (`daemon_supervisor::stop_when_app_goes`: SIGTERM to its group when it
  leads one, else just itself). Anything starting a daemon with
  `--token-stdin` must hold stdin open; closing it stops the daemon. The
  boot-time orphan sweep stays for a daemon from before this.
- **Hand-started daemon:** it mints its own and prints `ui token: …`.
- **Presenting it:** `Authorization: Bearer <token>` on `/ipc`, and
  `?token=` on the `/events` WebSocket (browsers can't set its headers).
  The comparison is constant-time.
- **Where the renderer gets it** (`transport.ts` `resolveToken`), paired
  with whichever source supplied the base:
  - the launcher's manual connect: a token field, or
    `http://host:port#oxplow-token=…`, saved as `oxplow.remoteToken`;
  - the shell's injected `window.__OXPLOW__.token`;
  - `VITE_OXPLOW_REMOTE_TOKEN` in dev.
  A `#oxplow-token=` URL fragment wins (scripted and Playwright
  sessions).
- **Headless dev:** start the daemon, read its `ui token:` line, and run
  vite with `VITE_OXPLOW_REMOTE=… VITE_OXPLOW_REMOTE_TOKEN=…`.

**Ungated: `/health` and custom component bundles** (P6b.D3,
`oxplow-daemon/src/components.rs`). `GET
/components/v/{version}/{*path}` serves a custom component's bundle as
its host loaded it (`load_component`, tsk984): a snapshot of the
bundle's files held in `Services::component_bundles`, keyed by its
version (the component's approval hash), so the frame runs exactly what
was hashed. The version is a path segment, so the bundle's relative URLs
stay in it; the bare folder URL redirects to the raw path plus `/`. It
can't take the token — a frame can't carry it — and needn't: it serves
an extension's own files, never project data (the frame reaches that
only through the host's bridged RPCs). It is mounted **outside the
permissive CORS layer**, so a web page can't read a bundle with `fetch`,
and it answers only a loopback `Host` (`127.0.0.1` or `localhost`,
optional port — `loopback_host`; not `[::1]`, since the page's
`frame-src` can't name an IPv6 literal and a frame there would never load,
tsk1048): without that, DNS rebinding (a
page whose name flips to 127.0.0.1) would read bundles same-origin; the
CSP's bundle-folder source is built from that validated `Host`. 404 for a
foreign `Host`, a version that isn't loaded, or anything but exactly one
of the snapshot's files (a map lookup). Every 200 has `Content-Type`
from a small table (`content_type_for`), `X-Content-Type-Options:
nosniff`, `Cache-Control: no-store`, `Referrer-Policy: no-referrer` and
a CSP that sandboxes the document itself (`sandbox allow-scripts`, so
its origin is opaque however it is loaded, not only inside the host's
iframe) and lets it load only its own files — `default-src 'none'`,
scripts, styles, images and fonts from the bundle folder's URL only —
no `'self'`, which even in a sandboxed frame matches the daemon's whole
origin (the response URL's; checked in Chromium and WebKit) and would let
a frame load another bundle's files (tsk983) — `connect-src 'none'`, `form-action 'none'`,
`base-uri 'none'` (`bundle_csp`); `/component-lib/` is named as a
script source (the client library) and a style source (the kit's sheet,
P11). `style-src` keeps `'unsafe-inline'` for the bundle's own inline
styles — CSS there can fetch nothing from outside, every fetching
directive being bounded to the bundle; inline scripts stay refused. The
page bounds where a frame may go with `frame-src http://127.0.0.1:*
http://localhost:*` — every loopback name the daemon
serves a bundle to, since a browser builds its frames' URLs from the base
it reached the daemon by (tsk1003):
the main window's CSP (`tauri.conf.json`; with no `devCsp`, Tauri
applies the same `csp` in dev) and, for a plain browser, a one-directive
meta CSP in `index.html` (P11, tsk960) — `lens/frameBound.test.ts`
keeps the two one value.

## Dispatch context

`dispatch` takes an `RpcContext { services, plugin_runtime }`
(`crates/oxplow-rpc/src/lib.rs`; derefs to `Services`). The
`rpc_dispatch!` registry has two sections: `svc { … }` cores receive
`&Services` (the ~180 common commands) and `ctx { … }` cores receive
`&RpcContext` (today just `open_terminal_session`, whose agent path
reads `plugin_runtime` — the control-plane hook/MCP URLs + token).
Both hosts populate it from their own control plane: the Tauri
wrapper from the managed `PluginRuntimeState`, the daemon in
`main.rs` from its `ControlPlane` handle. A host that passes
`plugin_runtime: None` degrades cleanly — plain shell terminals
work, agent spawn returns INVALID.

## Known v1 gaps

- **Picking a different project** = restarting the daemon with a
  different `--project` (it's project-scoped, like the shell's
  shell owns windows, daemons own projects). No remote directory browser.
- External-URL tabs and the native menu/clipboard stay local to the
  shell in both modes.
