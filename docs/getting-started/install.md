# Install

Oxplow is a desktop app. There are two paths: grab a prebuilt
installer from the latest CI run, or build from source.

!!! note
    Oxplow is early. Builds are unsigned. Macros and APIs change
    freely between releases. If that's a problem for your setup,
    wait — or pin to a specific commit.

## Option 1: prebuilt installer

CI produces an installer for every push to `main`.

1. Open the
   [latest successful run](https://github.com/nvoxland/oxplow/actions)
   on the `main` branch.
2. Scroll to **Artifacts**.
3. Download the artifact for your platform:
    - `oxplow-macos-arm64` → `.dmg`
    - `oxplow-windows` → `.exe`
    - `oxplow-linux-deb` → `.deb`
    - `oxplow-linux-appimage` → `.AppImage`
4. Install / open it like any other app.

On macOS the first launch will be blocked because the build is
unsigned. Right-click the `.app`, choose **Open**, then confirm in
the dialog. After the first launch macOS remembers your choice.

## Option 2: build from source

Requires:

- Node 20+ (or whatever `mise.toml` pins — easiest to use
  [mise](https://mise.jdx.dev) directly)
- [Bun](https://bun.sh) for the package manager and runtime
- A working C/C++ toolchain for `node-pty` and `better-sqlite3`

```bash
git clone https://github.com/nvoxland/oxplow
cd oxplow
mise install        # installs the pinned tool versions
bun install
bun run dev         # launches Electron in dev mode
```

For a packaged build, see [`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md)
in the repo — it covers `electron-builder` invocation, code-signing
notes, and the release tag flow.

## After install

1. Launch oxplow.
2. Use **File → Open Project** to point it at a git repo.
3. Read [Your first stream](first-stream.md) to send a prompt.

Oxplow stores everything project-local under `.oxplow/` inside the
project root: the SQLite database, the wiki notes folder, the
Claude Code plugin oxplow installs, and per-effort snapshots.
Worktrees for non-primary streams live as siblings of the project
root. There is no global state to configure.
