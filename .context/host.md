# The host

`oxplow-host` (`crates/oxplow-host`) is one headless process per
`OXPLOW_HOME`. It holds no project state. It keeps a registry of projects and
the holds devices have on them, and it runs one `oxplow-daemon` per held
project. Built so far: the core, routing and the served UI. Not built yet: the
Mac shell as a client, pairing.

## What is built

- **One host per home.** `oxplow-host serve` takes an exclusive lock on
  `host.lock` in `global_config_dir()`. Once it listens it writes `host.json`
  (`{ base_url, pid, version }`, atomically). A second `serve` for the same home
  exits and names the running host. Dropping the instance removes `host.json`.
- **Registry.** `host-projects.json` lists `{ id, path, name }`. The id is
  `prj-` and 8 hex digits of a hash of the canonical path; if another project
  has it, the next candidate is used. `oxplow-host project add <dir>` refuses a
  folder with no `.oxplow/`. `project list` and `project remove <id>` need no
  running host: the file is read on every use and changed under a lock.
- **Holds.** `host-holds.json` lists `(device, project)`. The host starts
  a project's daemon on the first hold and stops it when the last goes. A
  restarted host starts again what the file holds. Sleep or a lost connection is
  not a release.
- **Daemons** are started through `DaemonSupervisor` with `BundledDaemon` (the
  `oxplow-daemon` beside the host's binary), so each gets its UI token and its
  stdin lifeline from the host: the daemons stop when the host goes. The host
  has no lifeline.
- **States**, in memory: `starting | running | stopping | stopped |
  crashed { code, message }`. One worker thread boots and stops daemons, so
  they boot one at a time. A request changes the hold and queues the work; it
  returns while the project is `starting`. A watcher calls
  `DaemonSupervisor::poll_exits` twice a second and marks an exited daemon
  `crashed`. There is no restart loop: a new hold starts a crashed daemon again
  only when the last crash was over 60 seconds ago.
- **API**, all under `/host/` and all needing the host token (`Authorization:
  Bearer`): `GET /host/status` (version, pid, projects with state),
  `GET /host/projects`, `POST /host/holds` and `DELETE /host/holds` (JSON
  `{ device, project }`; each answers the project's view; 404 for an unknown
  project, 400 for no device). The token comes from stdin (`--token-stdin`, as
  the daemon's does) or is generated and printed. Device tokens and pairing
  replace it in the pairing step.
- **Flags** of `serve`: `--bind`, `--token-stdin`, `--ui <dir>` (the built UI),
  `--allow-host <name>` (repeatable) and `--daemon <path>` (another daemon
  binary, as `BinaryDaemon`: the browser suite runs `oxplow-daemon-sim`).
- **Logs:** `<global_config_dir>/logs/host.<date>.log`, daily, a week kept.


## The `Host` check

Every request, whatever its route, must carry a `Host` the host allows: the
loopback names (`127.0.0.1`, `localhost`, optional port; the same
`component_http::loopback_host` the daemon uses) and the names given with
`--allow-host` (name only, any port, case ignored). Anything else is `403`.
Without it a page whose name flips to this machine (DNS rebinding) could load
`/p/<id>/` and read the token in it. Start a host for other devices as
`oxplow-host serve --bind 0.0.0.0:7400 --allow-host tablet.lan`.

## Routing: `/p/<project>/…`

`web::app` builds the one router; `proxy.rs` forwards. A request under
`/p/<id>/` for `ipc/…`, `health`, `components/…` or `component-lib/…` goes to
that project's daemon with the prefix removed; `/p/<id>/events` is a
WebSocket tunnel to its `/events`.

- **Which daemon.** `Host::reach(id)`: `Ready` (endpoint and token),
  `Starting`, `Down(state)` or `Unknown`. A `starting` project is waited for,
  up to the daemon boot timeout (60 s), then `503` with `Retry-After`. Not
  held, stopped or stopping is `404`, crashed is `502`, an id the registry
  lacks is `404`; each body is short JSON (`error`, `project`, `state`).
- **Tokens.** `ipc/…` and `events` need the host token (bearer; `?token=` on
  the socket). The host drops it and sends the daemon's own, which only the
  host holds. `health`, `components/…` and `component-lib/…` need no client
  token, as the daemon's need none (a sandboxed frame cannot send one); the
  `Host` check is their guard. `Oxplow-Device`, `Oxplow-Connection` and the
  socket's `device`, `connection`, `name`, `kind` query pass as the client
  sent them; pairing replaces them with the device token's.
- **The tunnel** opens the daemon's socket first (a daemon that refuses is a
  `502`, not a socket that opens and closes). Frames pass both ways, each way
  in order: text, binary, ping, pong and close with its code. A send waits for
  the other side, so a slow reader slows the writer. The daemon's ping and
  `beat` reach the page; the page's pong and `used` reach the daemon.
- **Redirects.** A `Location` that is an absolute path (the daemon's
  `/components/v/<v>` to `…/`) gets the `/p/<id>` prefix.
- **Component frames.** The daemon builds a bundle's CSP from the base the
  page reached it by. The host sends `Oxplow-Public-Base:
  http://<Host>/p/<id>` (any copy a client sent is dropped); the daemon
  believes it only on a request with its own token and only when it is a plain
  `http(s)` URL (`component_http::public_base`), else it keeps the loopback
  `Host` check. The CSP's sources are `<base>/components/v/<v>/` and
  `<base>/component-lib/`.

## The served UI

`--ui <dir>` is the built UI (`apps/desktop/dist`), read at run time, never
embedded. Without it the host looks for `ui/` beside its binary,
`../Resources/ui`, then `../../apps/desktop/dist` (a checkout's
`target/<profile>`); with none, the pages answer `404` saying so. Vite builds
with `base: "./"`, so the same files load under `/` and `/p/<id>/` (and from
Tauri's asset protocol).

- `/p/<id>/` is `index.html` with `window.__OXPLOW__ = { kind: "project",
  base: "http://<Host>/p/<id>", project, token, projectDir: null }` put first
  in `<head>` (the Mac shell's injection, `windows.rs`; read by
  `windowContext.ts` and `transport.ts`). The token is the host token until
  pairing: **anyone who can load the page has it**, so use it on loopback or a
  trusted network only. The meta CSP's `frame-src` also gets the page's
  origin, so a component frame from a non-loopback name loads. The page exists
  for any registered project, held or not; the app shows its own disconnected
  state.
- `/p/<id>/<file>` and `/<file>` are files of the folder: plain names only
  (no `..`, no empty segment, percent escapes decoded first). `assets/…` is
  cached as immutable, the rest `no-cache`.
- `/` is a plain list of held projects (name, state, link), a placeholder for
  the tablet shell; it shows no paths. The app's own launcher needs the Mac
  shell's commands, so a browser cannot show it.

## Later steps

`/host/events`, the Mac shell starting and using a host, and pairing with
device tokens.
