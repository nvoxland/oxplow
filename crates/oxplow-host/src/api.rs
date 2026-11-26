//! The host's HTTP API under `/host/…`. Which caller may use a route
//! (`crate::access`):
//!
//! - owner only: `POST /host/pairing` (a code for a device), `GET
//!   /host/devices`, `PATCH /host/devices/<id>` `{ name }`, `DELETE
//!   /host/devices/<id>`, `POST /host/projects` `{ path }` (register a folder,
//!   or find it registered), `POST /host/shutdown`;
//! - owner or device: `GET /host/status` (version, pid, projects, connected
//!   devices), `GET /host/projects`, `POST|DELETE /host/holds` `{ device,
//!   project }` (a device's hold is always its own), `GET /host/me` (who the
//!   caller is, and the served UI's build), `DELETE /host/me` (a device
//!   forgets itself);
//! - anyone who reaches the host: `POST /host/pair` `{ code, name, kind }`
//!   answers `{ device, token }`. It must be JSON (so a page elsewhere
//!   cannot send it without a preflight, and no route here answers one) and
//!   same-origin when it says an origin. A wrong code costs a wait, one
//!   attempt at a time.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use oxplow_domain::{DeviceId, DeviceKind};
use serde::{Deserialize, Serialize};

use crate::access::{Access, Caller, Connected, Listener};
use crate::devices::{show_code, DevicesError, PairedDevice};
use crate::host::{Host, HostError, ProjectView};
use crate::web::refuse;

/// What the API is given.
pub struct ApiConfig {
    pub host: Arc<Host>,
    pub access: Arc<Access>,
    pub listener: Listener,
    /// Told when a client asks the host to stop; `serve` waits on it.
    pub stop: Arc<tokio::sync::Notify>,
    /// The served UI, for its build id.
    pub ui_dir: Option<PathBuf>,
}

#[derive(Clone)]
struct ApiState {
    host: Arc<Host>,
    access: Arc<Access>,
    listener: Listener,
    stop: Arc<tokio::sync::Notify>,
    ui_dir: Option<Arc<PathBuf>>,
}

#[derive(Serialize)]
struct Status {
    version: &'static str,
    pid: u32,
    projects: Vec<ProjectView>,
    /// Devices with an open event socket now.
    connected: Vec<Connected>,
    /// Where other devices reach this host, or why they cannot.
    public: crate::access::PublicListener,
}

#[derive(Deserialize)]
struct AddProject {
    path: PathBuf,
}

#[derive(Deserialize)]
struct HoldRequest {
    #[serde(default)]
    device: String,
    project: String,
}

#[derive(Deserialize)]
struct PairRequest {
    code: String,
    name: String,
    kind: DeviceKind,
}

#[derive(Deserialize)]
struct Rename {
    name: String,
}

#[derive(Serialize)]
struct Paired {
    device: PairedDevice,
    token: String,
}

#[derive(Serialize)]
struct Code {
    /// As people read it: `ABCD-EFGH`.
    code: String,
    /// Seconds it is good for.
    expires_in: u64,
}

#[derive(Serialize)]
struct Me {
    owner: bool,
    device: Option<PairedDevice>,
    /// A hash of the served `index.html`: a page that sees it change after
    /// a reconnect loads again (the host now serves another build).
    build: Option<String>,
}

pub fn router(cfg: ApiConfig) -> Router {
    let state = ApiState {
        host: cfg.host,
        access: cfg.access,
        listener: cfg.listener,
        stop: cfg.stop,
        ui_dir: cfg.ui_dir.map(Arc::new),
    };
    Router::new()
        .route("/host/status", get(status))
        .route("/host/projects", get(projects).post(add_project))
        .route("/host/holds", post(hold).delete(release))
        .route("/host/shutdown", post(shutdown))
        .route("/host/pairing", post(pairing))
        .route("/host/pair", post(pair))
        .route("/host/devices", get(devices))
        .route(
            "/host/devices/{id}",
            patch(rename_device).delete(remove_device),
        )
        .route("/host/me", get(me).delete(forget_me))
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
pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

fn unauthorized() -> Response {
    refuse(StatusCode::UNAUTHORIZED, "missing or wrong token")
}

fn caller(s: &ApiState, headers: &HeaderMap) -> Result<Caller, Box<Response>> {
    s.access
        .caller(s.listener, bearer(headers))
        .ok_or_else(|| Box::new(unauthorized()))
}

fn owner(s: &ApiState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    match caller(s, headers)? {
        Caller::Owner => Ok(()),
        Caller::Device(_) => Err(Box::new(refuse(
            StatusCode::FORBIDDEN,
            "only the host's owner may do that",
        ))),
    }
}

/// The JSON body, read after the caller is known (an extractor would answer
/// a bad body before a missing token).
fn body<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, Box<Response>> {
    serde_json::from_slice(bytes)
        .map_err(|e| Box::new(refuse(StatusCode::BAD_REQUEST, &e.to_string())))
}

macro_rules! check {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(refused) => return *refused,
        }
    };
}

