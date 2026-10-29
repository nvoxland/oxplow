//! The filesystem capability's interface (`.context/filesystems.md`):
//! where a file's bytes come from. A [`FileSystem`] answers `stat`,
//! `read` (a whole file or a byte range), `list` and, when it declares
//! `writable`, `write`; the [`FileSystemRegistry`] holds every one active
//! at once, by id (`oxplow:local`, `acme:s3`), and a reader names its
//! filesystem by that id — absent means [`LOCAL`].
//!
//! `rev` is opaque outside a filesystem: each one has its own vocabulary
//! for it (the local one's `working`, `git:<rev>`, `snap:<id>`; a
//! deployment hash), and `None` means the filesystem's default.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::ids::StreamId;
use crate::vcs::FileStatus;
use crate::DomainError;

/// The id of the stream's worktree as files: what a file URI names when it
/// names no filesystem.
pub const LOCAL: &str = "oxplow:local";

/// The running copies of extensions (`<name>/<path>`, `rev` = deployment hash).
pub const EXTENSIONS: &str = "oxplow:extensions";

/// Scratch files oxplow keeps for a while (`<name>.<ext>`, no revisions).
pub const TEMP: &str = "oxplow:temp";

/// The feature of a filesystem that accepts [`FileSystem::write`].
pub const WRITABLE: &str = "writable";

/// What a call is made for: the stream whose files a filesystem that has
/// streams (the local one) answers from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FsCx {
    pub stream: Option<StreamId>,
}

/// A half-open byte range of a file: `len` bytes from `offset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteRange {
    pub offset: u64,
    pub len: u64,
}

impl ByteRange {
    /// The slice of `bytes` the range names; short, or empty, past the end.
    pub fn slice(self, bytes: &[u8]) -> Vec<u8> {
        let len = bytes.len() as u64;
        let from = self.offset.min(len);
        let to = self.offset.saturating_add(self.len).min(len);
        bytes[from as usize..to as usize].to_vec()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FsKind {
    File,
    Dir,
}

impl FsKind {
    /// Its wire name (`file`, `dir`).
    pub fn as_str(self) -> &'static str {
        match self {
            FsKind::File => "file",
            FsKind::Dir => "dir",
        }
    }
}

/// What a filesystem knows of one path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FsStat {
    pub kind: FsKind,
    /// A file's byte length; none on a folder.
    #[specta(type = Option<crate::JsNumber>)]
    pub size: Option<u64>,
    /// Milliseconds since the epoch; none when the filesystem keeps no
    /// times (a past revision).
    #[specta(type = Option<crate::JsNumber>)]
    pub modified_at: Option<i64>,
    /// Whether a file's first 8 KB hold no NUL byte; none on a folder.
    pub is_text: Option<bool>,
    /// The editor's language id by extension (`rust`, `yaml`); none when
    /// it isn't one it knows, and on a folder.
    pub language: Option<String>,
}

/// One path in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct FsEntry {
    pub path: String,
    pub kind: FsKind,
    /// A file's change against the head, for a filesystem that has a VCS.
    pub status: Option<FileStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FsError {
    /// The path, revision or filesystem id isn't one: a path that leaves
    /// the filesystem (`..`, absolute), a revision it can't read.
    #[error("{0}")]
    Invalid(String),
    /// The filesystem doesn't accept writes.
    #[error("the `{0}` filesystem is read-only")]
    ReadOnly(String),
    /// The filesystem can't answer now (its process is gone).
    #[error("the `{filesystem}` filesystem is unavailable: {why}")]
    Unavailable { filesystem: String, why: String },
    #[error("{0}")]
    Storage(String),
}

impl From<FsError> for DomainError {
    fn from(e: FsError) -> Self {
        match e {
            FsError::Invalid(m) => DomainError::Invalid(m),
            FsError::ReadOnly(_) => DomainError::Invalid(e.to_string()),
            FsError::Unavailable { .. } | FsError::Storage(_) => {
                DomainError::Storage(e.to_string())
            }
        }
    }
}

impl From<DomainError> for FsError {
    fn from(e: DomainError) -> Self {
        match e {
            DomainError::Invalid(m) => FsError::Invalid(m),
            other => FsError::Storage(other.to_string()),
        }
    }
}

