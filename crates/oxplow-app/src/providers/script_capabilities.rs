//! Script implementations (`providers::script`) of the capabilities other
//! than the task tracker: each is a small extension written here, activated,
//! called through its capability's host, and held to that capability's
//! conformance suite. A capability whose host can't take a pure script is
//! refused at activation, with a diagnostic that says why.

use std::time::Duration;

use oxplow_domain::Actor;
use serde_json::json;

use crate::test_fixtures::ThreadFixture;

/// An extension `name` of the project, its manifest and its script files.
async fn activated(name: &str, manifest: &str, files: &[(&str, &str)]) -> ThreadFixture {
    let fx = crate::test_fixtures::services_with_thread().await;
    write(&fx, name, manifest, files);
    crate::test_fixtures::activate(&fx.svc).await;
    fx
}

fn write(fx: &ThreadFixture, name: &str, manifest: &str, files: &[(&str, &str)]) {
    let dir = fx
        .svc
        .layout
        .project_dir
        .join(".oxplow/extensions")
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("extension.yaml"), manifest).unwrap();
    for (path, text) in files {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

const HARNESS_MANIFEST: &str = r#"manifest: 2
name: script-harness
description: An agent harness written as a script.
engine: ">=0.7"
version: 0.1.0
intent:
  purpose: Show an agent harness implemented by a script.
  origin: null
  examples:
    - name: launches
      input: { sql: "SELECT 1" }
      expect: registered as a harness
implementations:
  - capability: agent_harness
    id: scripted
    title: Scripted
    entry: harness.star
"#;

const HARNESS: &str = r#"
def launch(x):
    i = x["input"]
    # The identity env (the session's bearer among it) rides the launch's
    # env, never its command line.
    return {"result": {"spec": {
        "kind": "pty",
        "command": "scripted-agent --cwd '%s'" % i["workspace"],
        "env": i["identity_env"],
    }}}

def tool_use(x):
    body = x["input"]["body"]
    name = body.get("tool_name")
    if not name:
        return {"result": {"tool": None}}
    return {"result": {"tool": {"name": name, "input": body.get("tool_input", {})}}}

def render(x):
    a = x["input"]["answer"]
    if a["kind"] == "deny":
        return {"result": {"body": {"decision": "block", "reason": a["reason"]}}}
    if a["kind"] == "context":
        return {"result": {"body": {"context": a["text"]}}}
    return {"result": {"body": {"continue": True}}}
"#;

/// A script is an agent harness: registered under its id beside the
/// built-ins, called through the harness host, and held to the harness
/// conformance suite.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_agent_harness_conforms() {
    let fx = activated(
        "script-harness",
        HARNESS_MANIFEST,
        &[("harness.star", HARNESS)],
    )
    .await;
    assert!(fx.svc.event_pump.settle_all(Duration::from_secs(10)).await);
    assert!(fx.svc.harnesses.has("scripted"), "registered");
    let run = crate::harness_conformance::suite(&fx.svc, "scripted", &json!({}), &[]).await;
    assert_eq!(run.findings, vec![]);
}

/// The suite is not vacuous against a script: one that puts the session's
/// bearer on the command line is found out.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_harness_that_leaks_the_bearer_is_found_out() {
    let leaky = HARNESS.replace(
        "\"command\": \"scripted-agent --cwd '%s'\" % i[\"workspace\"]",
        "\"command\": \"scripted-agent --token %s\" % i[\"endpoints\"][\"hook_token\"]",
    );
    assert_ne!(leaky, HARNESS);
    let fx = activated(
        "script-harness",
        HARNESS_MANIFEST,
        &[("harness.star", &leaky)],
    )
    .await;
    let run = crate::harness_conformance::suite(&fx.svc, "scripted", &json!({}), &[]).await;
    assert!(
        run.findings
            .iter()
            .any(|f| f.check == "a_launch_keeps_the_bearer_in_its_env"),
        "{:#?}",
        run.findings
    );
}

