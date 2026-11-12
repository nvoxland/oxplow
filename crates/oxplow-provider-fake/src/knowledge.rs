//! The fake as a **knowledge provider** (`OXPLOW_FAKE_CAPABILITY=knowledge`):
//! pages in memory (kept in `OXPLOW_FAKE_STATE` as items are), each write
//! answering the page's `knowledge.page.recorded@3` event, the store in its
//! URI the instance's id:
//!
//! - `write_page { id?, title?, body, verified_files, removed_files }` →
//!   `{ id }`; with no `id` it takes one from the title (lowercased, its
//!   other characters `-`); the title defaults to the id, and it keeps no
//!   pins, so the two ref lists are accepted and ignored;
//! - `delete_page { id }` → `{}`, the page marked `deleted`.
//!
//! It declares `features { edit: true }` and `data { kinds: [note] }`. A page's `uris`
//! are its body's `[[…]]` interiors in oxplow's ref grammar: one with a `/`
//! or a `.` is a file (`oxplow:oxplow/file?path=<path>`), `<store>:<id>` that
//! store's page, a bare one oxwiki's. The collector `knowledge_pages`
//! streams the pages changed after its cursor.

use std::collections::BTreeMap;

use oxplow_provider_protocol::model::*;
use oxplow_provider_protocol::ProtocolError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The event each write answers.
pub const RECORDED: &str = "knowledge.page.recorded";

/// A page as the fake keeps it: its record, and the world's revision when
/// it last changed (a read streams the pages changed after its cursor).
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Page {
    record: Value,
    pub(crate) rev: u64,
}

impl Page {
    pub(crate) fn record(&self) -> &Value {
        &self.record
    }
}

pub(crate) type Pages = BTreeMap<String, Page>;

/// What it declares in knowledge mode: the `knowledge` capability with its
/// kinds, its two verbs, the recorded event and a collector of the pages;
/// the same config in every mode.
pub fn declarations() -> InitializeResult {
    let string = json!({ "type": "string" });
    let refs = json!({ "type": "array", "items": string });
    InitializeResult {
        protocol_version: PROTOCOL_VERSION.into(),
        provider: Party {
            name: crate::PROVIDER.into(),
            version: "1".into(),
        },
        capabilities: vec![CapabilityDecl {
            capability: "knowledge".into(),
            features: json!({ "edit": true }),
            data: json!({ "kinds": [{ "id": "note", "title": "Note", "bucket": "note" }] }),
        }],
        commands: vec![
            crate::command(
                "write_page",
                "Write a page: create it, or replace its body.",
                json!({ "type": "object", "required": ["body", "verified_files", "removed_files"],
                        "additionalProperties": false,
                        "properties": { "id": string, "title": string, "body": string,
                                        "verified_files": refs, "removed_files": refs } }),
            ),
            crate::command(
                "delete_page",
                "Delete a page.",
                json!({ "type": "object", "required": ["id"], "additionalProperties": false,
                        "properties": { "id": string } }),
            ),
        ],
        event_types: vec![EventTypeDecl {
            event_type: RECORDED.into(),
            v: 3,
            // Core's schema, as a provider must declare it.
            schema: oxplow_domain::vocabulary::Vocabulary::core()
                .schema(
                    <oxplow_domain::events::schema::KnowledgePageRecorded as oxplow_domain::events::schema::EventType>::TYPE,
                    3,
                )
                .cloned()
                .unwrap_or(Value::Null),
        }],
        collectors: vec![CollectorDecl {
            name: "knowledge_pages".into(),
            entity: "knowledge_page".into(),
            description: "Every page, after the cursor.".into(),
        }],
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

fn str_field(input: &Value, field: &str) -> Result<String, ProtocolError> {
    input
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| invalid(&format!("/{field}"), "required"))
}

/// `body`'s `[[…]]` interiors as oxplow refs, once each, in order.
pub(crate) fn refs_of(body: &str) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    let mut rest = body;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else { break };
        let inner = after[..close].trim();
        if !inner.is_empty() {
            let r = match inner.split_once(':') {
                Some((store, id)) => oxplow_domain::uri::build::knowledge_uri(store, id),
                None if inner.contains('/') || inner.contains('.') => {
                    oxplow_domain::uri::build::file_uri(inner)
                }
                None => oxplow_domain::uri::build::knowledge_uri(
                    oxplow_domain::knowledge::OXWIKI,
                    inner,
                ),
            };
            if !refs.contains(&r) {
                refs.push(r);
            }
        }
        rest = &after[close + 2..];
    }
    refs
}

