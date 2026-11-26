# The host

`oxplow-host` (`crates/oxplow-host`) is one headless process per
`OXPLOW_HOME`. It holds no project state. It keeps a registry of projects and
the holds devices have on them, and it runs one `oxplow-daemon` per held
project. Built so far: the core, routing, the served UI and the Mac app's own
host. Not built yet: pairing.

There are two kinds of host, and no host ever stops because nothing is held
or nobody is connected:

- **The app's host** (`serve --for-app`): the Mac app's child. The app starts
  it at its first open, and it ends with the app (quit, crash or kill), every
  project and every other device's connection with it. See "The Mac app".
- **A headless host** (`oxplow host serve …`, or `oxplow-host serve` itself):
  for a server. It runs until it is stopped: a signal or `POST
  /host/shutdown`.

## What is built

- **One host per home.** `oxplow-host serve` takes an exclusive lock on
  `host.lock` in `global_config_dir()`. Once it listens it writes `host.json`
  (`{ base_url, pid, version, for_app }`, atomically). A second `serve` for the
  same home exits and names the running host and its kind; the app's host first
  waits up to 15 s for a previous app host that is ending (it stops its daemons
  after a quit). Dropping the instance removes `host.json`.
- **Registry.** `host-projects.json` lists `{ id, path, name }`. The id is
  `prj-` and 8 hex digits of a hash of the canonical path; if another project
  has it, the next candidate is used. `oxplow-host project add <dir>` refuses a
  folder with no `.oxplow/`. `project list` and `project remove <id>` need no
  running host: the file is read on every use and changed under a lock.
- **Holds** are `(device, project)`. The host starts a project's daemon on the
  first hold and stops it when the last goes. Sleep or a lost connection is not
  a release. A headless host keeps them in `host-holds.json` and starts again
  what the file holds when it restarts. The app's host keeps them in memory
  (`HoldsKept::InMemory`): they are the app's open windows, and a tablet holds
  its projects again when it comes back.
- **Daemons** are started through `DaemonSupervisor` with `BundledDaemon` (the
  `oxplow-daemon` beside the host's binary), so each gets its UI token and its
  stdin lifeline from the host: the daemons stop when the host goes. A project
  whose `.oxplow/daemon.json` names a live process of another oxplow (an older
  app, a daemon started by hand) is not started: it is `crashed` with a message
  that names the process, after a short wait for one that is ending.
- **States**, in memory: `starting | running | stopping | stopped |
  crashed { code, message }`. One worker thread boots and stops daemons, so
  they boot one at a time. A request changes the hold and queues the work; it
  returns while the project is `starting`. A watcher calls
  `DaemonSupervisor::poll_exits` twice a second and marks an exited daemon
  `crashed`. There is no restart loop: only a new hold (a person opening the
  project again) starts a crashed daemon.
- **API**, all under `/host/` and all needing the host token (`Authorization:
  Bearer`): `GET /host/status` (version, pid, projects with state),
  `GET /host/projects`, `POST /host/projects` (JSON `{ path }`: register that
  folder, or find it registered; 400 for a folder that is not a project),
  `POST /host/holds` and `DELETE /host/holds` (JSON `{ device, project }`; each
  answers the project's view; 404 for an unknown project, 400 for no device),
  and `POST /host/shutdown` (202; the host stops, its daemons with it). The token comes from stdin (`--token-stdin`, as
  the daemon's does) or is generated and printed. Device tokens and pairing
  replace it in the pairing step.
- **Flags** of `serve`: `--bind`, `--token-stdin`, `--ui <dir>` (the built UI),
  `--allow-host <name>` (repeatable), `--daemon <path>` (another daemon
  binary, as `BinaryDaemon`: the browser suite runs `oxplow-daemon-sim`) and
  `--for-app` (the app's host: needs `--token-stdin`; stdin after the token
  line is its lifeline).
- **Logs:** stderr and `<global_config_dir>/logs/host.<date>.log`, daily, a
  week kept. Stdout carries only the `oxplow-host listening on <url>` line (and
  a generated token).

## The Mac app

The app's host is a child of the app, as a daemon is a child of the host
(`oxplow_host::client`, `ShellWindows` in `oxplow-tauri-ipc/src/windows.rs`).

- **Start.** At the first open the app runs the `oxplow-host` beside it (the
  bundle's sidecar, `target/debug/oxplow-host` in a checkout) as `serve --bind
  127.0.0.1:0 --token-stdin --for-app`, writes a token made for this run, keeps
  stdin open and reads the `listening on` line (`start_own_host`). The token is
  never kept anywhere. A host that cannot start (a headless host has the home)
  ends, and its last stderr lines are the app's error. A host that has ended
  (crashed) is started again at the next open; the windows it served stay on
  their dead socket.
- **Open** (`HostClient::open`): `POST /host/projects` with the folder, a hold
  for the app's device, then a poll until the daemon runs. A daemon that does
  not start is released again and its message is the open's error. The window
  gets `<host>/p/<id>` as its base and the host token.
- **Close** of a window releases its hold (in the background). A tablet that
  holds the project keeps it running.
- **Quit** drops the lifeline and waits for nothing: the host stops its daemons
  and ends. Nothing is released first. The windows to reopen are the app's own
  `session.json`, as before.
- **Headless.** `oxplow host <args>` runs the sidecar with `<args>` (exec), as
  on a server. The app does not use a headless host on its own home before
  pairing: it has no token for it.


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

`/host/events` (project states, and "the host is ending" so a tablet can say
why), pairing with device tokens (the fixed port and allowed names of "Allow
other devices" are given to the app's host at its start), a warning at quit
when another device is connected, and the app using a headless host.