const KNOWLEDGE_MANIFEST: &str = r#"manifest: 2
name: script-knowledge
description: A knowledge store written as a script.
engine: ">=0.7"
version: 0.1.0
intent:
  purpose: Show a knowledge store implemented by a script.
  origin: null
  examples:
    - name: writes
      input: { sql: "SELECT 1" }
      expect: chosen as the knowledge store, pages are recorded
permissions: [sql.read]
implementations:
  - capability: knowledge
    id: scripted
    title: Scripted
    entry: store.star
    needs: [sql.read]
"#;

const KNOWLEDGE: &str = r###"
# A store that keeps nothing of its own: each answer is the page's whole
# record (`knowledge.page.recorded@2`), and a later verb reads the page
# back from oxplow's record (`v_knowledge_page`, `v_knowledge_body`).

def _refs(body):
    out = []
    for part in body.split("[[")[1:]:
        if "]]" not in part:
            continue
        inner = part.split("]]")[0].strip()
        if not inner:
            continue
        if "/" in inner or "." in inner:
            ref = uri.build("oxplow/file", path = inner)
        else:
            ref = uri.build("oxplow/wiki", slug = inner)
        if ref not in out:
            out.append(ref)
    return out

def _title(body, slug):
    for line in body.split("\n"):
        if line.startswith("# "):
            return line[2:].strip()
    return slug

def _recorded(slug, title, body, deleted):
    page = uri.build("oxplow/wiki", slug = slug)
    record = {"uri": page, "title": title, "body": body, "uris": _refs(body), "updated_at": "2026-01-01T00:00:00Z"}
    if deleted:
        record["deleted"] = True
    return {"type": "knowledge.page.recorded", "v": 2, "payload": {"page": record}, "subject": [page]}

def _page(slug):
    rows = scope("sql.read", {
        "sql": "SELECT p.title AS title, b.body AS body FROM v_knowledge_page p JOIN v_knowledge_body b ON b.uri = p.uri WHERE p.uri = :uri",
        "params": {"uri": uri.build("oxplow/wiki", slug = slug)},
    })
    if not rows:
        fail("no page %s" % slug)
    return rows[0]

def write_page(x):
    i = x["input"]
    title = i.get("title") or _title(i["body"], i["slug"])
    return {"result": {"page": i["slug"]}, "events": [_recorded(i["slug"], title, i["body"], False)]}

def delete_page(x):
    slug = x["input"]["slug"]
    page = _page(slug)
    return {"result": {}, "events": [_recorded(slug, page["title"], page["body"], True)]}

def link(x):
    i = x["input"]
    slug = i["page"]
    page = _page(slug)
    body = page["body"].rstrip()
    if "## Related" not in body:
        body += "\n\n## Related"
    body += "\n\n[[%s]]\n" % i["target"]
    return {"result": {}, "events": [_recorded(slug, page["title"], body, False)]}
"###;

/// `KNOWLEDGE` (or a variant of it) as the project's knowledge store,
/// chosen as a person's setting does.
async fn knowledge_store(script: &str) -> ThreadFixture {
    let fx = activated(
        "script-knowledge",
        KNOWLEDGE_MANIFEST,
        &[("store.star", script)],
    )
    .await;
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "oxplow.capability.choose",
            json!({ "capability": "knowledge", "provider": "scripted" }),
            true,
        )
        .await
        .unwrap();
    assert!(fx.svc.event_pump.settle_all(Duration::from_secs(10)).await);
    crate::test_fixtures::activate(&fx.svc).await;
    // The switch empties core's record: let it finish before any write.
    fx.svc
        .event_pump
        .settle(
            &[crate::knowledge::KnowledgeSwitch::NAME],
            Duration::from_secs(10),
        )
        .await;
    assert!(!fx.svc.wiki.live(), "the script is the active store");
    fx
}

/// A script is the project's knowledge store: chosen, the page commands
/// reach its verbs, core records what it answers, and the knowledge
/// conformance suite passes against it.
#[tokio::test(flavor = "multi_thread")]
async fn a_chosen_script_knowledge_store_conforms() {
    let fx = knowledge_store(KNOWLEDGE).await;
    let findings = crate::knowledge_conformance::suite(&fx.svc, &Actor::Human).await;
    assert_eq!(findings, vec![]);
}

