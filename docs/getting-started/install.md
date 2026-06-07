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

Prerequisites:

- **Bun** ≥ 1.3.9 — package manager + JS runtime.
- **Rust stable** (≥ 1.80) — `rust-toolchain.toml` pins it; rustup
  installs the right toolchain automatically.
- **Platform Tauri deps**:
    - macOS: `xcode-select --install`
    - Linux: `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev`,
      `librsvg2-dev`, `patchelf`, `build-essential`
    - Windows: WebView2 (preinstalled on modern Windows; otherwise
      Microsoft's redistributable) plus the MSVC build tools
- **Git** — oxplow expects the workspace root to be a repo.
- **`tmux`** — agent panes are tmux-managed (the suite skips when
  it's missing, but real use needs it).

If you use [mise](https://mise.jdx.dev), `mise install` picks up
bun / node / rust from `mise.toml`.

```bash
git clone https://github.com/nvoxland/oxplow
cd oxplow
bun install --frozen-lockfile     # frontend deps (React, Monaco, xterm)
bun run tauri:dev                 # builds the Rust shell + boots Vite
```

For everything else — split-process dev, packaged builds, the
release flow — see [`DEV.md`](https://github.com/nvoxland/oxplow/blob/main/DEV.md).

## After install

1. Launch oxplow from the directory you want to work in
   (`./bin/oxplow` from the repo if you built from source — it
   treats the current working directory as the project root).
2. The opened directory **is** the workspace. Oxplow does not
   climb upward looking for an enclosing repo (workspace
   isolation rule).
3. Read [Your first stream](first-stream.md) to send a prompt.

Oxplow stores everything project-local under `.oxplow/` inside
the project root: the SQLite database, the wiki pages folder,
per-effort snapshots, the agent runtime files oxplow installs, and
the LSP server cache. Worktrees for non-primary streams live as
siblings of the project root. There is no global state to
configure.
