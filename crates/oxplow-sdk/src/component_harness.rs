//! The component harness (`.context/extensions.md` "Testing a
//! component"): a fixture's `ui:` steps run a custom component's own code
//! against the test's oxplow, in the app's own webview, with no app around
//! it.
//!
//! A loopback server serves the harness's page (`harness-host.js`, built
//! from the app's bridge), the component's bundle exactly as the daemon
//! serves it (`oxplow_app::component_http`) but with the harness's agent
//! put first in its page, and the client library. The shell opens the
//! page (`oxplow component-harness <url>`) in a window of its own. What
//! the frame asks of the core (`invoke`, `querySql`, forms, `showView`)
//! runs on the test's oxplow; what it asks of the shell (a title,
//! actions, the view, navigation) is recorded. The test drives it one op
//! at a time ([`Harness::op`]): a DOM step for the agent, a nav-bar press,
//! a view change, new rows.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use oxplow_app::component_http::{self, Served};
use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

/// The harness's page script (`bun run build:harness`).
const HOST_JS: &str = include_str!("../assets/harness-host.js");
/// What the harness puts first in a bundle's page.
const AGENT_JS: &str = include_str!("../assets/harness-agent.js");
/// Its name beside the client library, which the bundle's CSP allows.
const AGENT_FILE: &str = "oxplow-harness-agent.js";

const PAGE: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>component harness</title></head>\
<body style=\"margin:0\"><iframe id=\"frame\" sandbox=\"allow-scripts allow-forms\" referrerpolicy=\"no-referrer\" \
style=\"border:0;width:100%;height:100vh\"></iframe><script src=\"/harness/host.js\"></script></body></html>";

/// The oxplow app that draws the harness: `OXPLOW_SHELL`, else an `oxplow`
/// beside the running program or up to two folders above it (an installed
/// app keeps its daemon beside it; a build its tests under `deps/`).
pub(crate) fn shell() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OXPLOW_SHELL") {
        return Some(PathBuf::from(path));
    }
    let name = format!("oxplow{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .skip(1)
        .take(3)
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
}

/// What the frame's host is answered from, and what it was asked.
#[derive(Default)]
struct Shared {
    /// What only the shell answers, as asked: `(kind, value)`.
    records: Vec<(String, Value)>,
    /// The answers to the next confirmations (none left: yes).
    confirms: VecDeque<bool>,
    /// The values the next form is submitted with (`null`: closed).
    form: Option<Value>,
    /// A command ran since the test last looked: its page should run again.
    acted: bool,
    /// Who waits for the last op's result.
    waiting: Option<oneshot::Sender<Value>>,
}

struct Ctx {
    svc: Arc<oxplow_app::Services>,
    root: PathBuf,
    /// `<extension>/<component id>`.
    component: String,
    version: String,
    start: Value,
    shared: Mutex<Shared>,
    ops: tokio::sync::Mutex<mpsc::Receiver<(Value, oneshot::Sender<Value>)>>,
}

/// A running harness: its server, and the shell's window on it.
pub(crate) struct Harness {
    ctx: Arc<Ctx>,
    ops: mpsc::Sender<(Value, oneshot::Sender<Value>)>,
    server: tokio::task::JoinHandle<()>,
    _window: tokio::process::Child,
}