/// The suite is not vacuous against a script: a store
/// that drops the link it was asked for is found out.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_store_that_misrecords_pages_is_found_out() {
    let wrong = KNOWLEDGE.replace(
        "body += \"\\n\\n[[%s]]\\n\" % i[\"target\"]",
        "body += \"\\n\"",
    );
    assert_ne!(wrong, KNOWLEDGE);
    let fx = knowledge_store(&wrong).await;
    let findings = crate::knowledge_conformance::suite(&fx.svc, &Actor::Human).await;
    let checks: Vec<_> = findings.iter().map(|f| f.check).collect();
    assert!(checks.contains(&"link"), "{findings:#?}");
}

const FILESYSTEM_MANIFEST: &str = r#"manifest: 2
name: script-fs
description: A filesystem written as a script.
engine: ">=0.7"
version: 0.1.0
intent:
  purpose: Show a filesystem implemented by a script.
  origin: null
  examples:
    - name: reads
      input: { sql: "SELECT 1" }
      expect: serves a fixed tree
implementations:
  - capability: filesystem
    id: shelf
    title: Shelf
    entry: shelf.star
"#;

/// The fixed tree both filesystem scripts start from: each file's bytes are a list of ints, and
/// the script encodes them (base64) as a provider answers.
const FILESYSTEM_HELPERS: &str = r#"
FILES = {
    "conformance-files/a.txt": [111, 110, 101, 10, 116, 119, 111, 10, 116, 104, 114, 101, 101, 10],
    "conformance-files/src/main.rs": [102, 110, 32, 109, 97, 105, 110, 40, 41, 32, 123, 125, 10],
    "conformance-files/src/deep/x.txt": [120, 10],
    "conformance-files/bin.dat": [0, 1, 2, 255, 0],
}

ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

def _b64(data):
    out = ""
    for at in range(0, len(data), 3):
        chunk = data[at:at + 3]
        n = chunk[0] << 16
        if len(chunk) > 1:
            n = n | (chunk[1] << 8)
        if len(chunk) > 2:
            n = n | chunk[2]
        out += ALPHABET[(n >> 18) & 63] + ALPHABET[(n >> 12) & 63]
        out += ALPHABET[(n >> 6) & 63] if len(chunk) > 1 else "="
        out += ALPHABET[n & 63] if len(chunk) > 2 else "="
    return out

def _parent(path):
    return path.rsplit("/", 1)[0] if "/" in path else ""

def _dirs(paths):
    found = {}
    for path in paths:
        folder = _parent(path)
        for _ in range(8):
            if not folder:
                break
            found[folder] = True
            folder = _parent(folder)
    return found
"#;

/// The read-only verbs over the fixed tree.
const FILESYSTEM_VERBS: &str = r#"
def stat(x):
    path = x["input"]["path"]
    if path in FILES:
        data = FILES[path]
        return {"result": {"entry": {"kind": "file", "size": len(data), "is_text": 0 not in data}}}
    if path in _dirs(FILES):
        return {"result": {"entry": {"kind": "dir"}}}
    return {"result": {"entry": None}}

def read(x):
    i = x["input"]
    if i["path"] not in FILES:
        return {"result": {"bytes": None}}
    data = FILES[i["path"]]
    offset = i.get("offset", 0)
    end = len(data) if "len" not in i else min(len(data), offset + i["len"])
    return {"result": {"bytes": _b64(data[min(offset, len(data)):end])}}

def list(x):
    i = x["input"]
    entries = [{"path": p, "kind": "file"} for p in FILES] + [{"path": d, "kind": "dir"} for d in _dirs(FILES)]
    if "parent" in i:
        entries = [e for e in entries if _parent(e["path"]) == i["parent"]]
    return {"result": {"entries": sorted(entries, key = lambda e: e["path"])}}
"#;

