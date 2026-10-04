//! A stand-in for Linear's GraphQL API, for the provider's tests and the
//! conformance kit: an HTTP server on localhost that answers the
//! operations this provider sends (by `operationName`) over an in-memory
//! workspace — one team, `ENG`, with Linear's default workflow plus a
//! `Blocked` state, and one project, `Roadmap`. It logs every request
//! (operation and variables) so a test can pin what a verb sends, checks
//! the API key, and can refuse its next request as rate limited.
//!
//! It models what this provider relies on, not Linear: a request it
//! doesn't know is a GraphQL error naming it. Behaviour checked against
//! a real workspace is a person's (see `.context/providers.md`).

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// One request the simulator answered.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub operation: String,
    pub variables: Value,
}

#[derive(Clone)]
struct Issue {
    id: String,
    identifier: String,
    title: String,
    description: Option<String>,
    state: usize,
    parent: Option<String>,
    project: Option<String>,
    priority: i64,
    updated: u64,
    /// When it was created, by the real clock (RFC 3339) — unlike
    /// `updated`, a counter, so reads are the same every run. Only the
    /// live suite's cleanup asks for it.
    created: String,
    trashed: bool,
}

struct World {
    key: String,
    states: Vec<(String, String, String, f64)>,
    issues: Vec<Issue>,
    /// `(id, issue, related, type)`.
    relations: Vec<(String, String, String, String)>,
    /// `(id, issue, body)`.
    comments: Vec<(String, String, String)>,
    /// Ids handed out so far (a create without its own id gets the next).
    minted: usize,
    clock: u64,
    max_page: usize,
    rate_limit_secs: Option<u64>,
    /// Refuse the next request of this operation as rate limited.
    rate_limit_op: Option<(String, u64)>,
    requests: Vec<Request>,
    /// Issues answered with this state type instead of their own (a
    /// workflow the provider can't map).
    odd_state_types: std::collections::HashMap<String, String>,
}

pub const TEAM_ID: &str = "team-eng";
pub const PROJECT_ID: &str = "project-roadmap";

/// A running simulator; it stops when dropped.
pub struct LinearSim {
    /// Its GraphQL endpoint (`http://localhost:<port>/graphql`).
    pub url: String,
    world: Arc<Mutex<World>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for LinearSim {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl LinearSim {
    /// Start one accepting `key` as its API key.
    pub async fn start(key: &str) -> std::io::Result<LinearSim> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let states = [
            ("Triage", "triage"),
            ("Backlog", "backlog"),
            ("Todo", "unstarted"),
            ("In Progress", "started"),
            ("Blocked", "started"),
            ("Done", "completed"),
            ("Canceled", "canceled"),
            ("Duplicate", "canceled"),
        ]
        .iter()
        .enumerate()
        .map(|(i, (name, kind))| {
            (
                format!("state-{i}"),
                name.to_string(),
                kind.to_string(),
                i as f64,
            )
        })
        .collect();
        let world = Arc::new(Mutex::new(World {
            key: key.to_string(),
            states,
            issues: Vec::new(),
            relations: Vec::new(),
            comments: Vec::new(),
            minted: 0,
            clock: 0,
            max_page: 50,
            rate_limit_secs: None,
            rate_limit_op: None,
            requests: Vec::new(),
            odd_state_types: std::collections::HashMap::new(),
        }));
        let served = world.clone();
        let task = tokio::spawn(async move {
            while let Ok((conn, _)) = listener.accept().await {
                let world = served.clone();
                tokio::spawn(async move {
                    let _ = serve(conn, world).await;
                });
            }
        });
        Ok(LinearSim {
            url: format!("http://localhost:{port}/graphql"),
            world,
            task,
        })
    }

    /// Every request answered so far, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.lock().requests.clone()
    }

    /// Forget the requests answered so far.
    pub fn clear_requests(&self) {
        self.lock().requests.clear();
    }

    /// Answer at most `n` issues a page, whatever `first` asks.
    pub fn set_max_page(&self, n: usize) {
        self.lock().max_page = n;
    }

    /// Refuse the next request as rate limited, retrying after `secs`.
    pub fn rate_limit_next(&self, secs: u64) {
        self.lock().rate_limit_secs = Some(secs);
    }

    /// Refuse the next `operation` request (`Issue`) as rate limited,
    /// retrying after `secs`; other requests go through.
    pub fn rate_limit_next_of(&self, operation: &str, secs: u64) {
        self.lock().rate_limit_op = Some((operation.to_string(), secs));
    }

    /// Answer `identifier` with workflow state type `kind`, whatever its
    /// state — a type the provider maps to nothing.
    pub fn set_state_type(&self, identifier: &str, kind: &str) {
        self.lock()
            .odd_state_types
            .insert(identifier.to_string(), kind.to_string());
    }

