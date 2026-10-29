# Filesystems — where a file's bytes come from

A **filesystem** is a capability with many implementations active at
once: each answers `stat`, `read`, `list` and, when it is `writable`,
`write`, and a reader names the one it means by its id. Core ships
`oxplow:local` (the stream's worktree). `oxplow:extensions` (the running
copies of extensions, `rev` = deployment hash) and `oxplow:temp` (scratch
files) are the next built-ins, and a provider can add its own later
(`<extension>:<name>`, `acme:s3`).

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
← `writable`). `oxplow:local` is a core row (`source: core`, shown in
`v_capability_provider` as provider `local`, `writable`) — it doesn't
depend on any extension being enabled. A provider process cannot yet
implement it: the host (`FileSystemsHost`, registering an
`ExternalFileSystem`) comes with external filesystems
([providers.md](./providers.md)).

## Tests

`crates/oxplow-app/src/filesystem_conformance.rs` is the contract as plain
async functions over `&dyn FileSystem` and a `rev`: stat matches read,
absent is `None`, reads round-trip and ranges slice, listings are one
level or the whole tree, a path never leaves the filesystem, only a
`writable` filesystem writes. It runs against `oxplow:local` at the
working tree, `git:HEAD` and a `snap:<id>`. A new filesystem adds a test
that runs the same functions. The editor language table is
`oxplow_domain::language_id` (the Monaco id space); a test holds it equal
to the desktop's `editor-language.ts`.