/// A script is a filesystem: registered under `<extension>:<id>` beside
/// core's, read through the filesystem host, and held to the filesystem
/// conformance checks (read-only, so the write check wants a refusal).
#[tokio::test(flavor = "multi_thread")]
async fn a_script_filesystem_passes_the_filesystem_checks() {
    use crate::filesystem_conformance as c;
    let fx = activated(
        "script-fs",
        FILESYSTEM_MANIFEST,
        &[(
            "shelf.star",
            &format!("{FILESYSTEM_HELPERS}{FILESYSTEM_VERBS}"),
        )],
    )
    .await;
    let fs = fx.svc.filesystems.resolve(Some("script-fs:shelf")).unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    assert!(!fs.has_feature("writable"));
    c::stat_describes_what_read_returns(&*fs, &cx, None).await;
    c::read_round_trips_and_ranges_slice(&*fs, &cx, None).await;
    c::lists_one_level_or_the_whole_tree(&*fs, &cx, None).await;
    c::a_path_never_leaves_the_filesystem(&*fs, &cx, None).await;
    c::writes_only_when_writable(&*fs, &cx, None).await;
}

const WRITABLE_FILESYSTEM_MANIFEST: &str = r#"manifest: 2
name: script-fs-w
description: A writable filesystem written as a script.
engine: ">=0.7"
version: 0.1.0
intent:
  purpose: Show a filesystem implemented by a script that keeps its writes in private state.
  origin: null
  examples:
    - name: writes
      input: { sql: "SELECT 1" }
      expect: serves a fixed tree and what was written to it
implementations:
  - capability: filesystem
    id: shelf
    title: Shelf
    entry: shelf.star
    features: { writable: true }
    needs: [state.read, state.write]
"#;

/// The fixed tree with writes laid over it: a write is kept in the
/// extension's private state (`written`, each path's base64 and size), and
/// every read verb looks there first.
const WRITABLE_FILESYSTEM_VERBS: &str = r#"
STATE = {"scope": "project", "area": "state"}

def _unb64(text):
    out = []
    n = 0
    bits = 0
    for c in text.elems():
        if c == "=":
            break
        n = (n << 6) | ALPHABET.index(c)
        bits += 6
        if bits >= 8:
            bits -= 8
            out.append((n >> bits) & 255)
            n = n & ((1 << bits) - 1)
    return out

def _written():
    snap = scope("state.read", STATE)
    return snap, (snap["values"].get("written", {}) if snap else {})

def stat(x):
    path = x["input"]["path"]
    _, written = _written()
    if path in written:
        return {"result": {"entry": {"kind": "file", "size": written[path]["size"], "is_text": True}}}
    if path in FILES:
        data = FILES[path]
        return {"result": {"entry": {"kind": "file", "size": len(data), "is_text": 0 not in data}}}
    if path in _dirs([p for p in FILES] + [p for p in written]):
        return {"result": {"entry": {"kind": "dir"}}}
    return {"result": {"entry": None}}

def read(x):
    i = x["input"]
    _, written = _written()
    if i["path"] in written:
        data = _unb64(written[i["path"]]["b64"])
    elif i["path"] in FILES:
        data = FILES[i["path"]]
    else:
        return {"result": {"bytes": None}}
    offset = i.get("offset", 0)
    end = len(data) if "len" not in i else min(len(data), offset + i["len"])
    return {"result": {"bytes": _b64(data[min(offset, len(data)):end])}}

def list(x):
    i = x["input"]
    _, written = _written()
    files = [p for p in FILES if p not in written] + [p for p in written]
    entries = [{"path": p, "kind": "file"} for p in files] + [{"path": d, "kind": "dir"} for d in _dirs(files)]
    if "parent" in i:
        entries = [e for e in entries if _parent(e["path"]) == i["parent"]]
    return {"result": {"entries": sorted(entries, key = lambda e: e["path"])}}

