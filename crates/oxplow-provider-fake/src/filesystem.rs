//! The fake as a **filesystem provider** (`OXPLOW_FAKE_CAPABILITY=filesystem`):
//! an in-memory tree of files. It starts holding the files
//! `OXPLOW_FAKE_FILES` names (a JSON object, path → base64 content) and,
//! with the feature `writable` (`OXPLOW_FAKE_FEATURES=writable`), takes
//! writes. It keeps one revision, `v1`; a `rev` that names another is
//! refused.
//!
//! - `stat { path, rev? }` → `{ entry }`, `entry` null when nothing is
//!   there, else `{ kind, size?, modified_at?, is_text? }`;
//! - `read { path, rev?, offset?, len? }` → `{ bytes }` (base64; null when
//!   no such file), the part `offset`/`len` name when given;
//! - `list { rev?, parent? }` → `{ entries: [{ path, kind }] }`, sorted;
//!   `parent` absent is the whole tree, `""` the root;
//! - `write { path, content }` → `{}` (declared only with `writable`),
//!   `content` base64.

use std::collections::BTreeMap;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::ProtocolError;
use serde_json::{json, Value};

/// Its one revision.
const REV: &str = "v1";

/// The modified time it reports for every file.
const MODIFIED_AT: i64 = 1_700_000_000_000;

/// The files, by path.
#[derive(Default)]
pub(crate) struct Files(BTreeMap<String, Vec<u8>>);

/// What it declares in filesystem mode: the `filesystem` capability and
/// its verbs (`write` only with `writable`), no event types, no
/// collectors; the same config as its work list.
pub fn declarations(writable: bool) -> InitializeResult {
    let path = json!({ "type": "string" });
    let rev = json!({ "type": "string" });
    let mut commands = vec![
        crate::command(
            "stat",
            "What a path is.",
            json!({
                "type": "object", "required": ["path"], "additionalProperties": false,
                "properties": { "path": path, "rev": rev },
            }),
        ),
        crate::command(
            "read",
            "A file's bytes, whole or a range.",
            json!({
                "type": "object", "required": ["path"], "additionalProperties": false,
                "properties": {
                    "path": path, "rev": rev,
                    "offset": { "type": "integer", "minimum": 0 },
                    "len": { "type": "integer", "minimum": 0 },
                },
            }),
        ),
        crate::command(
            "list",
            "The files and folders in a folder, or in the whole tree.",
            json!({
                "type": "object", "additionalProperties": false,
                "properties": { "rev": rev, "parent": { "type": "string" } },
            }),
        ),
    ];
    if writable {
        commands.push(crate::command(
            "write",
            "Replace a file's content.",
            json!({
                "type": "object", "required": ["path", "content"],
                "additionalProperties": false,
                "properties": { "path": path, "content": { "type": "string" } },
            }),
        ));
    }
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: crate::PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "filesystem".into(),
            features: json!({ "writable": writable }),
            data: Value::Null,
        }],
        commands,
        event_types: Vec::new(),
        collectors: Vec::new(),
        config_schema: json!({ "type": "object", "required": ["team"],
                               "properties": { "team": { "type": "string" } } }),
    }
}

fn invalid(field: &str, message: impl Into<String>) -> ProtocolError {
    ProtocolError::InvalidInput {
        field: field.into(),
        message: message.into(),
    }
}

/// `path` as the tree keeps it: relative, `/`-separated, nothing leaving.
fn checked(field: &str, path: &str, allow_root: bool) -> Result<String, ProtocolError> {
    if path.is_empty() && allow_root {
        return Ok(String::new());
    }
    let ok = !path.contains('\\')
        && path
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | ".."));
    if ok {
        Ok(path.to_string())
    } else {
        Err(invalid(field, format!("`{path}` isn't a path inside it")))
    }
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