/// A clock that moves one second per revision, so records are stable.
fn updated_at(rev: u64) -> String {
    format!(
        "2026-01-01T{:02}:{:02}:{:02}Z",
        rev / 3600 % 24,
        rev / 60 % 60,
        rev % 60
    )
}

fn event(record: &Value) -> Value {
    serde_json::to_value(EventDraft {
        event_type: RECORDED.into(),
        v: 3,
        payload: json!({ "page": record }),
        subject: vec![record["uri"].as_str().unwrap_or_default().to_string()],
    })
    .expect("a draft serializes")
}

/// Put `record` in `pages` at a new revision; the event it answers.
fn keep(pages: &mut Pages, rev: &mut u64, id: &str, mut record: Value) -> Value {
    *rev += 1;
    record["updated_at"] = json!(updated_at(*rev));
    let answer = event(&record);
    pages.insert(id.to_string(), Page { record, rev: *rev });
    answer
}

fn record_of(store: &str, id: &str, title: &str, body: &str, deleted: bool) -> Value {
    let mut record = json!({ "uri": oxplow_domain::uri::build::knowledge_uri(store, id),
                             "kind": "note", "title": title, "body": body,
                             "uris": refs_of(body), "updated_at": "" });
    if deleted {
        record["deleted"] = json!(true);
    }
    record
}

/// The id a title makes: its letters and digits lowercased, every other run
/// of characters one `-`.
fn id_from(title: &str) -> String {
    let mut id = String::new();
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            id.push(c.to_ascii_lowercase());
        } else if !id.is_empty() && !id.ends_with('-') {
            id.push('-');
        }
    }
    id.trim_end_matches('-').to_string()
}

/// `verb`'s answer to `input`, as the store `store` (the instance):
/// `(result, events)`. `rev` is the world's revision, bumped by a write.
pub(crate) fn answer(
    store: &str,
    pages: &mut Pages,
    rev: &mut u64,
    verb: &str,
    input: &Value,
) -> Result<(Value, Vec<Value>), ProtocolError> {
    match verb {
        "write_page" => {
            let body = str_field(input, "body")?;
            let title = input.get("title").and_then(Value::as_str);
            let id = match input.get("id").and_then(Value::as_str) {
                Some(id) => id.to_string(),
                None => id_from(title.ok_or_else(|| invalid("/id", "an id or a title"))?),
            };
            if id.is_empty() {
                return Err(invalid("/title", "it makes no id"));
            }
            let title = title.unwrap_or(&id).to_string();
            let recorded = keep(pages, rev, &id, record_of(store, &id, &title, &body, false));
            Ok((json!({ "id": id }), vec![recorded]))
        }
        "delete_page" => {
            let id = str_field(input, "id")?;
            let page = pages
                .get(&id)
                .filter(|p| p.record["deleted"] != json!(true))
                .ok_or_else(|| invalid("/id", format!("no page `{id}`")))?;
            let record = record_of(
                store,
                &id,
                page.record["title"].as_str().unwrap_or(&id),
                page.record["body"].as_str().unwrap_or_default(),
                true,
            );
            let recorded = keep(pages, rev, &id, record);
            Ok((json!({}), vec![recorded]))
        }
        other => Err(invalid(
            "/command",
            format!("a knowledge provider answers write_page or delete_page, not `{other}`"),
        )),
    }
}