def write(x):
    i = x["input"]
    snap, written = _written()
    values = dict(snap["values"]) if snap else {}
    written = dict(written)
    written[i["path"]] = {"b64": i["content"], "size": len(_unb64(i["content"]))}
    values["written"] = written
    scope("state.write", {
        "scope": "project",
        "area": "state",
        "expected": snap["stamp"] if snap else None,
        "schema_version": 1,
        "values": values,
    })
    return {"result": {}}
"#;

/// A script is a writable filesystem: a write is kept in its extension's
/// private state (`state.write`) and every later call reads it back
/// (`state.read`), and the filesystem conformance checks pass — the write
/// ones among them.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_filesystem_keeps_its_writes_in_private_state_and_conforms() {
    use crate::filesystem_conformance as c;
    let fx = activated(
        "script-fs-w",
        WRITABLE_FILESYSTEM_MANIFEST,
        &[(
            "shelf.star",
            &format!("{FILESYSTEM_HELPERS}{WRITABLE_FILESYSTEM_VERBS}"),
        )],
    )
    .await;
    let fs = fx
        .svc
        .filesystems
        .resolve(Some("script-fs-w:shelf"))
        .unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    assert!(fs.has_feature("writable"));
    c::stat_describes_what_read_returns(&*fs, &cx, None).await;
    c::read_round_trips_and_ranges_slice(&*fs, &cx, None).await;
    c::lists_one_level_or_the_whole_tree(&*fs, &cx, None).await;
    c::a_path_never_leaves_the_filesystem(&*fs, &cx, None).await;
    c::writes_only_when_writable(&*fs, &cx, None).await;
}

/// A filesystem over the stream's own worktree: every verb reads the tree
/// (`files`), which `needs: [tree.read]` (bare, or a path) gives it.
const WORKTREE_FILESYSTEM: &str = r#"
ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

def _b64(data):
    out = ""
    for at in range(0, len(data), 3):
        chunk = data[at:at + 3]
        n = chunk[0] << 16
        if len(chunk) > 1:
            n = n | (chunk[1] << 8)
        if len(chunk) > 2:
            n = n | chunk[2]
        out += ALPHABET[(n >> 18) & 63] + ALPHABET[(n >> 12) & 63]
        out += ALPHABET[(n >> 6) & 63] if len(chunk) > 1 else "="
        out += ALPHABET[n & 63] if len(chunk) > 2 else "="
    return out

def _hit(path):
    found = files(path)
    return found[0] if found else None

def stat(x):
    f = _hit(x["input"]["path"])
    if f == None:
        return {"result": {"entry": None}}
    return {"result": {"entry": {"kind": "file", "size": utf8_len(f["text"]), "is_text": True}}}

def read(x):
    f = _hit(x["input"]["path"])
    if f == None:
        return {"result": {"bytes": None}}
    return {"result": {"bytes": _b64([ord(c) for c in f["text"].elems()])}}

def list(x):
    return {"result": {"entries": [{"path": f["path"], "kind": "file"} for f in files("**")]}}
"#;

fn worktree_filesystem_manifest(permissions: &str, needs: &str) -> String {
    format!(
        r#"manifest: 2
name: script-tree
description: A filesystem over the worktree, written as a script.
engine: ">=0.7"
version: 0.1.0
intent:
  purpose: Show a script implementation that reads the stream's worktree.
  origin: null
  examples:
    - name: reads
      input: {{ sql: "SELECT 1" }}
      expect: serves the worktree's files
permissions: {permissions}
implementations:
  - capability: filesystem
    id: tree
    title: Tree
    entry: tree.star
    needs: {needs}
"#
    )
}

