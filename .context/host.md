# The host

`oxplow-host` (`crates/oxplow-host`) is one headless process per
`OXPLOW_HOME`. It holds no project state. It keeps a registry of projects and
the holds devices have on them, and it runs one `oxplow-daemon` per held
project. Built so far: the core, routing, the served UI, the Mac app's own
host, HTTPS, pairing, the Mac app's Devices window, and the app using a host on
another machine. Not built yet: `/host/events`.

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
- **API**, all under `/host/` (`api.rs`; who may call each, see "Pairing"):
  `GET /host/status` (version, pid, projects with state, `connected` devices,
  and `public`: the listener for other devices, `off`, `listening { url }` or
  `failed { error }`),
  `GET /host/projects` (each with `held_by_you` for a device), `POST
  /host/projects` (JSON `{ path }`: register that folder, or find it
  registered; 400 for a folder that is not a project), `POST|DELETE
  /host/holds` (JSON `{ device, project }`; each answers the project's view;
  404 for an unknown project, 400 for no device), `POST /host/shutdown` (202;
  the host stops, its daemons with it), `POST /host/pairing`, `POST
  /host/pair`, `GET /host/devices`, `PATCH|DELETE /host/devices/<id>`, `GET
  /host/me` and `DELETE /host/me`. A body is read after the caller is known, so
  a missing token is a 401 whatever the body.