impl Harness {
    /// Serve component `component` (loaded at `version` on `svc`, under
    /// `root`) mounted as `start` says, and open `shell`'s window on it.
    pub(crate) async fn open(
        shell: &Path,
        svc: Arc<oxplow_app::Services>,
        root: PathBuf,
        component: String,
        version: String,
        start: Value,
    ) -> Result<Harness, String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let (ops, rx) = mpsc::channel(1);
        let ctx = Arc::new(Ctx {
            svc,
            root,
            component,
            version,
            start,
            shared: Mutex::default(),
            ops: tokio::sync::Mutex::new(rx),
        });
        let app = Router::new()
            .route(
                "/harness/",
                get(|| async { page(PAGE, "text/html; charset=utf-8") }),
            )
            .route(
                "/harness/host.js",
                get(|| async { page(HOST_JS, "text/javascript; charset=utf-8") }),
            )
            .route("/harness/done", get(|| async { "done" }))
            .route("/harness/start", get(start_of))
            .route("/harness/record", post(record))
            .route("/harness/next", post(next))
            .route("/harness/rpc", post(rpc))
            .route("/components/v/{version}/", get(bundle_index))
            .route("/components/v/{version}/{*path}", get(bundle_file))
            .route("/component-lib/{file}", get(library))
            .with_state(ctx.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let window = tokio::process::Command::new(shell)
            .arg("component-harness")
            .arg(format!("http://127.0.0.1:{port}/harness/"))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| format!("the oxplow app at {} didn't start: {e}", shell.display()))?;
        Ok(Harness {
            ctx,
            ops,
            server,
            _window: window,
        })
    }

    /// Ask the host to do `op` (`{ op: dom | press | viewParams | update |
    /// done, … }`): what it answered, or why not within `wait`.
    pub(crate) async fn op(&self, op: Value, wait: Duration) -> Result<Value, String> {
        let (tx, rx) = oneshot::channel();
        tokio::time::timeout(wait, self.ops.send((op, tx)))
            .await
            .map_err(|_| "the harness's page didn't ask for its next step".to_string())?
            .map_err(|_| "the harness's page went away".to_string())?;
        tokio::time::timeout(wait, rx)
            .await
            .map_err(|_| "the harness's page didn't answer".to_string())?
            .map_err(|_| "the harness's page went away".to_string())
    }

    /// Whether the window draws (its page asks first, before it mounts
    /// anything): `None` when its page didn't say within `wait`.
    pub(crate) async fn drawing(&self, wait: Duration) -> Option<bool> {
        let until = tokio::time::Instant::now() + wait;
        loop {
            let said = self
                .shared()
                .records
                .iter()
                .find(|(kind, _)| kind == "drawing")
                .and_then(|(_, value)| value.as_bool());
            if said.is_some() || tokio::time::Instant::now() >= until {
                return said;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// What only the shell answers, as asked so far.
    pub(crate) fn records(&self) -> Vec<(String, Value)> {
        self.shared().records.clone()
    }

    /// The next confirmation's answer.
    pub(crate) fn confirm_next(&self, yes: bool) {
        self.shared().confirms.push_back(yes);
    }

    /// The next form's submission: its values, or `None` to close it.
    pub(crate) fn submit_next(&self, values: Option<Value>) {
        self.shared().form = Some(values.unwrap_or(Value::Null));
    }

    /// Whether a command ran since the last time this was asked.
    pub(crate) fn take_acted(&self) -> bool {
        std::mem::take(&mut self.shared().acted)
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.ctx.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// End it: the page is told it's done, the server stops, the window
    /// closes.
    pub(crate) async fn close(self) {
        let _ = self
            .op(json!({ "op": "done" }), Duration::from_secs(2))
            .await;
        self.server.abort();
    }
}

fn page(body: &'static str, content_type: &'static str) -> Response {
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response
}

fn respond(served: Served) -> Response {
    let mut response = served.body.into_response();
    let h = response.headers_mut();
    if let Ok(v) = HeaderValue::from_str(served.content_type) {
        h.insert(header::CONTENT_TYPE, v);
    }
    for (name, value) in served.headers {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            h.insert(name, value);
        }
    }
    response
}

fn host_of(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::HOST)?
        .to_str()
        .ok()
        .and_then(component_http::loopback_host)
}

async fn start_of(State(ctx): State<Arc<Ctx>>) -> Json<Value> {
    Json(ctx.start.clone())
}

async fn record(State(ctx): State<Arc<Ctx>>, Json(body): Json<Value>) -> StatusCode {
    let kind = body["kind"].as_str().unwrap_or_default().to_string();
    let mut shared = ctx.shared.lock().unwrap_or_else(|e| e.into_inner());
    shared.records.push((kind, body["value"].clone()));
    StatusCode::NO_CONTENT
}

/// The last op's result for whoever waits on it, then the next op.
async fn next(State(ctx): State<Arc<Ctx>>, Json(body): Json<Value>) -> Json<Value> {
    let waiting = ctx
        .shared
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .waiting
        .take();
    if let Some(tx) = waiting {
        let _ = tx.send(body["result"].clone());
    }
    match ctx.ops.lock().await.recv().await {
        Some((op, tx)) => {
            ctx.shared.lock().unwrap_or_else(|e| e.into_inner()).waiting = Some(tx);
            Json(op)
        }
        None => Json(json!({ "op": "done" })),
    }
}

/// A command's refusal as the app's IPC reports it to the frame's host.
fn refusal(e: &CommandError) -> Value {
    let (code, field) = match e {
        CommandError::Unknown { .. } => ("NOT_FOUND", None),
        CommandError::Invalid { field, .. } => ("INVALID", field.clone()),
        CommandError::Denied { .. } => ("DENIED", None),
        CommandError::NeedsConfirmation { .. } => ("NEEDS_CONFIRMATION", None),
        CommandError::Proposed { .. } => ("PROPOSED", None),
        CommandError::Failed { .. } => ("INTERNAL", None),
        CommandError::Unavailable { .. } => ("UNAVAILABLE", None),
        CommandError::Busy { .. } => ("BUSY", None),
    };
    json!({ "ok": false, "error": { "code": code, "message": e.to_string(), "field": field } })
}

/// Run a declared command as the component, for the person — what the
/// frame invokes, or a form it opened submits — and keep it for the test.
async fn invoke(ctx: &Ctx, command: &str, input: Value, confirmed: bool) -> Value {
    let out = oxplow_app::viz_actions::invoke_component_command(
        &ctx.svc,
        &ctx.root,
        oxplow_app::viz_actions::ComponentInvoke {
            component: ctx.component.clone(),
            command: command.to_string(),
            input: input.clone(),
            on_behalf_of: Actor::Human,
            confirmed,
            version: ctx.version.clone(),
        },
    )
    .await;
    // One that asks first isn't run yet: the host asks, then runs it again.
    let asked = matches!(out, Err(CommandError::NeedsConfirmation { .. }));
    let (answer, kept) = match out {
        Ok(out) => (
            json!({ "ok": true, "result": out.result }),
            json!({ "command": command, "input": input, "ok": true, "result": out.result }),
        ),
        Err(e) => (
            refusal(&e),
            json!({ "command": command, "input": input, "ok": false, "refused": e.to_string() }),
        ),
    };
    if !asked {
        let mut shared = ctx.shared.lock().unwrap_or_else(|e| e.into_inner());
        shared.records.push(("invoked".into(), kept));
        shared.acted = true;
    }
    answer
}

/// What the frame asks the core (or what the test answers for the person).
async fn rpc(State(ctx): State<Arc<Ctx>>, Json(body): Json<Value>) -> Json<Value> {
    let ok = |result: Value| Json(json!({ "ok": true, "result": result }));
    match body["method"].as_str().unwrap_or_default() {
        "invoke" => Json(
            invoke(
                &ctx,
                body["command"].as_str().unwrap_or_default(),
                body["input"].clone(),
                body["confirmed"].as_bool().unwrap_or(false),
            )
            .await,
        ),
        "confirm" => {
            let mut shared = ctx.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared
                .records
                .push(("confirm".into(), body["command"].clone()));
            ok(json!(shared.confirms.pop_front().unwrap_or(true)))
        }
        "querySql" => {
            let params = serde_json::from_value(body["params"].clone()).unwrap_or_default();
            let out = oxplow_app::viz_actions::run_component_sql(
                &ctx.svc,
                &ctx.root,
                oxplow_app::viz_actions::ComponentSql {
                    component: ctx.component.clone(),
                    sql: body["sql"].as_str().unwrap_or_default().to_string(),
                    params,
                    stream: None,
                    limit: None,
                    on_behalf_of: Actor::Human,
                    version: ctx.version.clone(),
                },
            )
            .await;
            match out {
                Ok(result) => ok(json!(result)),
                Err(e) => Json(refusal(&e)),
            }
        }
        "openForm" => {
            let form = {
                let mut shared = ctx.shared.lock().unwrap_or_else(|e| e.into_inner());
                shared.records.push((
                    "openForm".into(),
                    json!({ "command": body["command"], "input": body["input"] }),
                ));
                shared.form.take()
            };
            match form {
                Some(Value::Object(values)) => {
                    let mut input = body["input"].as_object().cloned().unwrap_or_default();
                    input.extend(values);
                    let command = body["command"].as_str().unwrap_or_default();
                    let out = invoke(&ctx, command, Value::Object(input), true).await;
                    match out["ok"].as_bool() {
                        Some(true) => ok(json!({ "result": out["result"] })),
                        _ => Json(out),
                    }
                }
                // Closed: as the person closing it.
                _ => ok(Value::Null),
            }
        }
        "askForm" => {
            let mut shared = ctx.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared
                .records
                .push(("askForm".into(), body["form"].clone()));
            ok(match shared.form.take() {
                Some(values @ Value::Object(_)) => values,
                _ => Value::Null,
            })
        }
        "showView" => Json(show_view(&ctx, body["spec"].as_str().unwrap_or_default()).await),
        other => Json(json!({ "ok": false, "error": {
            "code": "INVALID",
            "message": format!("the harness answers no `{other}`"),
        } })),
    }
}

/// Run inline page `spec` as the page under the frame would: its first
/// loader's rows, or why it's refused.
async fn show_view(ctx: &Ctx, spec: &str) -> Value {
    use oxplow_app::pages::runner::{PageRunner, RunRequest, RunTarget};
    let invalid = |message: String| json!({ "ok": false, "error": { "code": "INVALID", "message": message } });
    let page = match oxplow_app::pages::schema::parse_inline_page(spec) {
        Ok(page) => page,
        Err(e) => return invalid(e),
    };
    let run = PageRunner::of(&ctx.svc)
        .run(RunRequest {
            target: RunTarget::Spec(Arc::new(page)),
            params: Default::default(),
            ctx: Default::default(),
            viewer: Actor::Human,
        })
        .await;
    match run {
        Ok(run) => match run.loaders.iter().next() {
            Some((name, loader)) => match &loader.error {
                Some(e) => json!({ "ok": false, "error": {
                    "code": "INVALID", "message": e, "field": format!("/loaders/{name}/sql"),
                } }),
                None => json!({ "ok": true, "result": {
                    "columns": loader.result.columns,
                    "rows": loader.result.rows,
                    "truncated": loader.result.truncated,
                } }),
            },
            None => json!({ "ok": true, "result": null }),
        },
        Err(e) => invalid(e.to_string()),
    }
}

async fn bundle_index(
    State(ctx): State<Arc<Ctx>>,
    AxumPath(version): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    serve(&ctx, &version, "", &headers).await
}

async fn bundle_file(
    State(ctx): State<Arc<Ctx>>,
    AxumPath((version, path)): AxumPath<(String, String)>,
    headers: HeaderMap,
) -> Response {
    serve(&ctx, &version, &path, &headers).await
}

/// A file of the bundle, as the daemon serves it — its page with the
/// agent put first, so it hears the test from the start.
async fn serve(ctx: &Ctx, version: &str, path: &str, headers: &HeaderMap) -> Response {
    let Some(host) = host_of(headers) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(mut served) =
        component_http::bundle_file(&ctx.svc.component_bundles, host, version, path).await
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if path.is_empty() || path == "index.html" {
        served.body = with_agent(&String::from_utf8_lossy(&served.body)).into_bytes();
    }
    respond(served)
}

/// Page `html` with the agent's script first: in its `<head>`, else
/// before everything.
fn with_agent(html: &str) -> String {
    let tag = format!(
        "<script src=\"{}{AGENT_FILE}\"></script>",
        component_http::LIB_PATH
    );
    match html.to_ascii_lowercase().find("<head>") {
        Some(at) => format!("{}{tag}{}", &html[..at + 6], &html[at + 6..]),
        None => format!("{tag}{html}"),
    }
}

async fn library(AxumPath(file): AxumPath<String>, headers: HeaderMap) -> Response {
    if host_of(&headers).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if file == AGENT_FILE {
        return page(AGENT_JS, "text/javascript; charset=utf-8");
    }
    match component_http::library(&file) {
        Some(served) => respond(served),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agent_goes_first_in_the_bundles_page() {
        assert_eq!(
            with_agent("<!doctype html><html><head><title>x</title></head><body></body></html>"),
            "<!doctype html><html><head><script src=\"/component-lib/oxplow-harness-agent.js\"></script><title>x</title></head><body></body></html>"
        );
        assert_eq!(
            with_agent("<div></div>"),
            "<script src=\"/component-lib/oxplow-harness-agent.js\"></script><div></div>"
        );
    }
}
