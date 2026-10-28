# Install

Oxplow is a desktop app — a Tauri 2 shell wrapping a React /
Monaco / xterm frontend over a Rust backend. There are two paths:
grab a prebuilt installer from the latest CI run, or build from
source.

!!! note
    Oxplow is early. Builds are unsigned. APIs and on-disk shapes
    change between releases. If that's a problem for your setup,
    wait — or pin to a specific commit.

## Option 1: prebuilt installer

CI produces installers for every push to `main`, and tagged
releases (`v<version>`) attach the same bundles to a GitHub
Release.

### macOS: Homebrew (recommended)

The builds are unsigned, so macOS's Gatekeeper refuses to launch
them when downloaded directly from a browser — the dreaded
**"Oxplow.app is damaged and can't be opened"** dialog. Homebrew
sidesteps the whole mess by stripping the quarantine attribute
on install. One command:

```sh
brew install --cask nvoxland/oxplow/oxplow
```

That clones the [`nvoxland/homebrew-oxplow`](https://github.com/nvoxland/homebrew-oxplow)
tap, downloads the latest signed-ad-hoc DMG, drops quarantine,
and copies the app into `/Applications/`. Subsequent releases
land via `brew upgrade --cask oxplow`.

### macOS: direct DMG download (manual)

If you don't want Homebrew, grab the DMG from a
[release](https://github.com/nvoxland/oxplow/releases). After
copying `Oxplow.app` into `/Applications/`, **you must strip
the quarantine attribute manually** or macOS will refuse to
launch it:

```sh
xattr -dr com.apple.quarantine /Applications/Oxplow.app
```

Then double-click normally. macOS only re-applies the
quarantine attribute on fresh downloads, so the strip is a
one-time step per install.

!!! warning "Why this is necessary"
    The DMG is fine — the OS attaches a `com.apple.quarantine`
    extended attribute to anything downloaded from a browser,
    and Gatekeeper refuses to launch an unsigned binary while
    that attribute is set. Older macOS let you right-click →
    **Open** to bypass it; recent macOS removed that escape
    hatch for unsigned apps. Until oxplow is notarized, every
    direct-DMG install needs the `xattr` step.

### Windows: direct download

Grab the `.msi` or `.exe` from a release and run it. SmartScreen
warns on first launch because the build is unsigned — click
**More info → Run anyway**.

### Linux: direct download

Grab the `.deb` or `.AppImage` from a release. No quarantine
mechanism to work around.

## Option 2: build from source

To build oxplow yourself, follow [`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md)
("Run from source"). It lists the prerequisites and the commands.

## After install

1. Launch oxplow and open the directory you want to work in. The
   app prompts you to pick a project on first launch. To start it from a
   terminal, run **Install `oxplow` command line…** from the app's menu
   once, then `oxplow .` in the directory (it opens `$PWD`).
2. The opened directory **is** the workspace. Oxplow does not
   climb upward looking for an enclosing repo (workspace
   isolation rule).
3. Read [Your first stream](first-stream.md) to send a prompt.

See [Agents](../guide/agents.md) to enable Claude Code, Codex, and OpenCode,
choose the default, and run different agents in separate threads.

Oxplow stores everything project-local under `.oxplow/` inside
the project root: the SQLite database, the wiki pages folder,
per-effort snapshots, the agent runtime files oxplow installs, and
the LSP server cache. Worktrees for non-primary streams live as
siblings of the project root. There is no global state to
configure.