impl Files {
    /// The tree `json` holds: an object of path → base64 content.
    pub(crate) fn seeded(json: &str) -> Result<Self, String> {
        let map: BTreeMap<String, String> =
            serde_json::from_str(json).map_err(|e| format!("OXPLOW_FAKE_FILES: {e}"))?;
        let mut files = BTreeMap::new();
        for (path, content) in map {
            let bytes = STANDARD
                .decode(content)
                .map_err(|e| format!("OXPLOW_FAKE_FILES: `{path}`: {e}"))?;
            files.insert(path, bytes);
        }
        Ok(Self(files))
    }

    fn dirs(&self) -> std::collections::BTreeSet<String> {
        let mut dirs = std::collections::BTreeSet::new();
        for path in self.0.keys() {
            let mut at = parent_of(path);
            while !at.is_empty() {
                dirs.insert(at.to_string());
                at = parent_of(at);
            }
        }
        dirs
    }

    fn rev(input: &Value) -> Result<(), ProtocolError> {
        match input.get("rev").and_then(Value::as_str) {
            None | Some(REV) => Ok(()),
            Some(other) => Err(invalid("/rev", format!("no revision `{other}`"))),
        }
    }

    /// `verb`'s answer to `input`; `writable` is whether it takes writes.
    pub(crate) fn answer(
        &mut self,
        verb: &str,
        input: &Value,
        writable: bool,
    ) -> Result<Value, ProtocolError> {
        let path_of = |allow_root| {
            checked(
                "/path",
                input["path"].as_str().unwrap_or_default(),
                allow_root,
            )
        };
        match verb {
            "stat" => {
                Self::rev(input)?;
                let path = path_of(false)?;
                let entry = if let Some(bytes) = self.0.get(&path) {
                    json!({ "kind": "file", "size": bytes.len(), "modified_at": MODIFIED_AT,
                            "is_text": !bytes[..bytes.len().min(8192)].contains(&0) })
                } else if self.dirs().contains(&path) {
                    json!({ "kind": "dir" })
                } else {
                    Value::Null
                };
                Ok(json!({ "entry": entry }))
            }
            "read" => {
                Self::rev(input)?;
                let path = path_of(false)?;
                let Some(bytes) = self.0.get(&path) else {
                    return Ok(json!({ "bytes": null }));
                };
                let offset = input["offset"].as_u64().unwrap_or(0);
                let len = input["len"].as_u64().unwrap_or(u64::MAX);
                let from = offset.min(bytes.len() as u64) as usize;
                let to = offset.saturating_add(len).min(bytes.len() as u64) as usize;
                Ok(json!({ "bytes": STANDARD.encode(&bytes[from..to]) }))
            }
            "list" => {
                Self::rev(input)?;
                let parent = match input.get("parent").and_then(Value::as_str) {
                    Some(p) => Some(checked("/parent", p, true)?),
                    None => None,
                };
                let mut entries: Vec<(String, &str)> = self
                    .0
                    .keys()
                    .map(|p| (p.clone(), "file"))
                    .chain(self.dirs().into_iter().map(|d| (d, "dir")))
                    .filter(|(p, _)| parent.as_deref().is_none_or(|want| parent_of(p) == want))
                    .collect();
                entries.sort();
                Ok(json!({ "entries": entries.into_iter()
                    .map(|(path, kind)| json!({ "path": path, "kind": kind }))
                    .collect::<Vec<_>>() }))
            }
            "write" if writable => {
                let path = path_of(false)?;
                let content = STANDARD
                    .decode(input["content"].as_str().unwrap_or_default())
                    .map_err(|e| invalid("/content", e.to_string()))?;
                if self.dirs().contains(&path) {
                    return Err(invalid("/path", format!("`{path}` is a folder")));
                }
                self.0.insert(path, content);
                Ok(json!({}))
            }
            other => Err(invalid(
                "/command",
                format!("a filesystem provider answers stat, read, list or (writable) write, not `{other}`"),
            )),
        }
    }
}
