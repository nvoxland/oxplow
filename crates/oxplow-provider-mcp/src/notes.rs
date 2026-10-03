//! The notes MCP server (the `oxplow-provider-mcp-notes` binary): notes
//! with a title, body, state (`open`, `doing`, `stuck`, `closed`,
//! `dropped`) and parent, in memory — what the MCP adapter's tests and
//! its `notes` fixture extension drive. Each write bumps a revision;
//! `list_items` returns the notes changed after one.
//!
//! It is served over stdio ([`serve_stdio`], a server by `command`) or
//! over streamable HTTP at `/mcp` ([`serve_http`], a server by `url`,
//! P9.B4), there optionally behind a bearer token — anything else is
//! refused the way [`Refusal`] says.

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

/// The notes, as an MCP server's handler. Clones share one book.
#[derive(Clone)]
pub struct Notes {
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

/// A request the notes can't take: a tool error (`isError`), its output
/// naming the argument at fault — data for the mapping to turn into a
/// refusal, not a protocol failure.
fn refused(field: &str, message: String) -> CallToolResult {
    CallToolResult::structured_error(json!({ "error": message, "field": field }))
}

impl Notes {
    fn lock(&self) -> std::sync::MutexGuard<'_, Book> {
        self.book.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The refusal of a state that isn't a note state.
fn bad_state(state: &str) -> Option<CallToolResult> {
    (!STATES.contains(&state)).then(|| {
        refused(
            "state",
            format!("`{state}` isn't a state ({})", STATES.join(", ")),
        )
    })
}

/// The refusal of a parent that isn't a note.
fn bad_parent(book: &Book, parent: &str) -> Option<CallToolResult> {
    (!book.notes.iter().any(|n| n.id == parent))
        .then(|| refused("parent", format!("no note `{parent}`")))
}

#[tool_router]
impl Notes {
    pub fn new() -> Self {
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
        let refusal = bad_state(&state).or_else(|| {
            p.parent
                .as_deref()
                .and_then(|parent| bad_parent(&book, parent))
        });
        if let Some(refusal) = refusal {
            return Ok(refusal);
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
        let refusal = p.state.as_deref().and_then(bad_state).or_else(|| {
            p.parent
                .as_deref()
                .filter(|p| !p.is_empty())
                .and_then(|parent| bad_parent(&book, parent))
        });
        if let Some(refusal) = refusal {
            return Ok(refusal);
        }
        let Some(at) = book.notes.iter().position(|n| n.id == p.id) else {
            return Ok(refused("id", format!("no note `{}`", p.id)));
        };
        book.rev += 1;
        let rev = book.rev;
        let note = &mut book.notes[at];
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

impl Default for Notes {
    fn default() -> Self {
        Self::new()
    }
}

/// Serve the notes over this process's stdio until the client goes.
pub async fn serve_stdio() -> std::io::Result<()> {
    let service = Notes::new()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(std::io::Error::other)?;
    let _ = service.waiting().await;
    Ok(())
}

/// How the HTTP server refuses a request without its bearer token: the
/// ways services answer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `401` with `WWW-Authenticate: Bearer` (RFC 6750 §3).
    Challenge,
    /// `401` and nothing more.
    Bare,
    /// `403` with `WWW-Authenticate: Bearer error="insufficient_scope"`:
    /// the token is known but may not do this.
    Forbidden,
}

/// Which bearer tokens the HTTP server takes: a token check.
pub type Tokens = std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// [`Tokens`] that take `token` and no other.
pub fn only(token: &str) -> Tokens {
    let token = token.to_string();
    std::sync::Arc::new(move |given| given == token)
}

/// Serve the notes over streamable HTTP at `/mcp` on `listener` (see
/// [`http_router`]).
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    tokens: Option<Tokens>,
    refusal: Refusal,
) -> std::io::Result<()> {
    axum::serve(listener, http_router(tokens, refusal)).await
}

/// The notes at `/mcp` over streamable HTTP, every session over one book.
/// With `tokens`, a request whose `Authorization: Bearer <token>` they
/// don't take is refused as `refusal` says. The OAuth stand-in mounts it
/// behind the access tokens it issued (`oxplow-oauth-sim`).
pub fn http_router(tokens: Option<Tokens>, refusal: Refusal) -> axum::Router {
    use axum::http::{header, StatusCode};
    use axum::response::IntoResponse;
    use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService,
    };

    let notes = Notes::new();
    let service: StreamableHttpService<Notes, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(notes.clone()),
        Default::default(),
        StreamableHttpServerConfig::default(),
    );
    axum::Router::new()
        .nest_service("/mcp", service)
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let tokens = tokens.clone();
                async move {
                    let given = request
                        .headers()
                        .get(header::AUTHORIZATION)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.strip_prefix("Bearer "));
                    match tokens {
                        Some(takes) if !given.is_some_and(|t| takes(t)) => match refusal {
                            Refusal::Challenge => (
                                StatusCode::UNAUTHORIZED,
                                [(header::WWW_AUTHENTICATE, "Bearer")],
                            )
                                .into_response(),
                            Refusal::Bare => StatusCode::UNAUTHORIZED.into_response(),
                            Refusal::Forbidden => (
                                StatusCode::FORBIDDEN,
                                [(
                                    header::WWW_AUTHENTICATE,
                                    r#"Bearer error="insufficient_scope", scope="notes:write""#,
                                )],
                            )
                                .into_response(),
                        },
                        _ => next.run(request).await,
                    }
                }
            },
        ))
}
