# Filesystems — where a file's bytes come from

A **filesystem** is a capability with many implementations active at
once: each answers `stat`, `read`, `list` and, when it is `writable`,
`write`, and a reader names the one it means by its id. Core ships
`oxplow:local` (the stream's worktree), `oxplow:extensions` (the running
copies of extensions, `rev` = deployment hash) and `oxplow:temp` (scratch
files); a provider can add its own later (`<extension>:<name>`, `acme:s3`).

## The trait (`oxplow_domain::filesystem`)

```rust
trait FileSystem {
    fn id(&self) -> &str;                    // "oxplow:local", "acme:s3"
    fn features(&self) -> &[&'static str];   // [] or ["writable"]
    async fn stat(&self, cx: &FsCx, rev: Option<&str>, path: &str) -> Result<Option<FsStat>, FsError>;
    async fn read(&self, cx: &FsCx, rev: Option<&str>, path: &str, range: Option<ByteRange>) -> Result<Option<Vec<u8>>, FsError>;
    async fn list(&self, cx: &FsCx, rev: Option<&str>, parent: Option<&str>) -> Result<Vec<FsEntry>, FsError>;
    async fn write(&self, cx: &FsCx, path: &str, content: &[u8]) -> Result<(), FsError>; // default: ReadOnly
}
```

- `FsCx { stream }` is what a call is made for: the local filesystem
  routes the stream to its worktree (the primary checkout for none),
  as every read of a stream's files has.
- `rev` is **opaque outside a filesystem**: each one has its own word for
  it, and `None` is its default. Never parse one anywhere but in the
  filesystem it belongs to.
- A path is relative to the filesystem's root and never leaves it:
  `..` and absolute paths are `FsError::Invalid`. A path that isn't there
  is `None`, never an error; a folder that isn't there lists nothing.
- `ByteRange { offset, len }` reads a slice; past the end it is short or
  empty, never an error.
- `list(Some(parent))` is one level (the root is `""`); `list(None)` is
  the whole tree, folders included. A folder's listing is exactly the
  whole walk's rows with that parent.
- `FsStat { kind, size, modified_at, is_text, language }`: `is_text` is a
  NUL sniff of the first 8 KB, made in `stat` only (a reader that needs to
  know whether to show a file asks `stat` before `read`); `language` is
  the editor's language id by extension.
- `FsEntry { path, kind, status }`: `status` is the VCS change against the
  head, for a filesystem that has one. Listings leave `size` and
  `modified_at` out (they would cost a stat per row); `stat` has them.
- `write` is `ReadOnly` unless the filesystem declares `writable`. Today
  only text is written.

`FileSystemRegistry` (`Services.filesystems`) holds every filesystem by
id. `resolve(None)` is `oxplow:local`; an id that isn't registered is
refused naming the ones that are. There is no "active" one and nothing
is chosen: every registered filesystem serves.

## Built-ins

### `oxplow:local` (`crates/oxplow-app/src/filesystems/local.rs`)

`LocalFs` is `Trees` (read), `WorkspaceFiles` (the working tree's
listing, `write`) and the `WorktreeRouter`. Its `rev` is
`oxplow_domain::vcs::Revision`'s vocabulary and stays private to it:
absent or `working` is the files on disk, `snap:<id>` a local-history
snapshot, `git:<rev>` a VCS revision. `Trees` is not rewritten: it stays
the engine for diffs, the code-quality corpus and change analysis; only a
file's content, kind and listing go through the filesystem.

- The working tree is read from disk; a ranged read seeks, so a range of
  a big file never reads the rest. Another revision reads the whole file
  through `Trees::read_at` and slices it. `stat` there is a file's bytes
  read, or a folder when files lie under the path.
- The working tree's listing honours the workspace filter
  (`generated:`), as the Files page always has. A past revision lists its
  filtered `files_at` with the folders derived from the paths.
- `write` goes through `WorkspaceFiles::write` (announces
  `WorkspaceChanged`) and needs a stream: it never falls back to the
  primary checkout.

### `oxplow:extensions` (`filesystems/extensions.rs`)

`ExtensionsFs` serves the copies of extensions oxplow runs. A path is
`<extension name>/<path inside it>`; `rev` is a deployment hash and none
means the extension's **running** deployment. Read-only.

- Files are the deployment folder, `extension_store::deployment_dir(project,
  name, hash)`. The running hash is the catalog's: `running_hash` (=
  `Deployment::hash`, the same value `locate` writes to
  `PageProvenance.deployment`), for a deployed or shipped copy.
- An extension that runs no deployment — read from its source folder (an
  authoring check, the CLI, a dev build's own extensions under
  `enableBundledExtensions: false`), undeployed, or not here — has **no
  running files**: with no `rev` its paths are `None`/empty and it isn't
  in the root listing, exactly as `v_page.running_uri` is NULL for it. Name
  one of its stored deployments by hash to read that. The source folder is
  not this filesystem's business; it is `oxplow:local`.
- A `rev` is one folder name (letters and digits); anything else is
  `Invalid`. An unknown hash or extension is `None`.
