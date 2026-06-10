# Remote Daemon

Run the oxplow backend — worktrees, git, agents, watchers — on a
remote machine (an EC2 dev box, a homelab server) and drive it from
the desktop app on your laptop. Agents run in tmux on the remote box,
so they keep working when your laptop sleeps or your connection
drops; you reconnect and pick up where things are.

Single-user by design: the daemon binds to loopback on the remote
host and you reach it through an SSH tunnel. SSH is the auth layer —
there are no accounts, tokens, or TLS to configure.

## Start the daemon on the remote box

Build and run `oxplow-daemon` against a project that's already an
oxplow project (it has a `.oxplow/` directory — open it once in the
desktop app on that machine, or copy one over):

```bash
cargo build --release -p oxplow-daemon
./target/release/oxplow-daemon --project ~/src/myproject
```

It prints the listen address (default `127.0.0.1:7420`; override with
`--bind`) and the tunnel command to run from your laptop. The daemon
is project-scoped, like a desktop window: one daemon serves one
project. To work on a different project, run it with a different
`--project` (a second daemon on another port works fine — they hold
per-project instance locks, so two daemons can't fight over the same
project).

Run it under tmux or systemd if you want it to survive your SSH
session ending.

## Tunnel and connect

From your laptop:

```bash
ssh -L 7420:127.0.0.1:7420 your-remote-host
```

Then open oxplow with no project (a bare launch shows the launcher),
enter `http://127.0.0.1:7420` under **Remote Daemon**, and hit
Connect. The launcher probes the daemon first, so a dead tunnel fails
fast with an error instead of a hung window. Successful connections
are remembered for one-click reconnect.

The window reloads into the full app shell against the remote
project. Streams, tasks, files, terminals, the wiki — everything is
the remote box's state.

## Disconnects

If the connection drops, a red banner appears and the app retries in
the background. The work doesn't stop — agents run in tmux on the
remote box, and watchers/indexers are daemon-side. When the
connection comes back, the banner offers **Reload** to resync; we
don't reload automatically because that would drop unsaved editor
drafts.

To leave remote mode, hit **Disconnect** on the banner (or clear the
connection from the launcher).

## Current limitations

This is the first cut. Known gaps:

- **External URL tabs** open in your local browser context, not the
  remote box.
- **Native menus and clipboard** are your laptop's, as you'd expect.
- Latency is your tunnel's latency. Terminal echo over a
  high-latency link feels like SSH because it effectively is.
