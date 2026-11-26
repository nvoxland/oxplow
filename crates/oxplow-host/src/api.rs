//! The host's HTTP API under `/host/…`.
//!
//! `GET /host/status` (version, pid, projects with their state),
//! `GET /host/projects`, `POST /host/projects` (a JSON `{ path }`: register
//! that folder, or find it registered; answers the project),
//! `POST /host/holds` and `DELETE /host/holds` (a JSON `{ device, project }`;
//! each answers the project's view), and `POST /host/shutdown` (the host
//! stops, its daemons with it).
//!
//! Every route needs the host token (`Authorization: Bearer`). It is read from
//! stdin at start, like the daemon's UI token. Device tokens and pairing
//! replace it in the pairing step.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::host::{Host, HostError, ProjectView};

#[derive(Clone)]
struct ApiState {
    host: Arc<Host>,
    token: Arc<str>,
    /// Told when a client asks the host to stop; `serve` waits on it.
    stop: Arc<tokio::sync::Notify>,
}

#[derive(Serialize)]
struct Status {
    version: &'static str,
    pid: u32,
    projects: Vec<ProjectView>,
}

#[derive(Deserialize)]
struct AddProject {
    path: std::path::PathBuf,
}

#[derive(Deserialize)]
struct HoldRequest {
    device: String,
    project: String,
}

/// `stop` is told when a client asks the host to stop (`POST /host/shutdown`).
pub fn router(host: Arc<Host>, token: String, stop: Arc<tokio::sync::Notify>) -> Router {
    let state = ApiState {
        host,
        token: token.into(),
        stop,
    };
    Router::new()
        .route("/host/status", get(status))
        .route("/host/projects", get(projects).post(add_project))
        .route("/host/holds", axum::routing::post(hold).delete(release))
        .route("/host/shutdown", axum::routing::post(shutdown))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_token,
        ))
        .with_state(state)
}

/// Equal without stopping at the first difference, so timing says nothing of
/// how much of a guess was right.
pub(crate) fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// The bearer token a request carries.
pub(crate) fn bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

async fn require_token(State(state): State<ApiState>, req: Request, next: Next) -> Response {
    match bearer(req.headers()) {
        Some(t) if same_secret(t, &state.token) => next.run(req).await,
        _ => (StatusCode::UNAUTHORIZED, "missing or wrong host token").into_response(),
    }
}

async fn status(State(s): State<ApiState>) -> Json<Status> {
    Json(Status {
        version: crate::VERSION,
        pid: std::process::id(),
        projects: s.host.projects(),
    })
}

async fn projects(State(s): State<ApiState>) -> Json<Vec<ProjectView>> {
    Json(s.host.projects())
}