fn worktree_files(fx: &ThreadFixture) {
    let root = &fx.svc.layout.project_dir;
    for (path, text) in [("notes/hello.txt", "hi there\n"), ("secret/x.txt", "no\n")] {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
}

/// A script that needs `tree.read` (the extension permits `files.read`)
/// reads the worktree as of the call, not a stored snapshot of it: a file
/// written just before is there.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_implementation_reads_the_worktree_with_tree_read() {
    let fx = crate::test_fixtures::services_with_thread().await;
    worktree_files(&fx);
    write(
        &fx,
        "script-tree",
        &worktree_filesystem_manifest("[files.read]", "[tree.read]"),
        &[("tree.star", WORKTREE_FILESYSTEM)],
    );
    crate::test_fixtures::activate(&fx.svc).await;
    let fs = fx
        .svc
        .filesystems
        .resolve(Some("script-tree:tree"))
        .unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    let stat = fs
        .stat(&cx, None, "notes/hello.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stat.size, Some(9));
    let bytes = fs.read(&cx, None, "notes/hello.txt", None).await.unwrap();
    assert_eq!(bytes.as_deref(), Some(&b"hi there\n"[..]));
    // Written after the first call: the tree is the worktree's now.
    std::fs::write(fx.svc.layout.project_dir.join("notes/later.txt"), "later\n").unwrap();
    let listed = fs.list(&cx, None, None).await.unwrap();
    let paths: Vec<&str> = listed.iter().map(|e| e.path.as_str()).collect();
    assert!(
        paths.contains(&"notes/hello.txt") && paths.contains(&"notes/later.txt"),
        "{paths:?}"
    );
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with(".git/") || p.starts_with(".oxplow/snapshots")),
        "{paths:?}"
    );
}

/// `tree.read:<path>` bounds what the script sees to that folder: the
/// host filters the tree before the script has it.
#[tokio::test(flavor = "multi_thread")]
async fn a_tree_read_with_a_path_shows_only_that_path() {
    let fx = crate::test_fixtures::services_with_thread().await;
    worktree_files(&fx);
    write(
        &fx,
        "script-tree",
        &worktree_filesystem_manifest("[files.read]", "[\"tree.read:notes/\"]"),
        &[("tree.star", WORKTREE_FILESYSTEM)],
    );
    crate::test_fixtures::activate(&fx.svc).await;
    let fs = fx
        .svc
        .filesystems
        .resolve(Some("script-tree:tree"))
        .unwrap();
    let cx = oxplow_domain::filesystem::FsCx::default();
    assert!(fs
        .stat(&cx, None, "notes/hello.txt")
        .await
        .unwrap()
        .is_some());
    assert_eq!(fs.stat(&cx, None, "secret/x.txt").await.unwrap(), None);
    let listed = fs.list(&cx, None, None).await.unwrap();
    assert!(
        listed.iter().all(|e| e.path.starts_with("notes/")),
        "{listed:?}"
    );
}

