//! The filesystem capability's contract (`.context/filesystems.md`), as
//! checks any filesystem must pass. Each check takes a `&dyn FileSystem`,
//! the call's [`FsCx`] and the `rev` to read — a filesystem's revision is
//! its own word, so the caller says which one holds [`fixture`] — and the
//! tests at the bottom run the list against `oxplow:local` at the working
//! tree, `git:HEAD` and a `snap:<id>`. Test code: the module compiles
//! under `cfg(test)` only.
//!
//! 1. **Stat** — a file's size is the length of what `read` returns, a
//!    folder has none, a binary file is not text, and a path that isn't
//!    there is `None`.
//! 2. **Read** — a file reads back what was put there; a range is a slice
//!    of it, short or empty past the end, never an error.
//! 3. **List** — one folder lists one level; the whole tree lists
//!    everything, each file's folders included; a folder lists exactly
//!    the rows of the whole walk that have it as parent; a folder that
//!    isn't there lists nothing.
//! 4. **Paths** — a path that leaves the filesystem (`..`, absolute) is
//!    refused as invalid by every call (a write too, unless the filesystem
//!    is read-only, which refuses every write as such).
//! 5. **Write** — only a `writable` filesystem accepts one; any other
//!    refuses with `ReadOnly`.

use std::collections::BTreeSet;

use oxplow_domain::filesystem::{ByteRange, FileSystem, FsCx, FsEntry, FsError, FsKind};

/// The folder the fixture's files live under.
pub const ROOT: &str = "conformance-files";

/// The files every revision under test holds: path (under [`ROOT`]) and
/// content.
pub fn fixture() -> Vec<(String, Vec<u8>)> {
    [
        ("a.txt", &b"one\ntwo\nthree\n"[..]),
        ("src/main.rs", b"fn main() {}\n"),
        ("src/deep/x.txt", b"x\n"),
        ("bin.dat", &[0, 1, 2, 255, 0]),
    ]
    .into_iter()
    .map(|(p, c)| (format!("{ROOT}/{p}"), c.to_vec()))
    .collect()
}

fn at(path: &str) -> String {
    format!("{ROOT}/{path}")
}

