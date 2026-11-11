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

/// A fixed read-only tree: each file's bytes are a list of ints, and
/// the script encodes them (base64) as a provider answers.
const FILESYSTEM: &str = r#"
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

def _dirs():
    found = {}
    for path in FILES:
        folder = _parent(path)
        for _ in range(8):
            if not folder:
                break
            found[folder] = True
            folder = _parent(folder)
    return found

def stat(x):
    path = x["input"]["path"]
    if path in FILES:
        data = FILES[path]
        return {"result": {"entry": {"kind": "file", "size": len(data), "is_text": 0 not in data}}}
    if path in _dirs():
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
    entries = [{"path": p, "kind": "file"} for p in FILES] + [{"path": d, "kind": "dir"} for d in _dirs()]
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
        &[("shelf.star", FILESYSTEM)],
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