async fn status(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    check!(caller(&s, &headers));
    Json(Status {
        version: crate::VERSION,
        pid: std::process::id(),
        projects: s.host.projects(),
        connected: s.access.tunnels.connected(),
        public: s
            .access
            .public
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    })
    .into_response()
}

/// A project as a caller sees it: with whether the caller holds it.
#[derive(Serialize)]
struct ProjectForCaller {
    #[serde(flatten)]
    view: ProjectView,
    held_by_you: bool,
}

async fn projects(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    let who = check!(caller(&s, &headers));
    let holds = s.host.holds();
    let mine = |project: &str| match &who {
        Caller::Owner => false,
        Caller::Device(d) => holds
            .iter()
            .any(|h| h.project == project && h.device == d.id.as_str()),
    };
    let projects: Vec<ProjectForCaller> = s
        .host
        .projects()
        .into_iter()
        .map(|view| ProjectForCaller {
            held_by_you: mine(&view.id),
            view,
        })
        .collect();
    Json(projects).into_response()
}

async fn add_project(
    State(s): State<ApiState>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Response {
    check!(owner(&s, &headers));
    let req: AddProject = check!(body(&bytes));
    match s.host.registry().add(&req.path) {
        Ok(project) => Json(project).into_response(),
        Err(
            e @ (crate::registry::RegistryError::NotAProject(_)
            | crate::registry::RegistryError::Unreadable(..)),
        ) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn shutdown(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    check!(owner(&s, &headers));
    tracing::info!("a client asked the host to stop");
    s.stop.notify_one();
    StatusCode::ACCEPTED.into_response()
}

/// The device a hold is for: a device's own, whatever it named; the owner's
/// as it named it.
fn hold_device(who: &Caller, named: String) -> String {
    match who {
        Caller::Owner => named,
        Caller::Device(d) => d.id.to_string(),
    }
}

async fn hold(State(s): State<ApiState>, headers: HeaderMap, bytes: axum::body::Bytes) -> Response {
    let who = check!(caller(&s, &headers));
    let req: HoldRequest = check!(body(&bytes));
    answer(
        s.host
            .add_hold(&hold_device(&who, req.device), &req.project),
    )
}

async fn release(
    State(s): State<ApiState>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Response {
    let who = check!(caller(&s, &headers));
    let req: HoldRequest = check!(body(&bytes));
    answer(
        s.host
            .release_hold(&hold_device(&who, req.device), &req.project),
    )
}

async fn pairing(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    check!(owner(&s, &headers));
    let now = Instant::now();
    let (code, expires) = s
        .access
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .new_code(now);
    tracing::info!("a pairing code was made");
    Json(Code {
        code: show_code(&code),
        expires_in: expires.duration_since(now).as_secs(),
    })
    .into_response()
}

/// Whether `headers` say a JSON body and, if they name an origin, this
/// host's own.
fn pair_request_ok(headers: &HeaderMap) -> bool {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|t| t.trim() == "application/json")
        });
    let same_origin = match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        None => true,
        Some(origin) => {
            let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
            let named = origin.split_once("://").map(|(_, rest)| rest);
            host.is_some_and(|h| named == Some(h))
        }
    };
    json && same_origin
}