async fn add_project(State(s): State<ApiState>, Json(req): Json<AddProject>) -> Response {
    match s.host.registry().add(&req.path) {
        Ok(project) => Json(project).into_response(),
        Err(
            e @ (crate::registry::RegistryError::NotAProject(_)
            | crate::registry::RegistryError::Unreadable(..)),
        ) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn shutdown(State(s): State<ApiState>) -> StatusCode {
    tracing::info!("a client asked the host to stop");
    s.stop.notify_one();
    StatusCode::ACCEPTED
}

async fn hold(State(s): State<ApiState>, Json(req): Json<HoldRequest>) -> Response {
    answer(s.host.add_hold(&req.device, &req.project))
}

async fn release(State(s): State<ApiState>, Json(req): Json<HoldRequest>) -> Response {
    answer(s.host.release_hold(&req.device, &req.project))
}

fn answer(result: Result<ProjectView, HostError>) -> Response {
    match result {
        Ok(view) => Json(view).into_response(),
        Err(e @ HostError::UnknownProject(_)) => {
            (StatusCode::NOT_FOUND, e.to_string()).into_response()
        }
        Err(e @ HostError::NoDevice) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::tests::{Fixture, RUNS};
    use serde_json::json;

    const TOKEN: &str = "host-secret";

    async fn serve(fx: &Fixture) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(fx.host.clone(), TOKEN.to_string(), Default::default());
        tokio::spawn(async move { axum::serve(listener, app).await });
        url
    }

    #[tokio::test]
    async fn every_route_needs_the_host_token() {
        let fx = Fixture::new(RUNS);
        let url = serve(&fx).await;
        let http = reqwest::Client::new();
        for (method, path) in [
            (reqwest::Method::GET, "/host/status"),
            (reqwest::Method::GET, "/host/projects"),
            (reqwest::Method::POST, "/host/holds"),
            (reqwest::Method::DELETE, "/host/holds"),
            (reqwest::Method::POST, "/host/projects"),
            (reqwest::Method::POST, "/host/shutdown"),
        ] {
            let bare = http
                .request(method.clone(), format!("{url}{path}"))
                .send()
                .await
                .unwrap();
            assert_eq!(bare.status(), 401, "{method} {path} without a token");
            let wrong = http
                .request(method.clone(), format!("{url}{path}"))
                .bearer_auth("nope")
                .send()
                .await
                .unwrap();
            assert_eq!(wrong.status(), 401, "{method} {path} with a wrong token");
        }
        assert!(fx.host.holds().is_empty());
    }

    #[tokio::test]
    async fn status_and_projects_say_what_runs() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        let url = serve(&fx).await;
        let http = reqwest::Client::new();

        let status: serde_json::Value = http
            .get(format!("{url}/host/status"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["version"], crate::VERSION);
        assert_eq!(status["projects"][0]["id"], id);
        assert_eq!(status["projects"][0]["state"], "stopped");

        let projects: serde_json::Value = http
            .get(format!("{url}/host/projects"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(projects[0]["name"], "alpha");
    }

    #[tokio::test]
    async fn a_hold_starts_the_project_and_deleting_it_stops_it() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        let url = serve(&fx).await;
        let http = reqwest::Client::new();
        let body = json!({ "device": "dev-a", "project": id });

        let held: serde_json::Value = http
            .post(format!("{url}/host/holds"))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(held["state"], "starting");
        assert_eq!(held["holds"], 1);
        let id2 = id.clone();
        let running = tokio::task::spawn_blocking({
            // Waits on the worker thread, off the async runtime.
            let host = fx.host.clone();
            move || {
                for _ in 0..400 {
                    if host
                        .projects()
                        .iter()
                        .any(|p| p.id == id2 && p.state == crate::host::DaemonState::Running)
                    {
                        return true;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                false
            }
        })
        .await
        .unwrap();
        assert!(running);

        let released = http
            .delete(format!("{url}/host/holds"))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(released.status(), 200);
        let released: serde_json::Value = released.json().await.unwrap();
        assert_eq!(released["holds"], 0);
    }

    /// The Mac app opens a folder by its path: the host registers it (or
    /// finds it registered) and answers its id.
    #[tokio::test]
    async fn a_project_is_registered_by_its_path() {
        let fx = Fixture::new(RUNS);
        let url = serve(&fx).await;
        let http = reqwest::Client::new();
        let dir = fx.dir.path().join("beta");
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        let add = || {
            http.post(format!("{url}/host/projects"))
                .bearer_auth(TOKEN)
                .json(&json!({ "path": dir }))
                .send()
        };
        let first: serde_json::Value = add().await.unwrap().json().await.unwrap();
        assert_eq!(first["name"], "beta");
        assert!(first["id"].as_str().unwrap().starts_with("prj-"), "{first}");
        let again: serde_json::Value = add().await.unwrap().json().await.unwrap();
        assert_eq!(
            again["id"], first["id"],
            "the same folder is the same project"
        );

        let plain = fx.dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let refused = http
            .post(format!("{url}/host/projects"))
            .bearer_auth(TOKEN)
            .json(&json!({ "path": plain }))
            .send()
            .await
            .unwrap();
        assert_eq!(refused.status(), 400);
        assert!(refused
            .text()
            .await
            .unwrap()
            .contains("not an oxplow project"));
    }

    /// A client ends a headless host this way.
    #[tokio::test]
    async fn shutdown_asks_the_host_to_stop() {
        let fx = Fixture::new(RUNS);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(tokio::sync::Notify::new());
        let app = router(fx.host.clone(), TOKEN.to_string(), stop.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        let http = reqwest::Client::new();
        let bare = http
            .post(format!("{url}/host/shutdown"))
            .send()
            .await
            .unwrap();
        assert_eq!(bare.status(), 401);
        let asked = http
            .post(format!("{url}/host/shutdown"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(asked.status(), 202);
        tokio::time::timeout(std::time::Duration::from_secs(1), stop.notified())
            .await
            .expect("the stop was asked for");
    }

    #[tokio::test]
    async fn a_bad_hold_is_told_why() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        let url = serve(&fx).await;
        let http = reqwest::Client::new();
        let unknown = http
            .post(format!("{url}/host/holds"))
            .bearer_auth(TOKEN)
            .json(&json!({ "device": "dev-a", "project": "prj-00000000" }))
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 404);
        let nameless = http
            .post(format!("{url}/host/holds"))
            .bearer_auth(TOKEN)
            .json(&json!({ "device": "", "project": id }))
            .send()
            .await
            .unwrap();
        assert_eq!(nameless.status(), 400);
    }
}
