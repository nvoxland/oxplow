//! An external provider's filesystem: [`ExternalFileSystem`] is the
//! [`FileSystem`] registered in `Services.filesystems` under
//! `<extension>:<instance id>` while the instance runs
//! (`.context/providers.md` "A filesystem provider",
//! `.context/filesystems.md`). A reader's `stat`, `read` and `list` for a
//! file URI naming it are one invoke each:
//!
//! - `stat { path, rev? }` → `{ entry }`, `entry` null when nothing is
//!   there, else `{ kind: file|dir, size?, modified_at?, is_text? }` (the
//!   editor's `language` is the path's, core's);
//! - `read { path, rev?, offset?, len? }` → `{ bytes }` (base64), null
//!   when there is no such file; a range sends `offset` and `len`;
//! - `list { rev?, parent? }` → `{ entries: [{ path, kind }] }`, `parent`
//!   absent for the whole tree and `""` for the root;
//! - `write { path, content }` → `{}` (base64 `content`), declared only
//!   with the feature `writable`.
//!
//! A path that leaves the filesystem (`..`, absolute) is refused here,
//! before the call. A provider's refusal of an input is
//! [`FsError::Invalid`]; every other failure — its process gone, a
//! timeout, an error of its own — is [`FsError::Unavailable`], which a
//! reader shows and may try again. It emits nothing of its own.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use oxplow_domain::capability::FILESYSTEM;
use oxplow_domain::filesystem::{
    ByteRange, FileSystem, FileSystemRegistry, FsCx, FsEntry, FsError, FsKind, FsStat, WRITABLE,
};
use oxplow_domain::language_id::language_id_for_path;
use oxplow_domain::{CommandError, InputValidator};
use serde::Deserialize;
use serde_json::{json, Value};

use super::registry::{CapabilityHost, Instance, PreparedProviderCapability};

/// The filesystem side of the host: a started instance is a filesystem in
/// `Services.filesystems`, under `<extension>:<instance id>`.
pub struct FileSystemsHost {
    pub registry: Arc<FileSystemRegistry>,
    /// The filesystem id each published instance holds, by instance id:
    /// what [`CapabilityHost::retire`] takes out.
    published: Arc<parking_lot::Mutex<BTreeMap<String, String>>>,
}

impl FileSystemsHost {
    pub const CAPABILITY: &'static str = "filesystem";

    pub fn new(registry: Arc<FileSystemRegistry>) -> Self {
        Self {
            registry,
            published: Arc::default(),
        }
    }
}

/// The id an instance's filesystem has: the `filesystem` param of the file
/// URIs it serves. Refused when it falls in oxplow's own namespace.
fn filesystem_id(instance: &Instance) -> Result<String, String> {
    let extension = &instance.ext.name;
    if extension == "oxplow" {
        return Err("a filesystem id can't be in the `oxplow:` namespace".into());
    }
    Ok(format!("{extension}:{}", instance.id))
}

impl CapabilityHost for FileSystemsHost {
    fn capability(&self) -> &'static str {
        Self::CAPABILITY
    }

    /// Never: a filesystem's id carries its extension's name (`acme:s3`),
    /// which an instance id (`s3`) can't.
    fn has(&self, _id: &str) -> bool {
        false
    }

    fn prepare(&self, instance: &Arc<Instance>) -> Result<PreparedProviderCapability, String> {
        let declared = &instance.declared;
        let capability = declared
            .capabilities
            .iter()
            .find(|c| c.capability == Self::CAPABILITY)
            .ok_or("it declares no `filesystem` capability")?;
        let features = Some(capability.features.clone())
            .filter(|f| !f.is_null())
            .unwrap_or_else(|| json!({}));
        let id = filesystem_id(instance)?;
        let writable = features.get(WRITABLE).and_then(Value::as_bool) == Some(true);
        let mut verbs = BTreeMap::new();
        for verb in FILESYSTEM.verbs {
            if let Some(c) = declared.commands.iter().find(|c| c.name == verb.name) {
                let input = InputValidator::compile(&c.input_schema)
                    .map_err(|e| format!("verb `{}`: {e}", verb.name))?;
                verbs.insert(verb.name, input);
            }
        }
        for required in FILESYSTEM.required(&features) {
            if !verbs.contains_key(required) {
                return Err(format!("it declares no `{required}` verb"));
            }
        }
        let adapter = Arc::new(ExternalFileSystem {
            id: id.clone(),
            instance: instance.clone(),
            features: if writable { &[WRITABLE] } else { &[] },
            verbs,
        });
        let registry = self.registry.clone();
        let published = self.published.clone();
        let instance_id = instance.id.clone();
        Ok(PreparedProviderCapability::new(features, move || {
            published.lock().insert(instance_id, id);
            registry.register(adapter);
        }))
    }

    fn retire(&self, id: &str) {
        if let Some(filesystem) = self.published.lock().remove(id) {
            self.registry.unregister(&filesystem);
        }
    }
}