async fn pair(State(s): State<ApiState>, headers: HeaderMap, bytes: axum::body::Bytes) -> Response {
    if !pair_request_ok(&headers) {
        return refuse(
            StatusCode::FORBIDDEN,
            "pairing takes JSON from this host's own page",
        );
    }
    let Ok(req) = serde_json::from_slice::<PairRequest>(&bytes) else {
        return refuse(
            StatusCode::BAD_REQUEST,
            "pairing needs { code, name, kind }",
        );
    };
    // Each wrong code costs its own wait; nothing is held meanwhile, so a
    // flood of them delays nobody else (600 tries a code's ten minutes per
    // connection, against 31^8 codes).
    let good = s
        .access
        .pairing
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take(&req.code, Instant::now());
    if !good {
        tokio::time::sleep(s.access.wrong_code_wait).await;
        return refuse(
            StatusCode::UNAUTHORIZED,
            "that code is wrong or has expired; ask for a new one",
        );
    }
    match s.access.devices.pair(&req.name, req.kind) {
        Ok((device, token)) => {
            tracing::info!(device = %device.id, name = %device.name, "a device paired");
            Json(Paired { device, token }).into_response()
        }
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn devices(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    check!(owner(&s, &headers));
    Json(s.access.devices.list()).into_response()
}

fn device_id(raw: &str) -> Result<DeviceId, Box<Response>> {
    DeviceId::parse(raw).map_err(|_| Box::new(refuse(StatusCode::NOT_FOUND, "no such device")))
}

fn device_answer(result: Result<PairedDevice, DevicesError>) -> Response {
    match result {
        Ok(device) => Json(device).into_response(),
        Err(e @ DevicesError::Unknown(_)) => refuse(StatusCode::NOT_FOUND, &e.to_string()),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn rename_device(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    bytes: axum::body::Bytes,
) -> Response {
    check!(owner(&s, &headers));
    let req: Rename = check!(body(&bytes));
    let id = check!(device_id(&id));
    device_answer(s.access.devices.rename(&id, &req.name))
}

async fn remove_device(
    State(s): State<ApiState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    check!(owner(&s, &headers));
    let id = check!(device_id(&id));
    device_answer(s.access.remove_device(&s.host, &id))
}

async fn me(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    let who = check!(caller(&s, &headers));
    let build = match &s.ui_dir {
        Some(dir) => crate::ui::build_id(dir).await,
        None => None,
    };
    let (owner, device) = match who {
        Caller::Owner => (true, None),
        Caller::Device(d) => (false, Some(d)),
    };
    Json(Me {
        owner,
        device,
        build,
    })
    .into_response()
}

async fn forget_me(State(s): State<ApiState>, headers: HeaderMap) -> Response {
    match check!(caller(&s, &headers)) {
        Caller::Owner => refuse(StatusCode::BAD_REQUEST, "the owner is no paired device"),
        Caller::Device(d) => device_answer(s.access.remove_device(&s.host, &d.id)),
    }
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
    use std::time::Instant;

    const TOKEN: &str = "host-secret";

    struct Api {
        url: String,
        public: String,
        access: Arc<Access>,
        stop: Arc<tokio::sync::Notify>,
    }

    /// The API on a loopback listener and a public one, sharing one
    /// `Access` (as `serve` runs them).
    async fn api(fx: &Fixture) -> Api {
        let access = Access::new(
            TOKEN.into(),
            crate::devices::Devices::open(&fx.home()).unwrap(),
            std::time::Duration::from_millis(200),
        );
        let stop = Arc::new(tokio::sync::Notify::new());
        let mut urls = Vec::new();
        for listener in [Listener::Loopback, Listener::Public] {
            let tcp = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            urls.push(format!("http://{}", tcp.local_addr().unwrap()));
            let app = router(ApiConfig {
                host: fx.host.clone(),
                access: access.clone(),
                listener,
                stop: stop.clone(),
                ui_dir: None,
            });
            tokio::spawn(async move { axum::serve(tcp, app).await });
        }
        Api {
            public: urls.pop().unwrap(),
            url: urls.pop().unwrap(),
            access,
            stop,
        }
    }

    async fn serve(fx: &Fixture) -> String {
        api(fx).await.url
    }

    async fn code(api: &Api) -> String {
        let code: serde_json::Value = reqwest::Client::new()
            .post(format!("{}/host/pairing", api.url))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        code["code"].as_str().unwrap().to_string()
    }

    async fn pair(url: &str, code: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{url}/host/pair"))
            .json(&json!({ "code": code, "name": "Tab S9", "kind": "tablet" }))
            .send()
            .await
            .unwrap()
    }

    /// A tablet pairs over the public listener with a code the owner made,
    /// and its token then works there; the owner's does not.
    #[tokio::test]
    async fn a_device_pairs_with_the_owners_code_and_is_known_by_its_token() {
        let fx = Fixture::new(RUNS);
        let api = api(&fx).await;
        let code = code(&api).await;
        assert_eq!(code.len(), 9, "ABCD-EFGH: {code}");
        let paired: serde_json::Value = pair(&api.public, &code).await.json().await.unwrap();
        let token = paired["token"].as_str().unwrap();
        assert!(paired["device"]["id"].as_str().unwrap().starts_with("dev-"));
        assert_eq!(paired["device"]["name"], "Tab S9");

        let http = reqwest::Client::new();
        let me: serde_json::Value = http
            .get(format!("{}/host/me", api.public))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(me["owner"], false);
        assert_eq!(me["device"]["id"], paired["device"]["id"]);
        let owner_there = http
            .get(format!("{}/host/status", api.public))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(
            owner_there.status(),
            401,
            "the owner token is for loopback only"
        );
        assert_eq!(
            pair(&api.public, &code).await.status(),
            401,
            "a code is good once"
        );
    }

    #[tokio::test]
    async fn a_device_may_not_do_what_only_the_owner_may() {
        let fx = Fixture::new(RUNS);
        let api = api(&fx).await;
        let paired: serde_json::Value = pair(&api.public, &code(&api).await)
            .await
            .json()
            .await
            .unwrap();
        let token = paired["token"].as_str().unwrap().to_string();
        let http = reqwest::Client::new();
        for (method, path) in [
            (reqwest::Method::POST, "/host/pairing"),
            (reqwest::Method::GET, "/host/devices"),
            (reqwest::Method::POST, "/host/shutdown"),
        ] {
            let answer = http
                .request(method.clone(), format!("{}{path}", api.public))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap();
            assert_eq!(answer.status(), 403, "{method} {path}");
        }
        let add = http
            .post(format!("{}/host/projects", api.public))
            .bearer_auth(&token)
            .json(&json!({ "path": "/" }))
            .send()
            .await
            .unwrap();
        assert_eq!(add.status(), 403);
    }

    /// A device's project list says which it holds itself (its home page
    /// offers to close those).
    #[tokio::test]
    async fn a_device_sees_which_projects_it_holds() {
        let fx = Fixture::new(RUNS);
        let alpha = fx.project("alpha");
        fx.project("beta");
        let api = api(&fx).await;
        let paired: serde_json::Value = pair(&api.public, &code(&api).await)
            .await
            .json()
            .await
            .unwrap();
        let token = paired["token"].as_str().unwrap();
        let http = reqwest::Client::new();
        http.post(format!("{}/host/holds", api.public))
            .bearer_auth(token)
            .json(&json!({ "project": alpha }))
            .send()
            .await
            .unwrap();
        fx.host.add_hold("dev-mac", &fx.project("beta")).unwrap();
        let projects: serde_json::Value = http
            .get(format!("{}/host/projects", api.public))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let held = |name: &str| {
            projects
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap()["held_by_you"]
                .clone()
        };
        assert_eq!(held("alpha"), true);
        assert_eq!(held("beta"), false, "the Mac holds it, not this device");
    }

    /// A device's hold is its own, whatever device it names.
    #[tokio::test]
    async fn a_devices_hold_is_always_its_own() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        let api = api(&fx).await;
        let paired: serde_json::Value = pair(&api.public, &code(&api).await)
            .await
            .json()
            .await
            .unwrap();
        let token = paired["token"].as_str().unwrap();
        reqwest::Client::new()
            .post(format!("{}/host/holds", api.public))
            .bearer_auth(token)
            .json(&json!({ "device": "dev-the-mac", "project": id }))
            .send()
            .await
            .unwrap();
        let holds = fx.host.holds();
        assert_eq!(holds.len(), 1);
        assert_eq!(holds[0].device, paired["device"]["id"].as_str().unwrap());
    }

    /// Removing a device revokes its token and releases its holds; a device
    /// may forget itself.
    #[tokio::test]
    async fn a_removed_device_is_let_go_of() {
        let fx = Fixture::new(RUNS);
        let id = fx.project("alpha");
        let api = api(&fx).await;
        let http = reqwest::Client::new();
        let mut tokens = Vec::new();
        for _ in 0..2 {
            let paired: serde_json::Value = pair(&api.public, &code(&api).await)
                .await
                .json()
                .await
                .unwrap();
            tokens.push((
                paired["device"]["id"].as_str().unwrap().to_string(),
                paired["token"].as_str().unwrap().to_string(),
            ));
        }
        let (first_id, first_token) = &tokens[0];
        let (_, second_token) = &tokens[1];
        http.post(format!("{}/host/holds", api.public))
            .bearer_auth(first_token)
            .json(&json!({ "project": id }))
            .send()
            .await
            .unwrap();
        let removed = http
            .delete(format!("{}/host/devices/{first_id}", api.url))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(removed.status(), 200);
        assert!(fx.host.holds().is_empty(), "its hold went");
        let gone = http
            .get(format!("{}/host/me", api.public))
            .bearer_auth(first_token)
            .send()
            .await
            .unwrap();
        assert_eq!(gone.status(), 401);

        let forgot = http
            .delete(format!("{}/host/me", api.public))
            .bearer_auth(second_token)
            .send()
            .await
            .unwrap();
        assert_eq!(forgot.status(), 200);
        assert!(api.access.devices.list().is_empty());
    }

    /// A page on another site cannot pair a device: pairing takes JSON (a
    /// cross-site form cannot send it, and nothing here answers a
    /// preflight) and refuses a foreign Origin.
    #[tokio::test]
    async fn pairing_takes_json_from_the_hosts_own_page_only() {
        let fx = Fixture::new(RUNS);
        let api = api(&fx).await;
        let code = code(&api).await;
        let body = json!({ "code": code, "name": "x", "kind": "tablet" }).to_string();
        let http = reqwest::Client::new();
        let form = http
            .post(format!("{}/host/pair", api.public))
            .header("content-type", "text/plain")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(form.status(), 403);
        let foreign = http
            .post(format!("{}/host/pair", api.public))
            .header("content-type", "application/json")
            .header("origin", "https://evil.example")
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403);
        let own_origin = format!("http://{}", api.public.trim_start_matches("http://"));
        let own = http
            .post(format!("{}/host/pair", api.public))
            .header("content-type", "application/json")
            .header("origin", own_origin)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(own.status(), 200, "the code was still good");
    }

    /// A wrong code costs a wait, each its own: a flood of wrong codes holds
    /// up neither another device's attempt nor the owner making a code.
    #[tokio::test]
    async fn a_wrong_code_costs_a_wait() {
        let fx = Fixture::new(RUNS);
        let api = api(&fx).await;
        let started = Instant::now();
        let (a, b, made) = tokio::join!(
            pair(&api.public, "AAAA-AAAA"),
            pair(&api.public, "BBBB-BBBB"),
            async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let asked = Instant::now();
                code(&api).await;
                asked.elapsed()
            }
        );
        assert!(
            made < std::time::Duration::from_millis(150),
            "the owner waited {made:?}"
        );
        assert_eq!(
            (a.status(), b.status()),
            (
                reqwest::StatusCode::UNAUTHORIZED,
                reqwest::StatusCode::UNAUTHORIZED
            )
        );
        let took = started.elapsed();
        assert!(
            took >= std::time::Duration::from_millis(200),
            "a wrong code waits: {took:?}"
        );
        assert!(
            took < std::time::Duration::from_millis(390),
            "two wrong codes wait side by side, not in turn: {took:?}"
        );
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
            (reqwest::Method::POST, "/host/pairing"),
            (reqwest::Method::GET, "/host/devices"),
            (reqwest::Method::GET, "/host/me"),
            (reqwest::Method::DELETE, "/host/me"),
            (
                reqwest::Method::DELETE,
                "/host/devices/dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427",
            ),
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
        let api = api(&fx).await;
        let (url, stop) = (api.url.clone(), api.stop.clone());
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
