//! A scripted language server for tests (`python3`): answers initialize,
//! echoes `ping`, asks `workspace/configuration` / `workspace/applyEdit`
//! when poked, records `didOpen`s, dies on `die`, publishes whatever
//! diagnostics a `publish` notification carries, and gives fixed answers
//! to definition, references, hover, document/workspace symbols, the call
//! hierarchy and rename (0-based LSP positions around line 3).

#![cfg(test)]

use oxplow_config::LspServerConfig;

const SCRIPT: &str = r#"
import sys, json

def read_message():
    headers = b""
    while b"\r\n\r\n" not in headers:
        ch = sys.stdin.buffer.read(1)
        if not ch:
            return None
        headers += ch
    length = 0
    for line in headers.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    return json.loads(sys.stdin.buffer.read(length).decode("utf-8"))

def write_message(payload):
    body = json.dumps(payload).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n")
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()

opened = []
root = ""

def rng(l1, c1, l2, c2):
    return {"start": {"line": l1, "character": c1}, "end": {"line": l2, "character": c2}}
while True:
    msg = read_message()
    if msg is None:
        break
    method = msg.get("method")
    if "id" in msg and method == "initialize":
        root = msg["params"].get("rootUri") or ""
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": {
            "capabilities": {"completionProvider": {"triggerCharacters": ["."]}},
            "clientCapabilities": msg["params"]["capabilities"],
        }})
    elif "id" in msg and method == "shutdown":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": None})
    elif method == "exit":
        break
    elif method == "die":
        sys.exit(1)
    elif method == "ping":
        write_message({"jsonrpc": "2.0", "method": "pong", "params": msg.get("params")})
    elif method == "askConfig":
        write_message({"jsonrpc": "2.0", "id": 7, "method": "workspace/configuration",
                       "params": {"items": [{"section": "a"}, {"section": "b"}]}})
    elif method == "askApplyEdit":
        write_message({"jsonrpc": "2.0", "id": 9, "method": "workspace/applyEdit",
                       "params": {"label": "do it", "edit": {"changes": {}}}})
    elif method == "textDocument/didOpen":
        opened.append(msg["params"]["textDocument"])
    elif method == "listOpened":
        write_message({"jsonrpc": "2.0", "method": "openedDocs", "params": {"docs": opened}})
    elif "id" in msg and method == "textDocument/definition":
        uri = msg["params"]["textDocument"]["uri"]
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": [
            {"uri": uri, "range": rng(2, 4, 2, 9)}]})
    elif "id" in msg and method == "textDocument/references":
        uri = msg["params"]["textDocument"]["uri"]
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": [
            {"uri": uri, "range": rng(2, 4, 2, 9)},
            {"uri": uri, "range": rng(7, 0, 7, 5)}]})
    elif "id" in msg and method == "textDocument/hover":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": {
            "contents": {"kind": "markdown", "value": "**class** Widget"},
            "range": rng(2, 4, 2, 9)}})
    elif "id" in msg and method == "textDocument/documentSymbol":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": [
            {"name": "Widget", "kind": 5, "range": rng(2, 0, 6, 0), "selectionRange": rng(2, 6, 2, 12),
             "children": [{"name": "spin", "kind": 6, "range": rng(3, 4, 4, 0), "selectionRange": rng(3, 8, 3, 12)}]}]})
    elif "id" in msg and method == "workspace/symbol":
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": [
            {"name": "Widget", "kind": 5, "location": {"uri": root + "/w.py", "range": rng(2, 6, 2, 12)}}]})
    elif "id" in msg and method == "textDocument/prepareCallHierarchy":
        uri = msg["params"]["textDocument"]["uri"]
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": [
            {"name": "spin", "kind": 6, "uri": uri, "range": rng(3, 4, 4, 0), "selectionRange": rng(3, 8, 3, 12)}]})
    elif "id" in msg and method == "callHierarchy/incomingCalls":
        item = msg["params"]["item"]
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": [
            {"from": {"name": "main", "kind": 12, "uri": item["uri"], "range": rng(8, 0, 10, 0),
                      "selectionRange": rng(8, 4, 8, 8)},
             "fromRanges": [rng(9, 4, 9, 8)]}]})
    elif "id" in msg and method == "textDocument/rename":
        uri = msg["params"]["textDocument"]["uri"]
        write_message({"jsonrpc": "2.0", "id": msg["id"], "result": {"changes": {uri: [
            {"range": rng(2, 6, 2, 12), "newText": msg["params"]["newName"]}]}}})
    elif method == "publish":
        write_message({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": msg["params"]})
    elif "id" in msg and "method" not in msg:
        write_message({"jsonrpc": "2.0", "method": "answeredConfig", "params": {"result": msg.get("result")}})
"#;

/// The fake as the server for `language`, covering `extensions`.
pub fn config(language: &str, extensions: &[&str]) -> LspServerConfig {
    LspServerConfig {
        language_id: language.into(),
        extensions: extensions.iter().map(|e| e.to_string()).collect(),
        command: "python3".into(),
        args: vec!["-c".into(), SCRIPT.to_string()],
    }
}
