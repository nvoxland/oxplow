# The host

`oxplow-host` (`crates/oxplow-host`) is one headless process per
`OXPLOW_HOME`. It holds no project state. It keeps a registry of projects and
the holds devices have on them, and it runs one `oxplow-daemon` per held
project. Built so far: the core (this page). Not built yet: routing, the Mac
shell as a client, pairing.

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
- **Logs:** `<global_config_dir>/logs/host.<date>.log`, daily, a week kept.

## Later steps

Routing (`/p/<project>/…`, the page and its injected context, `/host/events`),
the Mac shell starting and using a host, and pairing with device tokens.