pub struct ExternalFileSystem {
    id: String,
    instance: Arc<Instance>,
    features: &'static [&'static str],
    /// Each declared contract verb's compiled input schema.
    verbs: BTreeMap<&'static str, InputValidator>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatAnswer {
    entry: Option<Entry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    kind: FsKind,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    modified_at: Option<i64>,
    #[serde(default)]
    is_text: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadAnswer {
    bytes: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListAnswer {
    entries: Vec<Listed>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Listed {
    path: String,
    kind: FsKind,
}

/// `path` as a filesystem keeps it: `/`-separated, none of `..`, `.` or
/// an empty segment (so never absolute). `""` only where `allow_root`.
fn checked(path: &str, allow_root: bool) -> Result<(), FsError> {
    if path.is_empty() && allow_root {
        return Ok(());
    }
    let ok = !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."));
    if ok {
        Ok(())
    } else {
        Err(FsError::Invalid(format!(
            "`{path}` isn't a path inside the filesystem"
        )))
    }
}

impl ExternalFileSystem {
    fn unavailable(&self, why: impl std::fmt::Display) -> FsError {
        FsError::Unavailable {
            filesystem: self.id.clone(),
            why: why.to_string(),
        }
    }

    /// Invoke `verb` with `input`, checked against what it declared, and
    /// read its answer as `T`.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        verb: &'static str,
        input: Value,
    ) -> Result<T, FsError> {
        let schema = self
            .verbs
            .get(verb)
            .ok_or_else(|| self.unavailable(format!("it declares no `{verb}` verb")))?;
        schema
            .check(&input)
            .map_err(|e| self.unavailable(format!("its `{verb}` schema refuses the call: {e}")))?;
        let key = format!("{verb}:{}", uuid::Uuid::new_v4().simple());
        let run = oxplow_domain::run::RunContext::core(
            oxplow_domain::Actor::System,
            oxplow_domain::run::Mode::External,
        );
        let out = self
            .instance
            .invoke(&run, verb, input, Some(key))
            .await
            .map_err(|e| match e {
                CommandError::Invalid { message, .. } => FsError::Invalid(message),
                other => self.unavailable(other),
            })?;
        if !out.events.is_empty() {
            return Err(self.unavailable(format!(
                "`{verb}` returned events, and a filesystem provider emits none"
            )));
        }
        serde_json::from_value(out.result)
            .map_err(|e| self.unavailable(format!("`{verb}` answered something else: {e}")))
    }
}

/// `input` with `rev` when there is one.
fn with_rev(mut input: Value, rev: Option<&str>) -> Value {
    if let Some(rev) = rev {
        input["rev"] = json!(rev);
    }
    input
}

#[async_trait]
impl FileSystem for ExternalFileSystem {
    fn id(&self) -> &str {
        &self.id
    }

    fn features(&self) -> &[&'static str] {
        self.features
    }

    async fn stat(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        path: &str,
    ) -> Result<Option<FsStat>, FsError> {
        checked(path, false)?;
        let answer: StatAnswer = self
            .call("stat", with_rev(json!({ "path": path }), rev))
            .await?;
        Ok(answer.entry.map(|e| {
            let file = e.kind == FsKind::File;
            FsStat {
                kind: e.kind,
                size: e.size.filter(|_| file),
                modified_at: e.modified_at,
                is_text: e.is_text.filter(|_| file),
                language: language_id_for_path(path)
                    .map(str::to_string)
                    .filter(|_| file),
            }
        }))
    }

    async fn read(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        path: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<Vec<u8>>, FsError> {
        checked(path, false)?;
        let mut input = json!({ "path": path });
        if let Some(range) = range {
            input["offset"] = json!(range.offset);
            input["len"] = json!(range.len);
        }
        let answer: ReadAnswer = self.call("read", with_rev(input, rev)).await?;
        let Some(bytes) = answer.bytes else {
            return Ok(None);
        };
        let mut bytes = base64::engine::general_purpose::STANDARD
            .decode(bytes)
            .map_err(|e| self.unavailable(format!("`read` of `{path}`: {e}")))?;
        // Never more than the range asked for, whatever it sent.
        if let Some(range) = range {
            bytes.truncate(usize::try_from(range.len).unwrap_or(usize::MAX));
        }
        Ok(Some(bytes))
    }

    async fn list(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        parent: Option<&str>,
    ) -> Result<Vec<FsEntry>, FsError> {
        let mut input = json!({});
        if let Some(parent) = parent {
            checked(parent, true)?;
            input["parent"] = json!(parent);
        }
        let answer: ListAnswer = self.call("list", with_rev(input, rev)).await?;
        let mut entries = Vec::with_capacity(answer.entries.len());
        for e in answer.entries {
            checked(&e.path, false)
                .map_err(|_| self.unavailable(format!("`list` named `{}`", e.path)))?;
            entries.push(FsEntry {
                path: e.path,
                kind: e.kind,
                status: None,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn write(&self, _cx: &FsCx, path: &str, content: &[u8]) -> Result<(), FsError> {
        if !self.has_feature(WRITABLE) {
            return Err(FsError::ReadOnly(self.id.clone()));
        }
        checked(path, false)?;
        let content = base64::engine::general_purpose::STANDARD.encode(content);
        let _: Value = self
            .call("write", json!({ "path": path, "content": content }))
            .await?;
        Ok(())
    }
}
