//! `oxplow-provider-mcp-notes`: a small MCP server over stdio — notes with
//! a title, body, state (`open`, `doing`, `stuck`, `closed`, `dropped`)
//! and parent, in memory — that the MCP adapter's tests and its `notes`
//! fixture extension drive. Each write bumps a revision; `list_items`
//! returns the notes changed after one.

use std::sync::{Arc, Mutex};

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{json, Value};

const STATES: &[&str] = &["open", "doing", "stuck", "closed", "dropped"];

#[derive(Clone)]
struct Note {
    id: String,
    title: String,
    body: String,
    state: String,
    parent: Option<String>,
    rev: u64,
}

impl Note {
    fn json(&self) -> Value {
        json!({ "id": self.id, "title": self.title, "body": self.body,
                "state": self.state, "parent": self.parent, "rev": self.rev })
    }
}

#[derive(Default)]
struct Book {
    notes: Vec<Note>,
    rev: u64,
}

#[derive(Clone)]
struct Notes {
    book: Arc<Mutex<Book>>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct ListParams {
    /// Only notes changed after this revision.
    since: Option<u64>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct CreateParams {
    title: String,
    body: Option<String>,
    /// One of open, doing, stuck, closed, dropped (default open).
    state: Option<String>,
    /// The parent note's id.
    parent: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
struct UpdateParams {
    id: String,
    title: Option<String>,
    body: Option<String>,
    state: Option<String>,
    /// The parent note's id; "" detaches it.
    parent: Option<String>,
}

fn invalid(message: String) -> ErrorData {
    ErrorData::invalid_params(message, None)
}

impl Notes {
    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|p| p.into_inner())
    }
}

fn check_state(state: &str) -> Result<(), ErrorData> {
    if STATES.contains(&state) {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{state}` isn't a state ({})",
            STATES.join(", ")
        )))
    }
}

fn check_parent(book: &Book, parent: &str) -> Result<(), ErrorData> {
    if book.notes.iter().any(|n| n.id == parent) {
        Ok(())
    } else {
        Err(invalid(format!("no note `{parent}`")))
    }
}

#[tool_router]
impl Notes {
    fn new() -> Self {
        Notes {
            book: Arc::default(),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "List the notes changed after a revision, with the latest revision.")]
    async fn list_items(
        &self,
        params: Parameters<ListParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let since = params.0.since.unwrap_or(0);
        let book = self.lock();
        let mut changed: Vec<&Note> = book.notes.iter().filter(|n| n.rev > since).collect();
        changed.sort_by_key(|n| n.rev);
        let cursor = changed.last().map_or(since, |n| n.rev);
        let items: Vec<Value> = changed.iter().map(|n| n.json()).collect();
        Ok(CallToolResult::structured(
            json!({ "items": items, "cursor": cursor }),
        ))
    }

    #[tool(description = "Create a note.")]
    async fn create_item(
        &self,
        params: Parameters<CreateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let mut book = self.lock();
        let state = p.state.unwrap_or_else(|| "open".into());
        check_state(&state)?;
        if let Some(parent) = &p.parent {
            check_parent(&book, parent)?;
        }
        book.rev += 1;
        let note = Note {
            id: format!("N-{}", book.notes.len() + 1),
            title: p.title,
            body: p.body.unwrap_or_default(),
            state,
            parent: p.parent,
            rev: book.rev,
        };
        let out = note.json();
        book.notes.push(note);
        Ok(CallToolResult::structured(out))
    }

    #[tool(description = "Change a note's title, body, state or parent.")]
    async fn update_item(
        &self,
        params: Parameters<UpdateParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let p = params.0;
        let mut book = self.lock();
        if let Some(state) = &p.state {
            check_state(state)?;
        }
        if let Some(parent) = p.parent.as_deref().filter(|p| !p.is_empty()) {
            check_parent(&book, parent)?;
        }
        book.rev += 1;
        let rev = book.rev;
        let note = book
            .notes
            .iter_mut()
            .find(|n| n.id == p.id)
            .ok_or_else(|| invalid(format!("no note `{}`", p.id)))?;
        if let Some(t) = p.title {
            note.title = t;
        }
        if let Some(b) = p.body {
            note.body = b;
        }
        if let Some(s) = p.state {
            note.state = s;
        }
        if let Some(parent) = p.parent {
            note.parent = (!parent.is_empty()).then_some(parent);
        }
        note.rev = rev;
        Ok(CallToolResult::structured(note.json()))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Notes {}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let Ok(service) = Notes::new().serve(rmcp::transport::stdio()).await else {
        std::process::exit(1);
    };
    let _ = service.waiting().await;
}