- **The owner token** comes from stdin (`--token-stdin`, as the daemon's does)
  or is generated and printed.
- **Flags** of `serve`: `--bind`, `--token-stdin`, `--ui <dir>` (the built UI),
  `--allow-host <name>` (repeatable), `--daemon <path>` (another daemon
  binary, as `BinaryDaemon`: the browser suite runs `oxplow-daemon-sim`) and
  `--for-app` (the app's host: needs `--token-stdin`; stdin after the token
  line is its lifeline), and `--public-bind <addr>` with `--tls-cert <pem>
  --tls-key <pem>` (see "HTTPS").
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
- **Devices window** (Oxplow ▸ Devices…, in a project window or the
  launcher; `host/DevicesSlideover.tsx`): Pair a Device shows a code, its
  countdown and the address to open on the device; the paired devices
  (rename in place, Remove after a confirm, a dot when connected); "Allow
  other devices". Its calls are Tauri shell commands (`commands/devices.rs`)
  to the app's own host client as its owner: no page calls `/host/*`, so the
  host answers no CORS there.
- **Allow other devices** is the shell's own file in the home,
  `host-settings.json` (`oxplow_host::settings`, beside `device.json`):
  `{ allowOtherDevices, address, names, tlsCert, tlsKey }`. The app reads it
  when it starts its host and passes `--public-bind <address>` (port 7400 when
  none is given: fixed, since a browser's origin holds its token), an
  `--allow-host` per name, and the certificate. Never every network by
  default: the person names an address (Use Tailscale fills the Mac's tailnet
  address and MagicDNS name from `tailscale status --json`). A change applies
  at the next app start (the window says whether the host runs with it). On,
  the host starts with the app, not at the first open.
- **Quit asks first** when another device has a window open (`/host/status`
  `connected`): "<names> are using Oxplow on this computer. Quitting stops
  their projects and agents too." The app menu's Quit is a custom item
  (Cmd-Q, handled in Rust: the predefined one is `terminate:` and asks
  nobody), and closing the launcher (an exit with no code) asks the same; a
  Cancel that leaves no window shows the launcher. Dock ▸ Quit and a logout
  cannot ask.
- **Headless.** `oxplow host <args>` runs the sidecar with `<args>` (exec), as
  on a server. The app's own host cannot start on a home a headless host has
  (it says so); use another `OXPLOW_HOME`, or connect to it as below.
- **Other computers** (the launcher's section; `oxplow_host::remote`,
  `commands/remote_hosts.rs`): the app pairs with a host on another machine
  as a `desktop` device, with its address (`https` unless it says `http`) and
  a code from `oxplow host pair` there. The device token is in the keychain
  (account `oxplow-host:<origin>`; the shell's dev build is signed for it,
  DEV.md "Sign dev builds"), the host in `remote-hosts.json` (origin, the id
  and name the host gave this Mac). The launcher lists each host's projects
  (and says when it runs another version); Open holds one there and opens a
  window on `<origin>/p/<id>` with that token and no `projectDir` (its files
  are there). Closing the window lets go there; quitting the app does not
  (the work goes on), and the window comes back at the next start
  (`session.json` keeps it as `<origin>/p/<id>`). Forget tells the host and
  drops the token. The app's CSP allows `https:`/`wss:` for these windows.


## HTTPS

`--public-bind <addr>` listens a second time, for other devices; `--bind`
stays plain HTTP for this machine (the Mac app's windows reach their host on
`127.0.0.1`, where a certificate for the tailnet name would not match). With
`--tls-cert` and `--tls-key` (PEM; `tailscale cert <name>` writes both for
the machine's tailnet name, `mkcert` for a LAN name) the public listener is
HTTPS (`tls.rs`, rustls; each handshake in its own task, 10 s at most; a
certificate that cannot be read, like a bind that fails, leaves the listener
out, never the host, and `/host/status` says why), and
the second stdout line says `oxplow-host listening for other devices on
https://…`. A page reached there gets an `https` base and its component
frames an `https` CSP (`web::Scheme`). A browser needs HTTPS for a secure
context: the microphone, the clipboard.

## Pairing

Two roles (`access.rs`). They are roles, not a wall: a paired device can open
a terminal on the host's machine.

- **The owner** holds the token the host started with: the Mac app's per-run
  token, or a headless host's. It is good only on the loopback listener
  (`--bind`) and on the **owner socket**, `host.sock` in the home (mode 0600;
  every request there is the owner, by the file's mode). The CLI uses the
  socket: `oxplow-host pair` (or `oxplow host pair`) prints a code, `devices
  [remove <id>]`, `status`, `stop`.
- **A device** holds the token it got at pairing, good on every listener.
  `devices.json` in the home keeps `{ id, name, kind, paired_at,
  token_sha256 }` (`devices.rs`): only a hash, so the file never holds a
  token. The host is its only writer and keeps it in memory; a file it cannot
  read stops the host at start with the reason (never read as "no devices").

Routes by role: owner only: `POST /host/pairing`, `GET /host/devices`,
`PATCH|DELETE /host/devices/<id>`, `POST /host/projects`, `POST
/host/shutdown`. Owner or device: `GET /host/status`, `GET /host/projects`,
`POST|DELETE /host/holds` (a device's hold is always its own), `GET /host/me`
(`{ owner, device, build }`), `DELETE /host/me` (a device forgets itself).
Anyone who reaches the host: `POST /host/pair`.

- **A code** (`POST /host/pairing`, owner) is 8 characters with no
  look-alikes (`23456789ABCDEFGHJKMNPQRSTUVWXYZ`), shown `ABCD-EFGH`, good once
  for 10 minutes, kept in memory.
- **`POST /host/pair` `{ code, name, kind }`** answers `{ device, token }`:
  the host makes the device's id (a device never chooses one). It takes only
  `Content-Type: application/json`, and an `Origin` that is the host's own
  when it names one: a page on another site cannot send it without a
  preflight, and no `/host/*` route answers one. A wrong code costs a second,
  one attempt at a time (`WRONG_CODE_WAIT`), so a code's 10 minutes allow 600
  guesses of 31^8, and nobody can keep pairing closed for others.
- **Who a device is** comes from its record: on `ipc/…` the host replaces
  `Oxplow-Device`; the socket's query is rebuilt from its parsed pairs, with
  only `connection` (the page load) as sent and `device`, `name`, `kind`
  from the record. The owner's pass as sent (the Mac app names its device).
- **Removing a device** (`DELETE /host/devices/<id>`, or `DELETE /host/me`)
  revokes its token, releases its holds and closes its sockets with **4003**.
  The host counts open sockets per device for `connected`.
- **The host ending** (a quit, a signal, `POST /host/shutdown`) closes every
  socket with **4002** first, so a page says why.

On the page (`apps/desktop/src/host/`, `transport.ts`): a page the host served
carries no token and says `host: <origin>`. With no token for that host in its
storage (`oxplow.hostToken`) it shows **PairScreen** before the app mounts:
the code and a name for the device; it keeps the token and the host's id for
the device (`oxplow.device`) and loads again. `/` is **HostHome**: pair, then
the host's projects; Open holds one for the device and goes to it, Close lets
go. A refused token (a 401 on a call, a 401 from `GET /host/me` after a drop
— a browser cannot see a socket upgrade's 401 — or a 4003) forgets the token
and pairs again. A 4002 shows "Oxplow was quit on the computer that runs it"
and retries slowly. A project page holds its project for the device at load
and before each reconnect (`checkHost`): the app's host starts again with no
holds, and holding twice is one hold. After each reconnect the page compares `/host/me`'s
`build` (a hash of the served `index.html`) with the one it loaded with, and
loads again when it changed.

## The `Host` check

Every request, whatever its route, must carry a `Host` the host allows: the
loopback names (`127.0.0.1`, `localhost`, optional port; the same
`component_http::loopback_host` the daemon uses) and the names given with
`--allow-host` (name only, any port, case ignored). Anything else is `403`.
Without it a page whose name flips to this machine (DNS rebinding) could call
the host as a same-origin page and act with a token kept there. `--bind` is
loopback only (the owner's token is good there; `serve` refuses another
address); other devices come in on `--public-bind`: `oxplow-host serve
--public-bind 100.101.102.103:7400 --allow-host mac.tailnet.ts.net`.

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
- **Tokens.** `ipc/…` and `events` need the owner's or a device's token
  (bearer; `?token=` on the socket), except a CORS preflight (`OPTIONS` with
  `Access-Control-Request-Method`): a Mac window is `tauri://localhost`, so
  its calls are cross-origin, WebKit asks first and never sends a token
  then, and the daemon's CORS layer answers it. The host drops it and sends the daemon's own, which only the
  host holds. `health`, `components/…` and `component-lib/…` need no client
  token, as the daemon's need none (a sandboxed frame cannot send one); the
  `Host` check is their guard. A device's identity comes from its record (see
  "Pairing").
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
  base: "<scheme>://<Host>/p/<id>", host: "<scheme>://<Host>", project, token:
  null, projectDir: null }` put first in `<head>` (as the Mac shell injects a
  window's, `windows.rs`; read by `windowContext.ts` and `transport.ts`). No
  token: a device uses its own. The meta CSP's `frame-src` also gets the
  page's origin, so a component frame from a non-loopback name loads, and the
  response says `frame-ancestors 'self'`. The page exists for any registered
  project, held or not; the app shows its own disconnected state.
- `/p/<id>/<file>` and `/<file>` are files of the folder: plain names only
  (no `..`, no empty segment, percent escapes decoded first). `assets/…` is
  cached as immutable, the rest `no-cache`.
- `/` is the same `index.html` with `kind: "hostHome"` (pairing, then the
  projects; nothing of them is in the page), a placeholder for the tablet
  shell.

## Later steps

`/host/events` (project states, for the tablet shell).