- Listing: `""` lists the extensions (those that run a deployment; with a
  `rev`, those whose store folder holds it), `<name>` the deployment's
  root, and so on one level; `None` walks everything. Symlinks are not
  listed.
- Escapes: the path is checked lexically (no `..`, not absolute), then the
  target is canonicalized and must lie inside the canonical deployment
  folder, so a symlink out is `Invalid`, never the file it points at.

### `oxplow:temp` (`filesystems/temp.rs`)

`TempFs` is scratch space oxplow's own code writes: a flat folder,
`<project>/.oxplow/runtime/temp/<name>`, so a restart keeps what a window
shows. A name is `^[A-Za-z0-9_-]+\.[A-Za-z0-9]+$` (no folders; anything
else is `Invalid`), and there are **no revisions** (a `rev` is `Invalid`).
It is not `writable`, so `oxplow.file.save` never touches it; the code
that owns a file writes with `Services.temp.put(name, bytes)` / `remove`
(the same `Arc<TempFs>` the registry holds). A file larger than the byte
cap is refused.

- GC (`TempFs::gc`): a file idle longer than 7 days (its modified time)
  goes, then the oldest (by modified time, then name) until at most 200
  files and 32 MB stay. It runs after every `put` and on a five-minute
  tick (`Services::spawn_every` in `boot.rs`).
- A read refreshes a file's modified time when it is more than a day old,
  so a file someone keeps opening stays; a younger one is left alone, so
  `stat`'s `modified_at` (a cache key) moves at most once a day by reading.

## Using one

- RPC `read_at { streamId, path, filesystem?, rev? }` and `stat_file`
  (same inputs, returns `FsStat | null`); TS `readAt(stream, path, { filesystem?, rev? })`
  and `statFile` in `apps/desktop/src/api.ts`.
- MCP `read` of a file URI (`ref_read::file_text`): resolves the URI's
  `filesystem`, `stat`s it, answers "binary file, N bytes" for a binary
  file instead of lossy UTF-8, and reads a ranged first 64 KiB + 1 to set
  `truncated` (a `#lines=` view reads the whole file, since the lines may
  lie anywhere).
- `apps/desktop/src/filesystem.ts`: `LOCAL_FS`, and `fsRevLabel(filesystem,
  rev)` for a tab or header (a local revision as `shortRevisionLabel`, another
  filesystem's hash shortened).

## In a URI

`oxplow:oxplow/file?path=…[&rev=…][&filesystem=<id>]`, and the same for
`oxplow/dir`. Absent `filesystem` is `oxplow:local`, so every URI written
before filesystems keeps its identity; the builders leave an explicit
`oxplow:local` out, so one file has one identity. See
[resources.md](./resources.md).

## The capability

`filesystem` is in `oxplow_domain::capability::CAPABILITIES`: `many`
(every implementation is active, none chosen), feature `writable`, and a
`ProviderContract` (`FILESYSTEM`: verbs `stat`, `read`, `list`; `write`
← `writable`). `oxplow:local`, `oxplow:extensions` and `oxplow:temp` are core rows (`source: core`, shown in
`v_capability_provider` as providers `local` (`writable`), `extensions`
and `temp`) — they don't depend on any extension being enabled. A provider process cannot yet
implement it: the host (`FileSystemsHost`, registering an
`ExternalFileSystem`) comes with external filesystems
([providers.md](./providers.md)).

## Tests

`crates/oxplow-app/src/filesystem_conformance.rs` is the contract as plain
async functions over `&dyn FileSystem` and a `rev`: stat matches read,
absent is `None`, reads round-trip and ranges slice, listings are one
level or the whole tree, a path never leaves the filesystem, only a
`writable` filesystem writes. It runs against `oxplow:local` at the
working tree, `git:HEAD` and a `snap:<id>`, and against `oxplow:extensions`
(a deployment holding the fixture, with no `rev` and by hash). `oxplow:temp`
is flat, so it runs only the path check and asserts its own rules. A new
filesystem adds a test that runs the same functions. The editor language table is
`oxplow_domain::language_id` (the Monaco id space); a test holds it equal
to the desktop's `editor-language.ts`.

## Listing as a dataset: `v_file`

`v_file` (a live dataset, `live_datasets.rs`) lists any filesystem through
the registry's `list`: columns `filesystem, stream_id, rev, path, parent,
name, kind, size, modified_at, uri, status`. It pushes down `filesystem`,
`rev` and `parent`. A query that names no `filesystem` lists `oxplow:local`
only; `rev` is the filesystem's own word (an empty one is its default), and
the row's `rev` is what the query asked. `parent = ''` is the root; a
one-level listing fills `size` and `modified_at` with `stat` for the files
(not the folders), the whole tree leaves them NULL. `uri` comes from
`file_uri_at` / `dir_uri_at`, so it names the filesystem and revision
whenever they aren't the local default. An unregistered `filesystem`
refuses, naming the ones that are. Pages that draw a folder (Directory) take
`filesystem` and `rev` params and link by the row's `uri`.
