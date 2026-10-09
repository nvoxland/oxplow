//! The fake as a **knowledge provider** (`OXPLOW_FAKE_CAPABILITY=knowledge`):
//! pages in memory (kept in `OXPLOW_FAKE_STATE` as items are), each write
//! answering the page's `knowledge.page.recorded@1` event:
//!
//! - `write_page { slug, title?, body, verified_refs, removed_refs }` →
//!   `{ page: "wiki:<slug>" }`; the title defaults to the slug, and it
//!   keeps no pins, so the two ref lists are accepted and ignored;
//! - `delete_page { slug }` → `{}`, the page marked `deleted`;
//! - `link { page, target }` → `{}`: `[[target]]` appended under a
//!   `## Related` heading.
//!
//! A page's `refs` are its body's `[[…]]` interiors in oxplow's ref
//! grammar: one with a `/` or a `.` is a file (`file:<path>`), a bare one
//! a page (`wiki:<slug>`). The collector `knowledge_pages` streams the
//! pages changed after its cursor.

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

/// What it declares in knowledge mode: the `knowledge` capability, its
/// three verbs, the recorded event and a collector of the pages; the same
/// config as its work list.
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
            features: json!({}),
            data: Value::Null,
        }],
        commands: vec![
            crate::command(
                "write_page",
                "Write a page: create it, or replace its body.",
                json!({ "type": "object", "required": ["slug", "body", "verified_refs", "removed_refs"],
                        "additionalProperties": false,
                        "properties": { "slug": string, "title": string, "body": string,
                                        "verified_refs": refs, "removed_refs": refs } }),
            ),
            crate::command(
                "delete_page",
                "Delete a page.",
                json!({ "type": "object", "required": ["slug"], "additionalProperties": false,
                        "properties": { "slug": string } }),
            ),
            crate::command(
                "link",
                "Link a page to a target, under its Related heading.",
                json!({ "type": "object", "required": ["page", "target"],
                        "additionalProperties": false,
                        "properties": { "page": string, "target": string } }),
            ),
        ],
        event_types: vec![EventTypeDecl {
            event_type: RECORDED.into(),
            v: 1,
            schema: json!({
                "type": "object", "required": ["page"], "additionalProperties": false,
                "properties": { "page": {
                    "type": "object",
                    "required": ["ref", "title", "body", "refs", "updated_at"],
                    "additionalProperties": false,
                    "properties": {
                        "ref": string, "title": string, "body": string, "refs": refs,
                        "updated_at": string, "deleted": { "type": "boolean" },
                    },
                } },
            }),
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
            let r = if inner.contains('/') || inner.contains('.') {
                format!("file:{inner}")
            } else {
                format!("wiki:{inner}")
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
        v: 1,
        payload: json!({ "page": record }),
        subject: vec![record["ref"].as_str().unwrap_or_default().to_string()],
    })
    .expect("a draft serializes")
}

/// Put `record` in `pages` at a new revision; the event it answers.
fn keep(pages: &mut Pages, rev: &mut u64, slug: &str, mut record: Value) -> Value {
    *rev += 1;
    record["updated_at"] = json!(updated_at(*rev));
    let answer = event(&record);
    pages.insert(slug.to_string(), Page { record, rev: *rev });
    answer
}

fn record_of(slug: &str, title: &str, body: &str, deleted: bool) -> Value {
    let mut record = json!({ "ref": format!("wiki:{slug}"), "title": title, "body": body,
                             "refs": refs_of(body), "updated_at": "" });
    if deleted {
        record["deleted"] = json!(true);
    }
    record
}

/// `verb`'s answer to `input`: `(result, events)`. `rev` is the world's
/// revision, bumped by a write.
pub(crate) fn answer(
    pages: &mut Pages,
    rev: &mut u64,
    verb: &str,
    input: &Value,
) -> Result<(Value, Vec<Value>), ProtocolError> {
    match verb {
        "write_page" => {
            let slug = str_field(input, "slug")?;
            let body = str_field(input, "body")?;
            let title = input
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(&slug)
                .to_string();
            let recorded = keep(pages, rev, &slug, record_of(&slug, &title, &body, false));
            Ok((json!({ "page": format!("wiki:{slug}") }), vec![recorded]))
        }
        "delete_page" => {
            let slug = str_field(input, "slug")?;
            let page = pages
                .get(&slug)
                .filter(|p| p.record["deleted"] != json!(true))
                .ok_or_else(|| invalid("/slug", format!("no page `{slug}`")))?;
            let record = record_of(
                &slug,
                page.record["title"].as_str().unwrap_or(&slug),
                page.record["body"].as_str().unwrap_or_default(),
                true,
            );
            let recorded = keep(pages, rev, &slug, record);
            Ok((json!({}), vec![recorded]))
        }
        "link" => {
            let page = str_field(input, "page")?;
            let slug = page.strip_prefix("wiki:").unwrap_or(&page).to_string();
            let target = str_field(input, "target")?;
            let target = target
                .strip_prefix("wiki:")
                .or_else(|| target.strip_prefix("file:"))
                .unwrap_or(&target)
                .to_string();
            let existing = pages
                .get(&slug)
                .filter(|p| p.record["deleted"] != json!(true))
                .ok_or_else(|| invalid("/page", format!("no page `{slug}`")))?;
            let title = existing.record["title"]
                .as_str()
                .unwrap_or(&slug)
                .to_string();
            let mut body = existing.record["body"]
                .as_str()
                .unwrap_or_default()
                .trim_end()
                .to_string();
            if !body.lines().any(|l| l.trim() == "## Related") {
                body.push_str("\n\n## Related");
            }
            body.push_str(&format!("\n\n[[{target}]]\n"));
            let recorded = keep(pages, rev, &slug, record_of(&slug, &title, &body, false));
            Ok((json!({}), vec![recorded]))
        }
        other => Err(invalid(
            "/command",
            format!("a knowledge provider answers write_page, delete_page or link, not `{other}`"),
        )),
    }
}