fn paths(entries: &[FsEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.path.as_str()).collect()
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// 1. Stat agrees with read; absent is `None`.
pub async fn stat_describes_what_read_returns(fs: &dyn FileSystem, cx: &FsCx, rev: Option<&str>) {
    for (path, content) in fixture() {
        let stat = fs.stat(cx, rev, &path).await.unwrap().expect(&path);
        assert_eq!(stat.kind, FsKind::File, "{path}");
        assert_eq!(stat.size, Some(content.len() as u64), "{path}");
        let read = fs.read(cx, rev, &path, None).await.unwrap().expect(&path);
        assert_eq!(read.len() as u64, stat.size.unwrap(), "{path}");
        assert_eq!(stat.is_text, Some(!content.contains(&0)), "{path}");
    }
    let rust = fs.stat(cx, rev, &at("src/main.rs")).await.unwrap().unwrap();
    assert_eq!(rust.language.as_deref(), Some("rust"));
    let plain = fs.stat(cx, rev, &at("bin.dat")).await.unwrap().unwrap();
    assert_eq!(plain.language, None);
    let dir = fs
        .stat(cx, rev, &at("src"))
        .await
        .unwrap()
        .expect("a folder");
    assert_eq!(
        (dir.kind, dir.size, dir.is_text, dir.language),
        (FsKind::Dir, None, None, None)
    );
    assert_eq!(fs.stat(cx, rev, &at("nope.txt")).await.unwrap(), None);
    assert_eq!(
        fs.stat(cx, rev, &at("nope/deeper.txt")).await.unwrap(),
        None
    );
}

/// 2. Read round-trips; a range is a slice.
pub async fn read_round_trips_and_ranges_slice(fs: &dyn FileSystem, cx: &FsCx, rev: Option<&str>) {
    for (path, content) in fixture() {
        let read = fs.read(cx, rev, &path, None).await.unwrap();
        assert_eq!(read.as_deref(), Some(&content[..]), "{path}");
    }
    assert_eq!(fs.read(cx, rev, &at("nope.txt"), None).await.unwrap(), None);
    assert_eq!(
        fs.read(cx, rev, &at("src"), None).await.unwrap(),
        None,
        "a folder has no bytes"
    );
    let path = at("a.txt");
    let whole = b"one\ntwo\nthree\n";
    let range = |offset, len| Some(ByteRange { offset, len });
    let read = |range| fs.read(cx, rev, &path, range);
    assert_eq!(read(range(4, 3)).await.unwrap().unwrap(), &whole[4..7]);
    assert_eq!(read(range(0, 3)).await.unwrap().unwrap(), &whole[..3]);
    assert_eq!(
        read(range(10, 100)).await.unwrap().unwrap(),
        &whole[10..],
        "past the end is short"
    );
    assert!(
        read(range(100, 5)).await.unwrap().unwrap().is_empty(),
        "beyond the end is empty, not an error"
    );
    assert_eq!(read(range(0, u64::MAX)).await.unwrap().unwrap(), whole);
    assert_eq!(
        fs.read(cx, rev, &at("nope.txt"), range(0, 3))
            .await
            .unwrap(),
        None
    );
}

/// 3. Listings.
pub async fn lists_one_level_or_the_whole_tree(fs: &dyn FileSystem, cx: &FsCx, rev: Option<&str>) {
    let kind_of =
        |entries: &[FsEntry], path: &str| entries.iter().find(|e| e.path == path).map(|e| e.kind);

    let level = fs.list(cx, rev, Some(ROOT)).await.unwrap();
    assert_eq!(
        paths(&level),
        [at("a.txt"), at("bin.dat"), at("src")],
        "one level, sorted"
    );
    assert_eq!(kind_of(&level, &at("src")), Some(FsKind::Dir));
    assert_eq!(kind_of(&level, &at("a.txt")), Some(FsKind::File));
    let src = fs.list(cx, rev, Some(&at("src"))).await.unwrap();
    assert_eq!(paths(&src), [at("src/deep"), at("src/main.rs")]);

    let root = fs.list(cx, rev, Some("")).await.unwrap();
    assert_eq!(
        kind_of(&root, ROOT),
        Some(FsKind::Dir),
        "the root lists the fixture's folder"
    );
    assert!(
        root.iter().all(|e| parent_of(&e.path).is_empty()),
        "{:?}",
        paths(&root)
    );

    let tree = fs.list(cx, rev, None).await.unwrap();
    let all: BTreeSet<&str> = paths(&tree).into_iter().collect();
    for (path, _) in fixture() {
        assert_eq!(kind_of(&tree, &path), Some(FsKind::File), "{path}");
    }
    for dir in [ROOT.to_string(), at("src"), at("src/deep")] {
        assert_eq!(kind_of(&tree, &dir), Some(FsKind::Dir), "{dir}");
    }
    assert_eq!(all.len(), tree.len(), "no path twice");
    let mut sorted = paths(&tree);
    sorted.sort_unstable();
    assert_eq!(sorted, paths(&tree), "sorted by path");
    // A folder's listing is the whole walk's rows with that parent.
    for parent in [ROOT.to_string(), at("src"), at("src/deep")] {
        let of_walk: Vec<&FsEntry> = tree
            .iter()
            .filter(|e| parent_of(&e.path) == parent)
            .collect();
        let listed = fs.list(cx, rev, Some(&parent)).await.unwrap();
        assert_eq!(
            paths(&listed),
            of_walk.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            "{parent}"
        );
    }

    assert!(
        fs.list(cx, rev, Some(&at("nope")))
            .await
            .unwrap()
            .is_empty(),
        "a folder that isn't there lists nothing"
    );
    assert!(
        fs.list(cx, rev, Some(&at("a.txt")))
            .await
            .unwrap()
            .is_empty(),
        "a file has no children"
    );
}

/// 4. Nothing leaves the filesystem.
pub async fn a_path_never_leaves_the_filesystem(fs: &dyn FileSystem, cx: &FsCx, rev: Option<&str>) {
    for path in ["../x", "a/../../x", "/etc/passwd"] {
        let invalid = |e: FsError| matches!(e, FsError::Invalid(_));
        assert!(
            fs.stat(cx, rev, path).await.err().is_some_and(invalid),
            "stat {path}"
        );
        assert!(
            fs.read(cx, rev, path, None)
                .await
                .err()
                .is_some_and(invalid),
            "read {path}"
        );
        assert!(
            fs.read(cx, rev, path, Some(ByteRange { offset: 0, len: 1 }))
                .await
                .err()
                .is_some_and(invalid),
            "ranged read {path}"
        );
        assert!(
            fs.list(cx, rev, Some(path))
                .await
                .err()
                .is_some_and(invalid),
            "list {path}"
        );
        // A read-only filesystem refuses every write the same way.
        let write = fs.write(cx, path, b"x").await.err();
        if fs.has_feature("writable") {
            assert!(write.is_some_and(invalid), "write {path}");
        } else {
            assert_eq!(
                write,
                Some(FsError::ReadOnly(fs.id().to_string())),
                "write {path}"
            );
        }
    }
}

/// 5. A filesystem writes when it says it can, and not otherwise. The
///    working tree takes the write; `rev` is the one that reads it back.
pub async fn writes_only_when_writable(fs: &dyn FileSystem, cx: &FsCx, rev: Option<&str>) {
    let path = at("written.txt");
    if fs.has_feature("writable") {
        fs.write(cx, &path, b"hello\n").await.unwrap();
        let read = fs.read(cx, rev, &path, None).await.unwrap();
        assert_eq!(read.as_deref(), Some(&b"hello\n"[..]));
        fs.write(cx, &path, b"again\n").await.unwrap();
        let read = fs.read(cx, rev, &path, None).await.unwrap();
        assert_eq!(read.as_deref(), Some(&b"again\n"[..]), "a write replaces");
    } else {
        assert_eq!(
            fs.write(cx, &path, b"hello\n").await,
            Err(FsError::ReadOnly(fs.id().to_string()))
        );
        assert_eq!(fs.read(cx, rev, &path, None).await.unwrap(), None);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use oxplow_db::FileSnapshot;
    use oxplow_domain::filesystem::{FsCx, LOCAL};
    use oxplow_domain::stores::StreamStore as _;

    use super::*;
    use crate::test_fixtures::{commit_all, services_with_effort, EffortFixture};

    fn write_fixture(ws: &std::path::Path) {
        for (path, content) in fixture() {
            let file = ws.join(&path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, content).unwrap();
        }
    }

    /// A snapshot of the fixture, with its bytes in the blob store.
    async fn snapshot_of_fixture(f: &EffortFixture) -> i64 {
        let svc = &f.svc;
        let stream = svc.stream_store.list().await.unwrap()[0].id;
        let id = svc.snapshot_store.create_snapshot(stream).await.unwrap();
        for (path, content) in fixture() {
            svc.snapshot_store
                .capture(FileSnapshot {
                    id: 0,
                    stream_id: stream,
                    path,
                    blob_hash: Some(svc.blobs.write(&content).unwrap()),
                    size_bytes: content.len() as i64,
                    captured_at: oxplow_domain::Timestamp::now(),
                    storage: oxplow_db::SnapshotStorage::Oxplow,
                    snapshot_id: Some(id),
                    mtime_ms: None,
                    content_hash: None,
                })
                .await
                .unwrap();
        }
        id
    }

    async fn run_all(fs: &dyn FileSystem, cx: &FsCx, rev: Option<&str>) {
        stat_describes_what_read_returns(fs, cx, rev).await;
        read_round_trips_and_ranges_slice(fs, cx, rev).await;
        lists_one_level_or_the_whole_tree(fs, cx, rev).await;
        a_path_never_leaves_the_filesystem(fs, cx, rev).await;
    }

    async fn local(f: &EffortFixture) -> (Arc<dyn FileSystem>, FsCx) {
        let stream = f.svc.stream_store.list().await.unwrap()[0].id;
        (
            f.svc.filesystems.resolve(Some(LOCAL)).unwrap(),
            FsCx {
                stream: Some(stream),
            },
        )
    }

    #[tokio::test]
    async fn local_passes_on_the_working_tree() {
        let f = services_with_effort().await;
        write_fixture(&f.svc.layout.project_dir);
        let (fs, cx) = local(&f).await;
        run_all(&*fs, &cx, None).await;
        run_all(&*fs, &cx, Some("working")).await;
        writes_only_when_writable(&*fs, &cx, None).await;
    }

    #[tokio::test]
    async fn local_passes_at_a_git_revision() {
        let f = services_with_effort().await;
        let ws = f.svc.layout.project_dir.clone();
        write_fixture(&ws);
        commit_all(&ws, "the fixture");
        // The disk moves on; the revision doesn't.
        std::fs::write(ws.join(at("a.txt")), "changed\n").unwrap();
        let (fs, cx) = local(&f).await;
        run_all(&*fs, &cx, Some("git:HEAD")).await;
    }

    #[tokio::test]
    async fn local_passes_at_a_snapshot() {
        let f = services_with_effort().await;
        let snap = snapshot_of_fixture(&f).await;
        let (fs, cx) = local(&f).await;
        run_all(&*fs, &cx, Some(&format!("snap:{snap}"))).await;
    }

    /// A revision is the local filesystem's word and it refuses others.
    #[tokio::test]
    async fn local_refuses_a_revision_it_does_not_know() {
        let f = services_with_effort().await;
        let (fs, cx) = local(&f).await;
        for rev in ["HEAD", "snap:x", "hg:"] {
            let e = fs
                .read(&cx, Some(rev), &at("a.txt"), None)
                .await
                .unwrap_err();
            assert!(matches!(e, FsError::Invalid(_)), "{rev}: {e:?}");
        }
    }

    /// A filesystem that doesn't say `writable` refuses writes, whatever
    /// else it does: the default `write` is the refusal.
    #[tokio::test]
    async fn a_read_only_filesystem_refuses_writes() {
        struct ReadOnly;
        #[async_trait::async_trait]
        impl FileSystem for ReadOnly {
            fn id(&self) -> &str {
                "acme:ro"
            }
            fn features(&self) -> &[&'static str] {
                &[]
            }
            async fn stat(
                &self,
                _: &FsCx,
                _: Option<&str>,
                _: &str,
            ) -> Result<Option<oxplow_domain::filesystem::FsStat>, FsError> {
                Ok(None)
            }
            async fn read(
                &self,
                _: &FsCx,
                _: Option<&str>,
                _: &str,
                _: Option<ByteRange>,
            ) -> Result<Option<Vec<u8>>, FsError> {
                Ok(None)
            }
            async fn list(
                &self,
                _: &FsCx,
                _: Option<&str>,
                _: Option<&str>,
            ) -> Result<Vec<FsEntry>, FsError> {
                Ok(Vec::new())
            }
        }
        writes_only_when_writable(&ReadOnly, &FsCx::default(), None).await;
    }
}