/// One filesystem, registered under its id. A path is relative to the
/// filesystem's root and never leaves it; an absent path is `None` (or an
/// empty listing), never an error.
#[async_trait]
pub trait FileSystem: Send + Sync {
    /// `oxplow:local`, `acme:s3`: `<extension>:<name>`.
    fn id(&self) -> &str;
    /// What it can do beyond reading: `[]` or `["writable"]`.
    fn features(&self) -> &[&'static str];
    /// Whether it declares `feature`.
    fn has_feature(&self, feature: &str) -> bool {
        self.features().contains(&feature)
    }
    /// What `path` is at `rev`; `None` when it isn't there.
    async fn stat(
        &self,
        cx: &FsCx,
        rev: Option<&str>,
        path: &str,
    ) -> Result<Option<FsStat>, FsError>;
    /// The bytes of the file `path` at `rev` — all of them, or the part
    /// `range` names (past the end it is short or empty). `None` when
    /// there is no such file.
    async fn read(
        &self,
        cx: &FsCx,
        rev: Option<&str>,
        path: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<Vec<u8>>, FsError>;
    /// The files and folders directly in `parent` (the root is `""`), or
    /// with `None` everything below the root. Sorted by path. A folder
    /// that isn't there has none.
    async fn list(
        &self,
        cx: &FsCx,
        rev: Option<&str>,
        parent: Option<&str>,
    ) -> Result<Vec<FsEntry>, FsError>;
    /// Replace the file `path` with `content`.
    async fn write(&self, _cx: &FsCx, _path: &str, _content: &[u8]) -> Result<(), FsError> {
        Err(FsError::ReadOnly(self.id().to_string()))
    }
}

/// The filesystems, by id. All of them are active at once; none is
/// chosen.
#[derive(Default)]
pub struct FileSystemRegistry {
    filesystems: RwLock<BTreeMap<String, Arc<dyn FileSystem>>>,
}

impl FileSystemRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a filesystem under its id, replacing one of that id.
    pub fn register(&self, fs: Arc<dyn FileSystem>) {
        self.filesystems
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(fs.id().to_string(), fs);
    }

    pub fn unregister(&self, id: &str) {
        self.filesystems
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn FileSystem>> {
        self.filesystems
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    pub fn ids(&self) -> Vec<String> {
        self.filesystems
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// The filesystem a reader names; none named is [`LOCAL`]. An id that
    /// isn't registered is refused, naming the ones that are.
    pub fn resolve(&self, id: Option<&str>) -> Result<Arc<dyn FileSystem>, FsError> {
        let id = id.unwrap_or(LOCAL);
        self.get(id).ok_or_else(|| {
            FsError::Invalid(format!(
                "no filesystem `{id}`; the filesystems are: {}",
                self.ids().join(", ")
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Named(&'static str);

    #[async_trait]
    impl FileSystem for Named {
        fn id(&self) -> &str {
            self.0
        }
        fn features(&self) -> &[&'static str] {
            &[]
        }
        async fn stat(
            &self,
            _: &FsCx,
            _: Option<&str>,
            _: &str,
        ) -> Result<Option<FsStat>, FsError> {
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

    #[test]
    fn the_registry_holds_every_filesystem_by_id() {
        let r = FileSystemRegistry::new();
        r.register(Arc::new(Named(LOCAL)));
        r.register(Arc::new(Named("acme:s3")));
        assert_eq!(r.ids(), ["acme:s3", "oxplow:local"]);
        assert!(r.has("acme:s3") && !r.has("acme:gcs"));
        r.unregister("acme:s3");
        assert_eq!(r.ids(), ["oxplow:local"]);
    }

    #[test]
    fn none_named_resolves_to_the_local_filesystem() {
        let r = FileSystemRegistry::new();
        r.register(Arc::new(Named(LOCAL)));
        assert_eq!(r.resolve(None).unwrap().id(), LOCAL);
        assert_eq!(r.resolve(Some(LOCAL)).unwrap().id(), LOCAL);
    }

    #[test]
    fn an_unknown_id_is_refused_naming_the_ones_there_are() {
        let r = FileSystemRegistry::new();
        r.register(Arc::new(Named(LOCAL)));
        r.register(Arc::new(Named("acme:s3")));
        let e = r.resolve(Some("acme:gcs")).err().expect("refused");
        assert_eq!(
            e,
            FsError::Invalid(
                "no filesystem `acme:gcs`; the filesystems are: acme:s3, oxplow:local".into()
            )
        );
    }

    #[tokio::test]
    async fn a_filesystem_is_read_only_unless_it_writes() {
        let fs = Named("acme:s3");
        assert!(!fs.has_feature(WRITABLE));
        assert_eq!(
            fs.write(&FsCx::default(), "a", b"x").await,
            Err(FsError::ReadOnly("acme:s3".into()))
        );
    }

    #[test]
    fn a_byte_range_is_a_slice_short_past_the_end() {
        let r = |offset, len| ByteRange { offset, len };
        assert_eq!(r(1, 2).slice(b"abcd"), b"bc");
        assert_eq!(r(3, 10).slice(b"abcd"), b"d");
        assert!(r(9, 2).slice(b"abcd").is_empty());
        assert_eq!(r(0, u64::MAX).slice(b"abcd"), b"abcd");
        assert_eq!(r(u64::MAX, u64::MAX).slice(b"abcd"), b"");
    }
}