/// The extension's permissions must cover the needs (a person approves them
/// when it is deployed): `tree.read` without `files.read` holds it back.
#[tokio::test(flavor = "multi_thread")]
async fn tree_read_needs_the_files_permission() {
    let fx = crate::test_fixtures::services_with_thread().await;
    write(
        &fx,
        "script-tree",
        &worktree_filesystem_manifest("[]", "[tree.read]"),
        &[("tree.star", WORKTREE_FILESYSTEM)],
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let said = format!("{problems:?}");
    assert!(
        said.contains("implementation `tree` needs `files.read` in `permissions:`"),
        "{said}"
    );
    assert!(fx
        .svc
        .filesystems
        .resolve(Some("script-tree:tree"))
        .is_err());
}

/// A script that calls a scope it didn't declare is refused when it calls:
/// `state.write` is the extension's to write only when its `needs` say so.
#[tokio::test(flavor = "multi_thread")]
async fn an_undeclared_state_write_is_refused() {
    let fx = activated(
        "script-fs-w",
        &WRITABLE_FILESYSTEM_MANIFEST
            .replace("needs: [state.read, state.write]", "needs: [state.read]"),
        &[(
            "shelf.star",
            &format!("{FILESYSTEM_HELPERS}{WRITABLE_FILESYSTEM_VERBS}"),
        )],
    )
    .await;
    let fs = fx
        .svc
        .filesystems
        .resolve(Some("script-fs-w:shelf"))
        .unwrap();
    let err = fs
        .write(
            &oxplow_domain::filesystem::FsCx::default(),
            "kept.txt",
            b"x",
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("`state.write` isn't in"), "{err}");
}

const SNAPSHOTS_MANIFEST: &str = r#"manifest: 2
name: script-snap
description: A snapshot store written as a script.
engine: ">=0.7"
version: 0.1.0
intent:
  purpose: Show a snapshots implementation that marks the worktree and keeps what it marked in private state.
  origin: null
  examples:
    - name: marks
      input: { sql: "SELECT 1" }
      expect: marks the worktree, reads back what it kept
permissions: [files.read]
implementations:
  - capability: snapshots
    id: kept
    title: Kept in state
    entry: snap.star
    features: { contents: true }
    needs: [tree.read, state.read, state.write]
"#;

/// A snapshots implementation in a script. `mark` reads the worktree
/// (`files`, `tree.read`) and keeps each mark's listing and each new file's
/// text in the extension's private state (`state.write`); `changed` compares
/// two kept listings; `read_at` answers the kept text, as base64.
const SNAPSHOTS: &str = r#"
STATE = {"scope": "project", "area": "state"}
ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"

def _utf8(text):
    out = []
    for n in text.codepoints():
        if n < 128:
            out.append(n)
        elif n < 2048:
            out += [192 | (n >> 6), 128 | (n & 63)]
        elif n < 65536:
            out += [224 | (n >> 12), 128 | ((n >> 6) & 63), 128 | (n & 63)]
        else:
            out += [240 | (n >> 18), 128 | ((n >> 12) & 63), 128 | ((n >> 6) & 63), 128 | (n & 63)]
    return out

def _b64(data):
    out = ""
    for at in range(0, len(data), 3):
        chunk = data[at:at + 3]
        n = chunk[0] << 16
        if len(chunk) > 1:
            n = n | (chunk[1] << 8)
        if len(chunk) > 2:
            n = n | chunk[2]
        out += ALPHABET[(n >> 18) & 63] + ALPHABET[(n >> 12) & 63]
        out += ALPHABET[(n >> 6) & 63] if len(chunk) > 1 else "="
        out += ALPHABET[n & 63] if len(chunk) > 2 else "="
    return out

def _kept():
    snap = scope("state.read", STATE)
    values = snap["values"] if snap else {}
    return snap, values.get("trees", {}), values.get("blobs", {})

def mark(x):
    tree = {}
    texts = {}
    for f in files("**"):
        identity = xxh3_128(f["text"])
        tree[f["path"]] = {"id": identity, "size": utf8_len(f["text"])}
        texts[identity] = f["text"]
    # The mark's handle names its listing: the same tree is the same handle.
    handle = xxh3_128(json.encode(tree))
    snap, trees, blobs = _kept()
    if handle not in trees:
        trees = dict(trees)
        trees[handle] = tree
        blobs = dict(blobs)
        for identity in texts:
            blobs[identity] = texts[identity]
        scope("state.write", {
            "scope": "project",
            "area": "state",
            "expected": snap["stamp"] if snap else None,
            "schema_version": 1,
            "values": {"trees": trees, "blobs": blobs},
        })
    return {"result": {"handle": handle, "file_count": len(tree), "unchanged": x["input"]["parent"] == handle}}

def changed(x):
    i = x["input"]
    _, trees, _ = _kept()
    new = trees[i["to"]]
    old = trees[i["from"]] if i["from"] != None else None
    changes = []
    for path in new:
        entry = new[path]
        if old == None or path not in old:
            changes.append({"path": path, "kind": "added", "identity": entry["id"], "size": entry["size"]})
        elif old[path]["id"] != entry["id"]:
            changes.append({"path": path, "kind": "modified", "identity": entry["id"], "size": entry["size"]})
    if old != None:
        for path in old:
            if path not in new:
                changes.append({"path": path, "kind": "deleted"})
    return {"result": {"changes": changes}}

def read_at(x):
    i = x["input"]
    _, trees, blobs = _kept()
    entry = trees.get(i["handle"], {}).get(i["path"])
    if entry == None:
        fail("no %s in mark %s" % (i["path"], i["handle"]))
    return {"result": {"bytes": _b64(_utf8(blobs[entry["id"]]))}}
"#;

/// A script is a snapshots implementation: it marks the worktree it reads
/// (`tree.read`), keeps what it marked in its private state
/// (`state.write`), and the snapshots conformance suite passes against it —
/// the round trip of kept bytes, content identity, diffs, turn anchoring
/// and budgets.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_snapshots_implementation_conforms() {
    use crate::snapshot_conformance as sc;
    let fx = crate::test_fixtures::services_with_thread().await;
    write(
        &fx,
        "script-snap",
        SNAPSHOTS_MANIFEST,
        &[("snap.star", SNAPSHOTS)],
    );
    let stream = sc::tests::stream_of(&fx, Duration::ZERO);
    crate::test_fixtures::activate(&fx.svc).await;
    sc::tests::choose(&fx, "kept").await;
    let active = fx.svc.snapshots.active().expect("an active implementation");
    assert_eq!(active.id(), "kept");
    assert!(active.contents());
    let run = sc::suite(&fx.svc, &*active, stream).await;
    assert!(run.findings.is_empty(), "{:#?}", run.findings);
}