    /// The comments on `identifier`, oldest first.
    pub fn comments(&self, identifier: &str) -> Vec<String> {
        self.lock()
            .comments
            .iter()
            .filter(|(_, i, _)| i == identifier)
            .map(|(_, _, b)| b.clone())
            .collect()
    }

    /// The issues not in the trash, by identifier.
    pub fn live_issues(&self) -> Vec<String> {
        self.lock()
            .issues
            .iter()
            .filter(|i| !i.trashed)
            .map(|i| i.identifier.clone())
            .collect()
    }

    /// The relations as `(issue, related, type)`.
    pub fn relations(&self) -> Vec<(String, String, String)> {
        self.lock()
            .relations
            .iter()
            .map(|(_, from, to, kind)| (from.clone(), to.clone(), kind.clone()))
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|p| p.into_inner())
    }
}

async fn serve(conn: TcpStream, world: Arc<Mutex<World>>) -> std::io::Result<()> {
    let (read, mut write) = conn.into_split();
    let mut reader = BufReader::new(read);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let mut length = 0usize;
        let mut key = String::new();
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).await?;
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                match name.trim().to_ascii_lowercase().as_str() {
                    "content-length" => length = value.trim().parse().unwrap_or(0),
                    "authorization" => key = value.trim().to_string(),
                    _ => {}
                }
            }
        }
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).await?;
        let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let (status, headers, reply) = {
            let mut w = world.lock().unwrap_or_else(|p| p.into_inner());
            answer(&mut w, &key, &request)
        };
        let text = reply.to_string();
        let mut head = format!(
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
            text.len()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        write.write_all(head.as_bytes()).await?;
        write.write_all(text.as_bytes()).await?;
        write.flush().await?;
    }
}

fn error(code: &str, message: &str) -> Value {
    json!({ "errors": [{ "message": message, "extensions": { "code": code } }] })
}

