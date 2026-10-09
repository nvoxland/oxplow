//! The fake as a **snapshots provider** (`OXPLOW_FAKE_CAPABILITY=snapshots`):
//! it marks a worktree and keeps what it saw in memory (never in
//! `OXPLOW_FAKE_STATE`; only the number of marks is kept there).
//!
//! - `mark { stream, worktree, trigger, …, parent? }` walks `worktree`
//!   (skipping `.git` and `.oxplow`), hashes each file (xxh3-128, lower-case
//!   hex), names the state `m<N>` and answers `{ handle, unchanged,
//!   file_count }`; `unchanged` is whether the tree equals `parent`'s;
//! - `changed { stream, from, to }` → `{ changes }`, `from: null` being the
//!   empty tree;
//! - `read_at { handle, path }` → `{ bytes }` (base64), declared only with
//!   the `contents` feature (`OXPLOW_FAKE_FEATURES=contents`).

use std::collections::BTreeMap;
use std::path::Path;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::ProtocolError;
use serde_json::{json, Value};

/// What a marked file was: its identity, size and bytes.
type Tree = BTreeMap<String, (String, u64, Vec<u8>)>;

/// The states it has marked, by handle.
#[derive(Default)]
pub(crate) struct Marks(BTreeMap<String, Tree>);

/// What it declares in snapshots mode: the `snapshots` capability and its
/// verbs (`read_at` only with `contents`), no event types, no collectors;
/// the same config as its work list.
pub fn declarations(contents: bool) -> InitializeResult {
    let handle = json!({ "type": ["string", "null"] });
    let mut commands = vec![
        crate::command(
            "mark",
            "Mark the worktree's state.",
            json!({
                "type": "object",
                "required": ["stream", "worktree", "trigger"],
                "additionalProperties": false,
                "properties": {
                    "stream": { "type": "string" },
                    "worktree": { "type": "string" },
                    "trigger": { "type": "string" },
                    "thread": { "type": "string" },
                    "turn": { "type": "integer" },
                    "effort": { "type": "string" },
                    "budget_ms": { "type": "integer" },
                    "parent": handle,
                },
            }),
        ),
        crate::command(
            "changed",
            "What changed between two marked states.",
            json!({
                "type": "object",
                "required": ["stream", "from", "to"],
                "additionalProperties": false,
                "properties": {
                    "stream": { "type": "string" },
                    "from": handle,
                    "to": { "type": "string" },
                },
            }),
        ),
    ];
    if contents {
        commands.push(crate::command(
            "read_at",
            "A file's bytes at a marked state.",
            json!({
                "type": "object",
                "required": ["handle", "path"],
                "additionalProperties": false,
                "properties": {
                    "handle": { "type": "string" },
                    "path": { "type": "string" },
                },
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
            capability: "snapshots".into(),
            features: json!({ "contents": contents }),
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

fn walk(root: &Path, dir: &Path, into: &mut Tree) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        if entry.file_type()?.is_dir() {
            if name != ".git" && name != ".oxplow" {
                walk(root, &path, into)?;
            }
        } else if entry.file_type()?.is_file() {
            let bytes = std::fs::read(&path)?;
            let rel = path
                .strip_prefix(root)
                .expect("under the root")
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let identity = format!("{:032x}", xxhash_rust::xxh3::xxh3_128(&bytes));
            into.insert(rel, (identity, bytes.len() as u64, bytes));
        }
    }
    Ok(())
}

impl Marks {
    fn tree(&self, field: &str, handle: &str) -> Result<&Tree, ProtocolError> {
        self.0
            .get(handle)
            .ok_or_else(|| invalid(field, format!("no marked state `{handle}`")))
    }

    /// The tree `field` of `input` names (`null`: the empty tree).
    fn side<'a>(&'a self, input: &Value, field: &str) -> Result<Option<&'a Tree>, ProtocolError> {
        match &input[field] {
            Value::Null => Ok(None),
            Value::String(h) => self.tree(&format!("/{field}"), h).map(Some),
            _ => Err(invalid(&format!("/{field}"), "a handle or null")),
        }
    }

    /// `verb`'s answer to `input`; `marked` is how many marks it has made
    /// (the next handle is `m<marked + 1>`).
    pub(crate) fn answer(
        &mut self,
        verb: &str,
        input: &Value,
        marked: u64,
        contents: bool,
    ) -> Result<Value, ProtocolError> {
        match verb {
            "mark" => {
                let worktree = input["worktree"]
                    .as_str()
                    .ok_or_else(|| invalid("/worktree", "a path"))?;
                let root = Path::new(worktree);
                let mut tree = Tree::new();
                walk(root, root, &mut tree).map_err(|e| invalid("/worktree", e.to_string()))?;
                let unchanged = match self.side(input, "parent")? {
                    Some(parent) => {
                        let ids = |t: &Tree| -> Vec<_> {
                            t.iter().map(|(p, (id, ..))| (p.clone(), id.clone())).collect()
                        };
                        ids(parent) == ids(&tree)
                    }
                    None => tree.is_empty(),
                };
                let handle = format!("m{}", marked + 1);
                let file_count = tree.len();
                self.0.insert(handle.clone(), tree);
                Ok(json!({ "handle": handle, "unchanged": unchanged, "file_count": file_count }))
            }
            "changed" => {
                let empty = Tree::new();
                let from = self.side(input, "from")?.unwrap_or(&empty);
                let to = self
                    .side(input, "to")?
                    .ok_or_else(|| invalid("/to", "a handle"))?;
                let mut changes = Vec::new();
                for (path, (identity, size, _)) in to {
                    let kind = match from.get(path) {
                        None => "added",
                        Some((was, ..)) if was != identity => "modified",
                        Some(_) => continue,
                    };
                    changes.push(json!({ "path": path, "kind": kind,
                                         "identity": identity, "size": size }));
                }
                for path in from.keys().filter(|p| !to.contains_key(*p)) {
                    changes.push(json!({ "path": path, "kind": "deleted" }));
                }
                Ok(json!({ "changes": changes }))
            }
            "read_at" if contents => {
                let handle = input["handle"].as_str().unwrap_or_default();
                let path = input["path"].as_str().unwrap_or_default();
                let (.., bytes) = self
                    .tree("/handle", handle)?
                    .get(path)
                    .ok_or_else(|| invalid("/path", format!("`{path}` isn't in `{handle}`")))?;
                Ok(json!({ "bytes": STANDARD.encode(bytes) }))
            }
            other => Err(invalid(
                "/command",
                format!("a snapshots provider answers mark, changed or (with contents) read_at, not `{other}`"),
            )),
        }
    }
}