/// A file with non-ASCII text is recorded at its size in bytes and reads
/// back as its UTF-8 bytes: the script's text is characters, the ledger's
/// files are bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_snapshots_implementation_keeps_non_ascii_text_as_bytes() {
    use crate::snapshot_conformance as sc;
    let fx = crate::test_fixtures::services_with_thread().await;
    write(
        &fx,
        "script-snap",
        SNAPSHOTS_MANIFEST,
        &[("snap.star", SNAPSHOTS)],
    );
    let stream = sc::tests::stream_of(&fx, Duration::ZERO);
    crate::test_fixtures::activate(&fx.svc).await;
    sc::tests::choose(&fx, "kept").await;
    std::fs::write(
        fx.svc.layout.project_dir.join("accents.txt"),
        "é\u{1F600}\n",
    )
    .unwrap();
    let active = fx.svc.snapshots.active().expect("an active implementation");
    let request = oxplow_domain::snapshot::MarkRequest::new(
        stream,
        oxplow_domain::snapshot::SnapshotTrigger::Manual,
    );
    let marked = active.mark(&request).await.unwrap().unwrap();
    let kept = active
        .read_at(marked.snapshot, "accents.txt")
        .await
        .unwrap();
    assert_eq!(kept, "é\u{1F600}\n".as_bytes());
    let files = fx
        .svc
        .snapshot_store
        .list_tree_files_at(marked.snapshot)
        .await
        .unwrap();
    let size = files
        .iter()
        .find(|f| f.path == "accents.txt")
        .map(|f| f.size_bytes);
    assert_eq!(size, Some(2 + 4 + 1));
}

/// The suite is not vacuous against a script: one that answers a read
/// with the wrong text is found out (and one that never reports a modified
/// file would be, by `changed_names_what_was_added_modified_and_deleted`).
#[tokio::test(flavor = "multi_thread")]
async fn a_script_snapshots_implementation_off_the_contract_is_found_out() {
    use crate::snapshot_conformance as sc;
    let wrong = SNAPSHOTS.replace("blobs[entry[\"id\"]]", "\"same\"");
    assert_ne!(wrong, SNAPSHOTS);
    let fx = crate::test_fixtures::services_with_thread().await;
    write(
        &fx,
        "script-snap",
        SNAPSHOTS_MANIFEST,
        &[("snap.star", &wrong)],
    );
    let stream = sc::tests::stream_of(&fx, Duration::ZERO);
    crate::test_fixtures::activate(&fx.svc).await;
    sc::tests::choose(&fx, "kept").await;
    let active = fx.svc.snapshots.active().expect("an active implementation");
    let run = sc::suite(&fx.svc, &*active, stream).await;
    // Text that isn't the file's fails the mark, which checks every kept file
    // against the identity the script reported.
    assert!(
        run.findings
            .iter()
            .any(|f| f.check == "a_mark_records_a_snapshot"
                && f.message.contains("aren't the identity it reported")),
        "{:#?}",
        run.findings
    );
}