fn timestamp(clock: u64) -> String {
    format!(
        "2026-01-01T{:02}:{:02}:{:02}.000Z",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

fn answer(w: &mut World, key: &str, request: &Value) -> (u16, Vec<(String, String)>, Value) {
    if key != w.key {
        return (
            400,
            Vec::new(),
            error(
                "AUTHENTICATION_ERROR",
                "Authentication required, not authenticated",
            ),
        );
    }
    let operation = request["operationName"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let vars = request["variables"].clone();
    let of_op = match &w.rate_limit_op {
        Some((name, _)) if *name == operation => w.rate_limit_op.take().map(|(_, s)| s),
        _ => None,
    };
    if let Some(secs) = w.rate_limit_secs.take().or(of_op) {
        return (
            400,
            vec![("Retry-After".into(), secs.to_string())],
            error("RATELIMITED", "Rate limit exceeded"),
        );
    }
    w.requests.push(Request {
        operation: operation.clone(),
        variables: vars.clone(),
    });
    match op(w, &operation, &vars) {
        Ok(data) => (200, Vec::new(), json!({ "data": data })),
        Err(message) => (200, Vec::new(), error("INVALID_INPUT", &message)),
    }
}

fn issue_json(w: &World, i: &Issue) -> Value {
    let (sid, name, kind, _) = &w.states[i.state];
    let kind = w.odd_state_types.get(&i.identifier).unwrap_or(kind);
    json!({
        "id": i.id,
        "identifier": i.identifier,
        "title": i.title,
        "description": i.description,
        "url": format!("https://linear.app/sim/issue/{}", i.identifier),
        "priority": i.priority,
        "updatedAt": timestamp(i.updated),
        "trashed": i.trashed,
        "state": { "id": sid, "name": name, "type": kind },
        "parent": i.parent.as_ref().map(|p| json!({ "identifier": p })),
        "team": { "key": "ENG" },
    })
}

/// An issue by a top-level `id` argument, which Linear accepts as the
/// uuid or the identifier (`ENG-12`).
fn find(w: &World, id: &str) -> Result<usize, String> {
    w.issues
        .iter()
        .position(|i| (i.id == id || i.identifier == id) && !i.trashed)
        .ok_or_else(|| "Entity not found: Issue".to_string())
}

/// An issue by an input object's id field (`parentId`, `issueId`,
/// `relatedIssueId`), which Linear types as a uuid: an identifier there is
/// refused, as Linear refuses it.
fn find_uuid(w: &World, id: &str) -> Result<usize, String> {
    w.issues
        .iter()
        .position(|i| i.id == id && !i.trashed)
        .ok_or_else(|| format!("Argument Validation Error: `{id}` isn't an issue uuid"))
}

fn state_index(w: &World, id: &str) -> Result<usize, String> {
    w.states
        .iter()
        .position(|s| s.0 == id)
        .ok_or_else(|| format!("no workflow state `{id}`"))
}

/// A create's id: the client's own (`input.id`), refused when anything
/// already has it — as Linear refuses a repeated id — else a new one.
fn new_id(w: &mut World, input: &Value) -> Result<String, String> {
    match input["id"].as_str() {
        Some(id) => {
            let taken = w.issues.iter().any(|i| i.id == id)
                || w.comments.iter().any(|c| c.0 == id)
                || w.relations.iter().any(|r| r.0 == id);
            if taken {
                return Err(format!(
                    "Argument Validation Error: `{id}` is already in use"
                ));
            }
            Ok(id.to_string())
        }
        None => {
            w.minted += 1;
            Ok(format!("00000000-0000-4000-8000-{:012}", w.minted))
        }
    }
}

fn tick(w: &mut World) -> u64 {
    w.clock += 1;
    w.clock
}

/// Apply an issue input's fields (`IssueCreateInput` / `IssueUpdateInput`).
fn apply(w: &World, i: &mut Issue, input: &Value) -> Result<(), String> {
    if let Some(t) = input["title"].as_str() {
        i.title = t.to_string();
    }
    if let Some(d) = input.get("description") {
        i.description = d.as_str().map(str::to_string);
    }
    if let Some(p) = input.get("parentId") {
        i.parent = match p.as_str() {
            None => None,
            Some(p) => Some(w.issues[find_uuid(w, p)?].identifier.clone()),
        };
    }
    if let Some(s) = input["stateId"].as_str() {
        i.state = state_index(w, s)?;
    }
    if let Some(p) = input["priority"].as_i64() {
        i.priority = p;
    }
    if let Some(p) = input["projectId"].as_str() {
        i.project = Some(p.to_string());
    }
    Ok(())
}

fn op(w: &mut World, operation: &str, vars: &Value) -> Result<Value, String> {
    match operation {
        "Team" => {
            let teams: Vec<Value> = (vars["key"] == "ENG")
                .then(|| {
                    let states: Vec<Value> = w
                        .states
                        .iter()
                        .map(|(id, name, kind, position)| {
                            json!({ "id": id, "name": name, "type": kind, "position": position })
                        })
                        .collect();
                    json!({ "id": TEAM_ID, "key": "ENG", "states": { "nodes": states } })
                })
                .into_iter()
                .collect();
            Ok(json!({ "teams": { "nodes": teams } }))
        }
        "Project" => {
            let projects: Vec<Value> = (vars["name"] == "Roadmap")
                .then(|| json!({ "id": PROJECT_ID, "name": "Roadmap" }))
                .into_iter()
                .collect();
            Ok(json!({ "projects": { "nodes": projects } }))
        }
        "Issue" => {
            let at = find(w, vars["id"].as_str().unwrap_or_default())?;
            Ok(json!({ "issue": issue_json(w, &w.issues[at]) }))
        }
        "IssueCreate" => {
            let input = &vars["input"];
            if input["teamId"] != TEAM_ID {
                return Err("no team".into());
            }
            let id = new_id(w, input)?;
            let n = w.issues.len() + 1;
            let mut issue = Issue {
                id,
                identifier: format!("ENG-{n}"),
                title: String::new(),
                description: None,
                state: 1,
                parent: None,
                project: None,
                priority: 0,
                updated: 0,
                created: oxplow_domain::Timestamp::now().to_string(),
                trashed: false,
            };
            apply(w, &mut issue, input)?;
            issue.updated = tick(w);
            let out = issue_json(w, &issue);
            w.issues.push(issue);
            Ok(json!({ "issueCreate": { "success": true, "issue": out } }))
        }
        "IssueUpdate" => {
            let at = find(w, vars["id"].as_str().unwrap_or_default())?;
            let mut issue = w.issues[at].clone();
            apply(w, &mut issue, &vars["input"])?;
            issue.updated = tick(w);
            w.issues[at] = issue;
            Ok(json!({ "issueUpdate": { "success": true, "issue": issue_json(w, &w.issues[at]) } }))
        }
        "IssueRelationCreate" => {
            let input = &vars["input"];
            let id = new_id(w, input)?;
            let from = find_uuid(w, input["issueId"].as_str().unwrap_or_default())?;
            let to = find_uuid(w, input["relatedIssueId"].as_str().unwrap_or_default())?;
            let kind = input["type"].as_str().unwrap_or_default().to_string();
            if !["blocks", "related", "duplicate"].contains(&kind.as_str()) {
                return Err(format!("no relation type `{kind}`"));
            }
            let pair = (
                w.issues[from].identifier.clone(),
                w.issues[to].identifier.clone(),
            );
            w.relations.push((id, pair.0, pair.1, kind));
            Ok(json!({ "issueRelationCreate": { "success": true,
                       "issueRelation": { "issue": issue_json(w, &w.issues[from]) } } }))
        }
        "CommentCreate" => {
            let input = &vars["input"];
            let id = new_id(w, input)?;
            let at = find_uuid(w, input["issueId"].as_str().unwrap_or_default())?;
            let body = input["body"].as_str().unwrap_or_default().to_string();
            let identifier = w.issues[at].identifier.clone();
            w.comments.push((id, identifier, body));
            Ok(json!({ "commentCreate": { "success": true,
                       "comment": { "issue": issue_json(w, &w.issues[at]) } } }))
        }
        "Comment" => {
            let id = vars["id"].as_str().unwrap_or_default();
            let comment = w
                .comments
                .iter()
                .find(|c| c.0 == id)
                .ok_or("Entity not found: Comment")?;
            let at = find(w, &comment.1)?;
            Ok(json!({ "comment": { "issue": issue_json(w, &w.issues[at]) } }))
        }
        "IssueRelation" => {
            let id = vars["id"].as_str().unwrap_or_default();
            let relation = w
                .relations
                .iter()
                .find(|r| r.0 == id)
                .ok_or("Entity not found: IssueRelation")?;
            let at = find(w, &relation.1)?;
            Ok(json!({ "issueRelation": { "issue": issue_json(w, &w.issues[at]) } }))
        }
        "IssueDelete" => {
            let at = find(w, vars["id"].as_str().unwrap_or_default())?;
            w.issues[at].trashed = true;
            w.issues[at].updated = tick(w);
            Ok(json!({ "issueDelete": { "success": true } }))
        }
        "Issues" => {
            let filter = &vars["filter"];
            let since = filter["updatedAt"]["gt"].as_str().map(str::to_string);
            let project = filter["project"]["id"]["eq"].as_str().map(str::to_string);
            if filter["team"]["id"]["eq"] != TEAM_ID {
                return Err("the filter names no team".into());
            }
            let mut matching: Vec<&Issue> = w
                .issues
                .iter()
                .filter(|i| {
                    since
                        .as_deref()
                        .is_none_or(|s| timestamp(i.updated).as_str() > s)
                })
                .filter(|i| project.is_none() || i.project == project)
                .collect();
            matching.sort_by_key(|i| i.updated);
            let start = match vars["after"].as_str() {
                None => 0,
                Some(cursor) => matching
                    .iter()
                    .position(|i| i.identifier == cursor)
                    .map_or(matching.len(), |p| p + 1),
            };
            let first = vars["first"].as_u64().unwrap_or(50) as usize;
            let page: Vec<&Issue> = matching
                .iter()
                .skip(start)
                .take(first.min(w.max_page))
                .copied()
                .collect();
            let more = start + page.len() < matching.len();
            let end = page.last().map(|i| i.identifier.clone());
            let nodes: Vec<Value> = page.iter().map(|i| issue_json(w, i)).collect();
            Ok(json!({ "issues": { "nodes": nodes,
                       "pageInfo": { "hasNextPage": more, "endCursor": end } } }))
        }
        // What the live suite's cleanup asks: the issues the key's user
        // created in a team since a time. Trashed issues aren't listed,
        // as in Linear; every issue here is the key's user's.
        "IssuesCreated" => {
            let filter = &vars["filter"];
            if filter["team"]["key"]["eq"] != "ENG" {
                return Err("the filter names no team".into());
            }
            if filter["creator"]["isMe"]["eq"] != true {
                return Err("the filter doesn't keep to the key's user".into());
            }
            let since = filter["createdAt"]["gte"].as_str().unwrap_or_default();
            let matching: Vec<&Issue> = w
                .issues
                .iter()
                .filter(|i| !i.trashed && i.created.as_str() >= since)
                .collect();
            let start = match vars["after"].as_str() {
                None => 0,
                Some(cursor) => matching
                    .iter()
                    .position(|i| i.identifier == cursor)
                    .map_or(matching.len(), |p| p + 1),
            };
            let first = vars["first"].as_u64().unwrap_or(50) as usize;
            let page: Vec<&Issue> = matching
                .iter()
                .skip(start)
                .take(first.min(w.max_page))
                .copied()
                .collect();
            let more = start + page.len() < matching.len();
            let end = page.last().map(|i| i.identifier.clone());
            let nodes: Vec<Value> = page
                .iter()
                .map(|i| json!({ "id": i.id, "identifier": i.identifier }))
                .collect();
            Ok(json!({ "issues": { "nodes": nodes,
                       "pageInfo": { "hasNextPage": more, "endCursor": end } } }))
        }
        other => Err(format!("the simulator doesn't answer `{other}`")),
    }
}
