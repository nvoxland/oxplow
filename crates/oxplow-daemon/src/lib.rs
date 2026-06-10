//! Headless oxplow backend daemon.
//!
//! Boots the same `Services` + background orchestration as the Tauri
//! desktop shell, then serves the shared `oxplow-rpc` dispatch over
//! HTTP on loopback:
//!
//! - `POST /ipc/:name` — JSON body = the renderer's invoke args
//!   (camelCase keys); response = the tauri-specta result envelope
//!   `{"status":"ok","data":…}` / `{"status":"error","error":…}` so the
//!   frontend's existing unwrap path works unchanged.
//! - `GET /events` — WebSocket multiplexing the oxplow / lsp /
//!   terminal event channels (wired by the streaming workstream).
//!
//! Single-user by design: bind to `127.0.0.1` on the remote box and
//! reach it through `ssh -L <localPort>:127.0.0.1:<port>`. SSH is the
//! auth layer; the daemon itself runs no TLS and no token check (a
//! bearer stub can be added behind a flag for direct-expose later).
//!
//! Library shape (`run_server` + `Daemon`) so integration tests can
//! boot the full stack on an ephemeral port in-process.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path as AxumPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use oxplow_app::Services;

/// Shared router state: the booted services plus the control-plane
/// info (hook/MCP URLs + token) for a future remote agent-spawn path.
#[derive(Clone)]
pub struct DaemonState {
    pub services: Arc<Services>,
    pub hook_base_url: String,
    pub mcp_endpoint_url: String,
    pub hook_token: String,
}

/// Handle returned by [`run_server`]: the bound address (useful when
/// binding port 0 in tests) and the join handle for the accept loop.
pub struct Daemon {
    pub bind_addr: SocketAddr,
    pub task: tokio::task::JoinHandle<()>,
}

/// `POST /ipc/:name` — dispatch a command by wire name. The body is
/// the args object the renderer already sends to Tauri's `invoke`
/// (absent/`null` for no-arg commands). Always replies 200 with the
/// tauri-specta envelope; transport-level errors (unreadable body)
/// reply 400. An unknown command name lands as a NOT_FOUND envelope,
/// mirroring what the dispatch registry returns.
async fn ipc_handler(
    State(state): State<DaemonState>,
    AxumPath(name): AxumPath<String>,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let args = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);
    let result = oxplow_rpc::dispatch(&name, args, &state.services).await;
    let envelope = match result {
        Ok(data) => serde_json::json!({ "status": "ok", "data": data }),
        Err(e) => serde_json::json!({ "status": "error", "error": e }),
    };
    (StatusCode::OK, Json(envelope)).into_response()
}

/// Liveness probe for tunnels/scripts (the renderer uses `/ipc/ping`).
async fn health() -> &'static str {
    "ok"
}

/// Build the daemon router. Split out so tests and the binary share
/// the exact route table.
pub fn router(state: DaemonState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ipc/{name}", post(ipc_handler))
        .with_state(state)
}

/// Bind `addr` and serve the daemon router on it. Returns the bound
/// address (resolves port 0) and the detached server task.
pub async fn run_server(addr: SocketAddr, state: DaemonState) -> std::io::Result<Daemon> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bind_addr = listener.local_addr()?;
    let app = router(state);
    let task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "daemon http server exited");
        }
    });
    Ok(Daemon { bind_addr, task })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn services() -> (Arc<Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "test"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["commit", "-q", "--allow-empty", "-m", "init"]);
        let svc = Arc::new(Services::in_memory(dir.path()).unwrap());
        (svc, dir)
    }

    fn daemon_state(services: Arc<Services>) -> DaemonState {
        DaemonState {
            services,
            hook_base_url: "http://127.0.0.1:0/hook".into(),
            mcp_endpoint_url: "http://127.0.0.1:0/mcp".into(),
            hook_token: "test-token".into(),
        }
    }

    #[tokio::test]
    async fn ipc_ping_returns_ok_envelope() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/ping", daemon.bind_addr);
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::Value::Null)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "ok");
        assert_eq!(resp["data"], "pong");
    }

    #[tokio::test]
    async fn ipc_list_streams_round_trips() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/list_streams", daemon.bind_addr);
        // No body at all — mirrors the renderer omitting args.
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "ok");
        assert!(resp["data"].is_array());
        // ensure_primary hasn't run (no boot orchestration in this
        // test), so the list may be empty — the envelope shape is the
        // contract under test, not project seeding.
    }

    #[tokio::test]
    async fn ipc_unknown_command_is_error_envelope() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/definitely_not_a_command", daemon.bind_addr);
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "error");
        assert_eq!(resp["error"]["code"], "NOT_FOUND");
    }

    #[tokio::test]
    async fn ipc_bad_args_is_invalid_envelope() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/ipc/get_task", daemon.bind_addr);
        let resp: serde_json::Value = reqwest::Client::new()
            .post(&url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(resp["status"], "error");
        assert_eq!(resp["error"]["code"], "INVALID");
    }

    #[tokio::test]
    async fn health_route_responds() {
        let (svc, _dir) = services();
        let daemon = run_server("127.0.0.1:0".parse().unwrap(), daemon_state(svc))
            .await
            .unwrap();
        let url = format!("http://{}/health", daemon.bind_addr);
        let body = reqwest::get(&url).await.unwrap().text().await.unwrap();
        assert_eq!(body, "ok");
    }
}
