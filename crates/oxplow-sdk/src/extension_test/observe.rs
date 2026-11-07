use super::*;

/// `observe: { <kind>: { .. } }`: a fact oxplow only learns from what
/// happens around it, made through the API core makes it with.
///
/// - `event: { type, v?, payload, subject?, anchors? }`: an event logged
///   and delivered, the extension's effects reacting.
/// - `evidence: { effort, observations?: [..], metric_deltas?: [..] }`: an
///   effort's evidence (a metric delta is `{ key, current, baseline?, warn_at?,
///   fail_at?, crossing?, kind?, direction?, .. }`).
/// - `commit: { sha, at, subject, files?, .. }`: a commit the indexer read.
/// - `change: { kind?, target, files?, functions? }`: a change's analysis,
///   as the change analysis stores it (`kind` is `commit` by default; an
///   effort's `target` is its id, like `eff1`).
/// - `test_run: { effort, at?, cases?: [{ name, status }], asserted_failed? }`:
///   a test run an effort recorded, as a run recorder writes it.
/// - `snapshot: { trigger?, elapsed_ms?, budget_ms?, revision?, branch?, files?: [{ path, hash }] }`:
///   one take in the stream's snapshot log.
/// - `diagnostics: { path, language?, items?: [{ severity, message, line?, col?, source?, code? }] }`:
///   what a language server reported for a file.
/// - `symbols: { path, language?, items?: [{ name, kind, id?, container?, signature?, line?, col? }] }`:
///   the definitions the index read from a file at a snapshot of it.
/// - `run: { extension, contribution, kind?, outcome?, days_ago?, cpu_ms?, ai_calls?, error? }`:
///   a finished run of an extension's code, as the host records it.
/// - `health: { extension, contribution, kind?, disable }`: a contribution
///   disabled after failures, with the reason.
/// - `token_usage: { thread?, effort?, session?, model?, prompt?, input_tokens?,
///   output_tokens?, cache_creation_input_tokens?, cache_read_input_tokens?, turn? }`:
///   the tokens a turn spent.
/// - `hint: { hint, evaluated?, thread? }`: a hint's query ran that many times.
/// - `nudge: { kind, message, thread?, effort?, audience?, delivered? }`: a
///   nudge a hint raised.
/// - `analyze: { kind: working | commit | …, sha?, streamId? }`: a change
///   analyzed now (`change_analysis::ensure_change`), so its files and
///   functions are there to read.
/// - `refs: { branches?, tags?, heads? }`: the branches, tags and stream
///   heads the git indexer read (each replaces what was stored).
/// - `visit: { page, kind?, label?, thread?, duration_ms? }`: a page the person
///   opened, as the shell records it.
/// - `facts: { measure, producer?, items?: [{ value?, path?, line?, rule?, severity?, detail?, subject?, dims? }] }`:
///   what a collector found, recorded as one capture of the measure.
/// - `retention: { days }`: the log's retention sweep, run as it is `days`
///   from now: payloads past their keep are expired, envelopes stay.
/// - `collect: { effort, collectors?: [key] }`: the fact collectors an
///   effort's end triggers, run over what the effort recorded.
/// - `turn: { prompt?, during?: [{ command, input?, as? }], tools?: [{ name, kind, paths?, command?, detail?,
///   ok?, exit_code? }], write?: { path, body }, answer?, compactions?: n,
///   context?: { tokens, window? }, thread?, session?, subagent?, peer? }`:
///   one turn on `thread` (thr1), in `session`, as
///   its harness's hooks report it — the prompt, each tool call, the file
///   the turn wrote (a snapshot of the worktree is taken after it), the
///   context compactions, and the turn's end with the agent's last message
///   and how full its context was.
pub(super) async fn observe(host: &Host, ext: &Extension, observed: &Value) -> Result<(), String> {
    let Some(map) = observed.as_object().filter(|m| m.len() == 1) else {
        return Err("name one kind — fix: `observe: { turn: { .. } }`".into());
    };
    let (kind, body) = map.iter().next().expect("one key");
    match kind.as_str() {
        "turn" => observe_turn(host, body).await,
        "evidence" => observe_evidence(host, body).await,
        "event" => observe_event(host, ext, body).await,
        "commit" => observe_commit(host, body).await,
        "change" => observe_change(host, body).await,
        "test_run" => observe_test_run(host, body).await,
        "snapshot" => observe_snapshot(host, body).await,
        "collect" => observe_collect(host, body).await,
        "diagnostics" => observe_diagnostics(host, body).await,
        "symbols" => observe_symbols(host, body).await,
        "retention" => observe_retention(host, body).await,
        "facts" => observe_facts(host, body).await,
        "visit" => observe_visit(host, body).await,
        "refs" => observe_refs(host, body).await,
        "analyze" => observe_analyze(host, body).await,
        "run" => observe_run(host, body).await,
        "health" => observe_health(host, body).await,
        "token_usage" => observe_token_usage(host, body).await,
        "hint" => observe_hint(host, body).await,
        "nudge" => observe_nudge(host, body).await,
        "conversation" => observe_conversation(host, body).await,
        other => {
            return Err(format!(
                "`{other}` isn't a kind — fix: `turn`, `evidence`, `event`, `commit`, `change`, `test_run`, `snapshot`, `collect`, `diagnostics`, `symbols`, `retention`, `facts`, `visit`, `refs`, `analyze`, `run`, `health`, `token_usage`, `hint`, `nudge` or `conversation`"
            ))
        }
    }?;
    // What the log now holds reaches the datasets, as a command's events do;
    // a reaction's own events (a checkpoint, a policy's answer to it) are
    // heard in turn.
    host.settle().await;
    host.svc
        .event_pump
        .deliver_projections()
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `event: { type, v?, payload, subject?, anchors? }`: an event oxplow
/// logs when something outside it happens (a contribution disabled after
/// failing, say), logged and delivered as a fixture's `when: { event }` is,
/// the extension's effects reacting to it.
pub(super) async fn observe_event(
    host: &Host,
    ext: &Extension,
    body: &Value,
) -> Result<(), String> {
    start_effects(host, ext).await?;
    let view = EventView::fixture(body, &host.svc.vocabulary.current()).map_err(|e| e.message())?;
    let mut env = oxplow_domain::events::Envelope::new(
        view.event_type.clone(),
        view.v,
        "fixture",
        view.payload.clone(),
    )
    .map_err(|e| e.to_string())?;
    env.subject = view.subject.clone();
    env.anchors = view.anchors.clone();
    host.svc
        .event_log_store
        .append(env)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The stream the throwaway's thread lives in.
pub(super) async fn first_stream(host: &Host) -> Result<oxplow_domain::StreamId, String> {
    host.svc
        .streams
        .list_streams()
        .await
        .map_err(|e| e.to_string())?
        .first()
        .map(|s| s.id)
        .ok_or_else(|| "no stream".to_string())
}

/// `change: { kind?, target, files?: [{ path, status?, additions?,
/// deletions?, zone?, is_test? }], functions?: [{ path, symbol, name?, status?,
/// container?, signature_changed?, body_changed?, start_line?, visibility?,
/// is_test?, length?, params_before?, params_after?, complexity?, complexity_delta? }] }`:
/// what the change analysis stored for a change (`SqliteChangeStore::
/// store_results`). A `working` or `effort` change's head is the working
/// tree; a `commit`'s is the commit `target`.
pub(super) async fn observe_change(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::{ChangeFileRow, ChangeFunctionRow, ChangeResults};
    use oxplow_domain::vcs::Revision;
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let kind = text(body, "kind").unwrap_or_else(|| "commit".into());
    let target = text(body, "target").ok_or("`change.target` names what the change is of")?;
    let target = match (kind.as_str(), target.strip_prefix("eff")) {
        ("effort", Some(n)) if n.parse::<i64>().is_ok() => n.to_string(),
        _ => target,
    };
    let head = match kind.as_str() {
        "working" | "effort" => Revision::Working,
        _ => Revision::git(target.clone()),
    };
    let int = |v: &Value, key: &str| v.get(key).and_then(Value::as_i64);
    let flag = |v: &Value, key: &str| v.get(key).and_then(Value::as_bool).unwrap_or(false);
    let list = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let mut results = ChangeResults::default();
    for f in list("files") {
        results.files.push(ChangeFileRow {
            path: text(&f, "path").ok_or("a change file names its `path`")?,
            status: text(&f, "status").unwrap_or_else(|| "modified".into()),
            additions: int(&f, "additions").unwrap_or(0),
            deletions: int(&f, "deletions").unwrap_or(0),
            zone: Some(text(&f, "zone").unwrap_or_else(|| "other".into())),
            is_test: flag(&f, "is_test"),
            ..Default::default()
        });
    }
    for f in list("functions") {
        let path = text(&f, "path").ok_or("a change function names its `path`")?;
        let symbol = text(&f, "symbol").ok_or("a change function names its `symbol`")?;
        results.functions.push(ChangeFunctionRow {
            symbol_ref: oxplow_domain::uri::build::symbol_uri(&path, &symbol),
            name: text(&f, "name").unwrap_or_else(|| symbol.clone()),
            container: text(&f, "container").unwrap_or_default(),
            status: text(&f, "status").unwrap_or_else(|| "modified".into()),
            signature_changed: flag(&f, "signature_changed"),
            body_changed: flag(&f, "body_changed"),
            start_line: int(&f, "start_line").unwrap_or(1),
            visibility: text(&f, "visibility").unwrap_or_else(|| "public".into()),
            is_test: flag(&f, "is_test"),
            length: int(&f, "length"),
            params_before: int(&f, "params_before"),
            params_after: int(&f, "params_after"),
            complexity: f.get("complexity").and_then(Value::as_f64),
            complexity_delta: f.get("complexity_delta").and_then(Value::as_f64),
            path,
            ..Default::default()
        });
    }
    let stream = first_stream(host).await?;
    let (row, _) = host
        .svc
        .change_store
        .get_or_create(stream.value(), &kind, &target, None, &head)
        .await
        .map_err(|e| e.to_string())?;
    host.svc
        .change_store
        .store_results(row.id, results, None, 0, 0, None)
        .await
        .map_err(|e| e.to_string())
}

/// `test_run: { effort, at?, producer?, branch?, cases?: [{ name, status }],
/// asserted_failed? }`: a run an effort recorded, as a run recorder writes
/// it: a `tests` capture with its detail and an `oxplow.test_case` fact
/// per case (or per failure asserted as a count with no report), through
/// the fact store. A `producer` other than `tests` is a record without
/// measurements.
pub(super) async fn observe_test_run(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::effort_store::EffortStore as _;
    use oxplow_domain::{EffortId, Timestamp};
    let effort = body
        .get("effort")
        .and_then(Value::as_str)
        .and_then(|e| e.strip_prefix("eff"))
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or("`test_run.effort` is an effort's id, like `eff1`")?;
    let found = host
        .svc
        .effort_store
        .get_effort(&EffortId::new(effort))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no effort eff{effort}"))?;
    let at = match body.get("at").and_then(Value::as_str) {
        Some(at) => Timestamp::parse(at).map_err(|e| format!("`test_run.at`: {e}"))?,
        None => Timestamp::now(),
    };
    let producer = body
        .get("producer")
        .and_then(Value::as_str)
        .unwrap_or("tests");
    let branch = body.get("branch").and_then(Value::as_str).unwrap_or("main");
    let asserted_failed = body
        .get("asserted_failed")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut cases = Vec::new();
    for c in body
        .get("cases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| c.get(key).and_then(Value::as_str).map(str::to_string);
        cases.push((
            text("name").ok_or("a test case names its `name`")?,
            text("status").ok_or("a test case names its `status`")?,
        ));
    }
    let failed = cases
        .iter()
        .filter(|(_, status)| status == "failed")
        .count() as i64
        + asserted_failed;
    let suites: Vec<Value> = if cases.is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "name": "suite",
            "cases": cases.iter().map(|(name, status)| json!({
                "classname": "mod", "name": name, "status": status
            })).collect::<Vec<_>>()
        })]
    };
    let facts = &host.svc.fact_store;
    let case = facts
        .get_measure("oxplow.test_case")
        .await
        .map_err(|e| e.to_string())?
        .ok_or("no `oxplow.test_case` measure")?;
    let stream = first_stream(host).await?;
    let mut capture = oxplow_db::NewMetricCapture::done(stream.value(), producer, "hook");
    capture.thread_id = Some(found.thread_id.value());
    capture.effort_id = Some(effort);
    capture.branch = Some(branch.to_string());
    capture.captured_at = Some(at);
    capture.detail_json = Some(
        json!({
            "kind": "test-detail",
            "payload": { "command": "cargo test", "failed": failed, "suites": suites }
        })
        .to_string(),
    );
    let mut rows: Vec<oxplow_db::NewFact> = cases
        .iter()
        .map(|(name, status)| oxplow_db::NewFact {
            subject: Some(oxplow_domain::uri::build::test_uri("mod", name)),
            dims_json: Some(json!({ "oxplow.status": status }).to_string()),
            ..oxplow_db::NewFact::new(case.id, 1.0)
        })
        .collect();
    for _ in 0..asserted_failed {
        rows.push(oxplow_db::NewFact {
            dims_json: Some(r#"{"oxplow.status":"failed"}"#.to_string()),
            ..oxplow_db::NewFact::new(case.id, 1.0)
        });
    }
    if producer != "tests" {
        rows.clear();
    }
    facts
        .record_facts(capture, rows)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `diagnostics: { path, language?, items?: [{ severity, message, line?,
/// col?, source?, code? }] }`: what a language server reported for one file
/// (`SqliteDiagnosticStore::replace_file`); no items says the file is clean.
pub(super) async fn observe_diagnostics(host: &Host, body: &Value) -> Result<(), String> {
    let path = body
        .get("path")
        .and_then(Value::as_str)
        .ok_or("`diagnostics.path` names the file")?;
    let language = body
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("rust");
    let mut rows = Vec::new();
    for d in body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| d.get(key).and_then(Value::as_str).map(str::to_string);
        let int = |key: &str, default: i64| d.get(key).and_then(Value::as_i64).unwrap_or(default);
        rows.push(oxplow_db::DiagnosticRow {
            severity: text("severity").ok_or("a diagnostic names its `severity`")?,
            message: text("message").ok_or("a diagnostic names its `message`")?,
            source: text("source"),
            code: text("code"),
            line: int("line", 1),
            col: int("col", 1),
            end_line: int("end_line", int("line", 1)),
            end_col: int("end_col", int("col", 1) + 1),
        });
    }
    let stream = first_stream(host).await?;
    host.svc
        .diagnostic_store
        .replace_file(stream.value(), language.into(), path.into(), rows)
        .await
        .map_err(|e| e.to_string())
}

/// `symbols: { path, language?, items?: [{ name, kind, id?, container?,
/// signature?, line?, col? }] }`: the definitions the symbol index read from
/// a file (`SqliteSymbolStore::record`), at a snapshot of the tree taken
/// with the file in it.
pub(super) async fn observe_symbols(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::symbol_store::{FileSymbols, SymbolCapture, SymbolRow};
    let path = body
        .get("path")
        .and_then(Value::as_str)
        .ok_or("`symbols.path` names the file")?;
    let language = body
        .get("language")
        .and_then(Value::as_str)
        .unwrap_or("rust");
    let mut rows = Vec::new();
    for sym in body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| sym.get(key).and_then(Value::as_str).map(str::to_string);
        let int = |key: &str, default: i64| sym.get(key).and_then(Value::as_i64).unwrap_or(default);
        let name = text("name").ok_or("a symbol names its `name`")?;
        rows.push(SymbolRow {
            id: text("id").unwrap_or_else(|| name.clone()),
            kind: text("kind").unwrap_or_else(|| "fn".into()),
            container: text("container"),
            signature: text("signature"),
            line: int("line", 1),
            col: int("col", 1),
            start_line: int("line", 1),
            start_col: int("col", 1),
            end_line: int("line", 1),
            end_col: int("col", 1) + name.len() as i64,
            name,
        });
    }
    let outcome = take_snapshot(
        host,
        &json!({ "trigger": "manual", "files": [{ "path": path, "hash": "symbols" }] }),
    )
    .await?
    .ok_or("no snapshot to read the symbols at")?;
    let stream = first_stream(host).await?;
    host.svc
        .symbol_store
        .record(
            stream.value(),
            outcome.snapshot_id,
            vec![FileSymbols {
                path: path.into(),
                language: language.into(),
                rows: Some(rows),
            }],
            SymbolCapture {
                files_collected: 1,
                files_failed: 0,
            },
        )
        .await
        .map_err(|e| e.to_string())
}

/// `run: { extension, contribution, kind?, outcome?, days_ago?, cpu_ms?,
/// ai_calls?, error? }`: a finished run of `extension`'s `contribution`
/// (`kind` a collector by default), recorded as the host does
/// (`extension_run_store::record_tx`); `days_ago` is when it began.
pub(super) async fn observe_run(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_domain::contribution::{ContributionId, ContributionKind};
    use oxplow_domain::run::{ContributionRef, Mode, Outcome, RunContext};
    let text = |k: &str| body.get(k).and_then(Value::as_str);
    let extension = text("extension").ok_or("`run.extension` names the extension")?;
    let local = text("contribution").ok_or("`run.contribution` names the contribution")?;
    let kind = match text("kind").unwrap_or("collector") {
        "collector" => ContributionKind::Collector,
        "effect" => ContributionKind::Effect,
        "command" => ContributionKind::Command,
        "job" => ContributionKind::Job,
        "advisory" => ContributionKind::Advisory,
        "implementation" => ContributionKind::Implementation,
        other => return Err(format!("`run.kind: {other}` — fix: collector, effect, command, job, advisory or implementation")),
    };
    let outcome = match text("outcome").unwrap_or("ok") {
        "ok" => Outcome::Ok,
        "skipped" => Outcome::Skipped,
        "failed" => Outcome::Failed,
        "denied" => Outcome::Denied,
        "cancelled" => Outcome::Cancelled,
        other => {
            return Err(format!(
                "`run.outcome: {other}` — fix: ok, skipped, failed, denied or cancelled"
            ))
        }
    };
    let mut run = RunContext::contribution(
        ContributionRef {
            id: ContributionId::new(extension, kind, local),
            version: None,
            generation: None,
            deployment: None,
        },
        Actor::System,
        Mode::External,
    );
    if let Some(days) = body.get("days_ago").and_then(Value::as_f64) {
        let at = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms() - (days * 86_400_000.0) as i64,
        );
        run = run.began_at(at);
    }
    run.used_cpu(body.get("cpu_ms").and_then(Value::as_u64).unwrap_or(0), 0);
    for _ in 0..body.get("ai_calls").and_then(Value::as_u64).unwrap_or(0) {
        run.used_ai();
    }
    let error = text("error").map(str::to_string);
    host.svc
        .db
        .transaction(move |tx| {
            oxplow_db::extension_run_store::record_tx(tx, &run, outcome, error.as_deref())
        })
        .await
        .map_err(|e| e.to_string())
}

/// `health: { extension, contribution, kind?, disable }`: the contribution
/// disabled after failures, with `disable` as the reason
/// (`contribution_health_store::disable_tx`).
pub(super) async fn observe_health(host: &Host, body: &Value) -> Result<(), String> {
    let text = |k: &str| body.get(k).and_then(Value::as_str);
    let key = oxplow_db::ContributionKey {
        extension: text("extension").ok_or("`health.extension`")?.to_string(),
        contribution: text("contribution")
            .ok_or("`health.contribution`")?
            .to_string(),
        kind: match text("kind").unwrap_or("collector") {
            "collector" => "collector",
            "effect" => "effect",
            "provider" => "provider",
            other => {
                return Err(format!(
                    "`health.kind: {other}` — fix: collector, effect or provider"
                ))
            }
        },
    };
    let reason = text("disable")
        .ok_or("`health.disable` is the reason it was disabled")?
        .to_string();
    let now = oxplow_domain::Timestamp::now().to_string();
    host.svc
        .db
        .transaction(move |tx| {
            oxplow_db::contribution_health_store::disable_tx(tx, &key, &reason, &now)
        })
        .await
        .map_err(|e| e.to_string())
}

/// The id (`thr3`) or `thr1` of the thread a `thread` key names.
pub(super) fn thread_text(body: &Value) -> String {
    body.get("thread")
        .and_then(Value::as_str)
        .unwrap_or("thr1")
        .to_string()
}

/// `token_usage: { thread?, effort?, session?, model?, prompt?, input_tokens?,
/// output_tokens?, cache_creation_input_tokens?, cache_read_input_tokens?,
/// turn? }`: the tokens a turn spent (`SqliteTokenUsageStore::record`).
pub(super) async fn observe_token_usage(host: &Host, body: &Value) -> Result<(), String> {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let int = |k: &str| body.get(k).and_then(Value::as_i64).unwrap_or(0);
    let stream = first_stream(host).await?;
    host.svc
        .token_usage_store
        .record(oxplow_db::NewAgentTokenUsage {
            stream_id: stream.to_string(),
            thread_id: thread_text(body),
            effort_id: text("effort"),
            session_id: text("session").unwrap_or_else(|| "s".into()),
            agent_kind: text("agent").unwrap_or_else(|| "claude".into()),
            model: text("model").or_else(|| Some("m".into())),
            prompt: text("prompt"),
            input_tokens: int("input_tokens"),
            output_tokens: int("output_tokens"),
            cache_creation_input_tokens: int("cache_creation_input_tokens"),
            cache_read_input_tokens: int("cache_read_input_tokens"),
            message_count: body
                .get("message_count")
                .and_then(Value::as_i64)
                .unwrap_or(1),
            turn_id: body.get("turn").and_then(Value::as_i64),
            cause: None,
        })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `hint: { hint, evaluated?, thread? }`: the hint's query ran `evaluated`
/// times (1) on the thread (`SqliteAgentNudgeStore::evaluated`).
pub(super) async fn observe_hint(host: &Host, body: &Value) -> Result<(), String> {
    let hint = body
        .get("hint")
        .and_then(Value::as_str)
        .ok_or("`hint.hint` names the hint")?;
    let thread = oxplow_domain::ThreadId::try_from_str(&thread_text(body))
        .ok_or("`hint.thread` is a thread, like `thr1`")?;
    for _ in 0..body.get("evaluated").and_then(Value::as_u64).unwrap_or(1) {
        host.svc
            .nudge_store
            .evaluated(thread.value(), vec![hint.to_string()])
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `nudge: { kind, message, thread?, effort?, audience?, delivered? }`: a
/// nudge a hint raised (`SqliteAgentNudgeStore::record`), delivered or not.
pub(super) async fn observe_nudge(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::agent_nudge_store::{Audience, NewAgentNudge};
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let id = host
        .svc
        .nudge_store
        .record(NewAgentNudge {
            thread_id: thread_text(body),
            effort_id: text("effort"),
            kind: text("kind").ok_or("`nudge.kind`")?,
            message: text("message").unwrap_or_default(),
            trigger: None,
            turn_id: None,
            cause: None,
            audience: match text("audience").as_deref() {
                Some("person") => Audience::Person,
                _ => Audience::Agent,
            },
        })
        .await
        .map_err(|e| e.to_string())?;
    if let (Some(id), true) = (
        id,
        body.get("delivered")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    ) {
        host.svc
            .nudge_store
            .mark_delivered(vec![id])
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `analyze: { kind: working, streamId } | { kind: commit, sha, streamId? } |
/// …`: the change is made and analyzed to the end
/// (`change_analysis::ensure_change`, what `oxplow_ensure_change` calls), so a
/// page reads its files and functions without waiting for the background
/// analysis. `streamId` of `working` defaults to the primary stream's.
pub(super) async fn observe_analyze(host: &Host, body: &Value) -> Result<(), String> {
    let mut target = body.clone();
    if target.get("kind").and_then(Value::as_str) == Some("working")
        && target.get("streamId").is_none()
    {
        let stream = first_stream(host).await?;
        target["streamId"] = json!(stream.to_string());
    }
    let target: oxplow_app::change_analysis::ChangeTarget =
        serde_json::from_value(target).map_err(|e| format!("`analyze`: {e}"))?;
    oxplow_app::change_analysis::ensure_change(&host.svc, target)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `refs: { branches?: [{ name, kind?, head, stream?, default?, upstream?,
/// ahead?, behind? }], tags?: [{ name, sha }], heads?: [{ stream, sha }] }`:
/// the repository's refs as the git indexer stores them
/// (`SqliteGitStore::replace_branches` / `replace_tags` /
/// `replace_stream_heads`); a part left out keeps what was stored.
pub(super) async fn observe_refs(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::git_store::{GitBranchRow, GitTagRow};
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let list = |k: &str| body.get(k).and_then(Value::as_array).cloned();
    let git = &host.svc.git_store;
    if let Some(branches) = list("branches") {
        let mut rows = Vec::new();
        for b in &branches {
            rows.push(GitBranchRow {
                name: text(b, "name").ok_or("a branch names its `name`")?,
                kind: text(b, "kind").unwrap_or_else(|| "local".into()),
                remote: text(b, "remote"),
                head_sha: text(b, "head"),
                stream_id: b.get("stream").and_then(Value::as_i64),
                is_default: b.get("default").and_then(Value::as_bool).unwrap_or(false),
                upstream: text(b, "upstream"),
                ahead: b.get("ahead").and_then(Value::as_u64).unwrap_or(0) as u32,
                behind: b.get("behind").and_then(Value::as_u64).unwrap_or(0) as u32,
            });
        }
        git.replace_branches(rows)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(tags) = list("tags") {
        let mut rows = Vec::new();
        for t in &tags {
            rows.push(GitTagRow {
                name: text(t, "name").ok_or("a tag names its `name`")?,
                sha: text(t, "sha").ok_or("a tag names its `sha`")?,
            });
        }
        git.replace_tags(rows).await.map_err(|e| e.to_string())?;
    }
    if let Some(heads) = list("heads") {
        let mut rows = Vec::new();
        for h in &heads {
            rows.push((
                h.get("stream")
                    .and_then(Value::as_i64)
                    .ok_or("a head names its `stream`")?,
                text(h, "sha").ok_or("a head names its `sha`")?,
            ));
        }
        git.replace_stream_heads(rows)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// `visit: { page, kind?, label?, thread?, duration_ms? }`: a page the person
/// opened (`page` its ref, `kind` its page kind, `page` by default), recorded
/// as the shell's `record_page_visit` does (`PageVisitStore::record`).
/// `conversation: { harness, id, title?, first_prompt?, started_at?,
/// updated_at?, size_bytes?, parent? }`: a conversation `harness` saved, as it
/// lists it in `v_agent_conversation`; `{ harness, unavailable: <why> }` is
/// one that can't list. A test's oxplow lists only these
/// (`SavedConversations::stand_in`).
pub(super) async fn observe_conversation(host: &Host, body: &Value) -> Result<(), String> {
    const KEYS: [&str; 9] = [
        "harness",
        "id",
        "title",
        "first_prompt",
        "started_at",
        "updated_at",
        "size_bytes",
        "parent",
        "unavailable",
    ];
    for key in body.as_object().into_iter().flat_map(|m| m.keys()) {
        if !KEYS.contains(&key.as_str()) {
            return Err(format!(
                "`conversation.{key}` — fix: one of {}",
                KEYS.map(|k| format!("`{k}`")).join(", ")
            ));
        }
    }
    let text = |key: &str| body.get(key).and_then(Value::as_str).map(str::to_string);
    let harness = text("harness").ok_or("`conversation.harness` is the harness that saved it")?;
    host.svc
        .harnesses
        .get(&harness)
        .map_err(|e| format!("`conversation.harness`: {e}"))?;
    let saved = &host.svc.saved_conversations;
    if let Some(why) = text("unavailable") {
        saved.unavailable(&harness, &why);
        return Ok(());
    }
    let time = |key: &str| match text(key) {
        Some(t) => oxplow_domain::Timestamp::parse(&t)
            .map(Some)
            .map_err(|e| format!("`conversation.{key}`: {e}")),
        None => Ok(None),
    };
    saved.give(
        &harness,
        oxplow_domain::agent::harness::NativeConversation {
            id: text("id").ok_or("`conversation.id` is its id, what a resume names")?,
            title: text("title"),
            first_prompt: text("first_prompt"),
            started_at: time("started_at")?,
            updated_at: time("updated_at")?,
            size_bytes: body.get("size_bytes").and_then(Value::as_u64),
            parent: text("parent"),
        },
    );
    Ok(())
}

pub(super) async fn observe_visit(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::analytics_stores::PageVisitStore as _;
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    let page = text("page").ok_or("`visit.page` is the page's ref")?;
    host.svc
        .page_visit_store
        .record(
            text("kind").unwrap_or("page"),
            page,
            text("label"),
            body.get("duration_ms").and_then(Value::as_i64),
            text("thread"),
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `facts: { measure, producer?, items?: [{ value?, path?, line?, rule?,
/// severity?, detail?, subject?, dims? }] }`: findings of `measure` (declared by
/// core or an extension), recorded as one capture
/// (`SqliteFactStore::record_facts`), the way a collector's run records them.
pub(super) async fn observe_facts(host: &Host, body: &Value) -> Result<(), String> {
    let key = body
        .get("measure")
        .and_then(Value::as_str)
        .ok_or("`facts.measure` names the measure")?;
    let producer = body.get("producer").and_then(Value::as_str).unwrap_or(key);
    let facts = &host.svc.fact_store;
    let measure = facts
        .get_measure(key)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no measure `{key}` — fix: declare it in `measures:`"))?;
    let rows: Vec<oxplow_db::NewFact> = body
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| {
            let text = |k: &str| f.get(k).and_then(Value::as_str).map(str::to_string);
            oxplow_db::NewFact {
                subject: text("subject"),
                path: text("path"),
                line: f.get("line").and_then(Value::as_i64),
                severity: text("severity"),
                rule: text("rule"),
                detail: text("detail"),
                dims_json: f.get("dims").map(Value::to_string),
                ..oxplow_db::NewFact::new(
                    measure.id,
                    f.get("value").and_then(Value::as_f64).unwrap_or(1.0),
                )
            }
        })
        .collect();
    let stream = first_stream(host).await?;
    facts
        .record_facts(
            oxplow_db::NewMetricCapture::done(stream.value(), producer, "hook"),
            rows,
        )
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `retention: { days }`: the event log's retention sweep
/// (`event_retention::sweep`) as it runs `days` from now — a payload past
/// its kind's keep expires, its envelope (type, subject, cause) stays.
pub(super) async fn observe_retention(host: &Host, body: &Value) -> Result<(), String> {
    let days = body
        .get("days")
        .and_then(Value::as_i64)
        .ok_or("`retention.days` is how many days ahead the sweep runs")?;
    let later = oxplow_domain::Timestamp::from_unix_ms(
        oxplow_domain::Timestamp::now().unix_ms() + days * oxplow_db::event_retention::DAY_MS,
    );
    oxplow_db::event_retention::sweep(&host.svc.db, later, &Default::default())
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// `collect: { effort, collectors?: [key] }`: the fact collectors an
/// `effort.finished` triggers (`MetricsService::run_effort_collectors`),
/// run for the effort now; `collectors` names the ones to run (by key),
/// else every one runs.
pub(super) async fn observe_collect(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_db::effort_store::EffortStore as _;
    use oxplow_domain::EffortId;
    let effort = body
        .get("effort")
        .and_then(Value::as_str)
        .and_then(|e| e.strip_prefix("eff"))
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or("`collect.effort` is an effort's id, like `eff1`")?;
    let found = host
        .svc
        .effort_store
        .get_effort(&EffortId::new(effort))
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no effort eff{effort}"))?;
    let wanted: Option<Vec<String>> = body.get("collectors").and_then(Value::as_array).map(|l| {
        l.iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect()
    });
    host.svc
        .metrics
        .run_effort_collectors(&found.thread_id, &found.id, None, &|c| {
            wanted.as_ref().is_none_or(|w| w.contains(&c.key))
        })
        .await;
    Ok(())
}

/// `snapshot: { trigger?, elapsed_ms?, budget_ms?, revision?, branch?, files?: [{ path, hash }] }`:
/// one take in the stream's snapshot log (`SqliteSnapshotStore::
/// record_take`): its parent is the take before it, and it is over budget
/// when `elapsed_ms` is more than `budget_ms`. A take whose files hash as
/// its parent's makes no new snapshot.
pub(super) async fn observe_snapshot(host: &Host, body: &Value) -> Result<(), String> {
    take_snapshot(host, body).await.map(|_| ())
}

pub(super) async fn take_snapshot(
    host: &Host,
    body: &Value,
) -> Result<Option<oxplow_db::TakeOutcome>, String> {
    use oxplow_db::{FileSnapshot, SnapshotStorage, TakeRecord};
    use oxplow_domain::snapshot::SnapshotTrigger;
    let stream = first_stream(host).await?;
    let trigger = match body.get("trigger").and_then(Value::as_str) {
        Some(t) => SnapshotTrigger::from_db_str(t)
            .ok_or_else(|| format!("`{t}` isn't a snapshot trigger"))?,
        None => SnapshotTrigger::Manual,
    };
    let mut rows = Vec::new();
    for f in body
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| f.get(key).and_then(Value::as_str);
        rows.push(FileSnapshot {
            id: 0,
            stream_id: stream,
            path: text("path")
                .ok_or("a snapshot file names its `path`")?
                .to_string(),
            blob_hash: Some(text("hash").unwrap_or("h").to_string()),
            size_bytes: 1,
            captured_at: oxplow_domain::Timestamp::now(),
            storage: SnapshotStorage::Oxplow,
            snapshot_id: None,
            mtime_ms: Some(1),
            content_hash: None,
        });
    }
    host.svc
        .snapshot_store
        .record_take(TakeRecord {
            stream_id: stream,
            rows,
            trigger,
            thread_id: None,
            turn_id: None,
            effort_id: None,
            branch: body
                .get("branch")
                .and_then(Value::as_str)
                .map(str::to_string),
            revision: match body.get("revision").and_then(Value::as_str) {
                Some(r) => Some(r.parse().map_err(|e| format!("`snapshot.revision`: {e}"))?),
                None => None,
            },
            elapsed_ms: body.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(1),
            budget_ms: body.get("budget_ms").and_then(Value::as_u64),
            source: "fixture".into(),
            provider: None,
            contents: false,
            handle: None,
        })
        .await
        .map_err(|e| e.to_string())
}

/// `commit: { sha, at, subject, author?, email?, body?, parents?, files?:
/// [{ path, status?, additions?, deletions? }] }`: a commit the git indexer
/// read, stored as it stores one (`SqliteGitStore::upsert_commit`).
pub(super) async fn observe_commit(host: &Host, body: &Value) -> Result<(), String> {
    let text = |key: &str| body.get(key).and_then(Value::as_str);
    let at = text("at").ok_or("`commit.at` is when it was committed (RFC 3339)")?;
    let committed = oxplow_domain::Timestamp::parse(at).map_err(|e| format!("`commit.at`: {e}"))?;
    let mut files = Vec::new();
    for f in body
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        files.push(oxplow_db::git_store::GitCommitFileRow {
            path: f
                .get("path")
                .and_then(Value::as_str)
                .ok_or("a commit file names its `path`")?
                .to_string(),
            status: f
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("M")
                .to_string(),
            additions: f.get("additions").and_then(Value::as_i64).unwrap_or(0),
            deletions: f.get("deletions").and_then(Value::as_i64).unwrap_or(0),
        });
    }
    host.svc
        .git_store
        .upsert_commit(oxplow_db::git_store::GitCommitRow {
            sha: text("sha").ok_or("`commit.sha` names it")?.to_string(),
            author: text("author").unwrap_or("fixture").to_string(),
            email: text("email").unwrap_or("fixture@example.test").to_string(),
            committed_secs: committed.unix_ms() / 1000,
            subject: text("subject").unwrap_or_default().to_string(),
            body: text("body").unwrap_or_default().to_string(),
            parents: body
                .get("parents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(str::to_string))
                .collect(),
            files,
        })
        .await
        .map_err(|e| e.to_string())
}

/// `evidence: { effort, observations: [{ kind, metric_value?, payload?,
/// provenance?, source? }] }`: what an effort's evidence holds — its
/// `test-run`s and `diff-coverage` — as the evidence store keeps it
/// (`EffortEvidenceStore::replace`, what an effort's refresh calls); the
/// observations are the effort's whole set.
pub(super) async fn observe_evidence(host: &Host, body: &Value) -> Result<(), String> {
    let effort = body
        .get("effort")
        .and_then(Value::as_str)
        .and_then(|e| e.strip_prefix("eff"))
        .and_then(|n| n.parse::<i64>().ok())
        .ok_or("`evidence.effort` is an effort's id, like `eff1`")?;
    let mut observations = Vec::new();
    for o in body
        .get("observations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str, default: &str| {
            o.get(key)
                .and_then(Value::as_str)
                .unwrap_or(default)
                .to_string()
        };
        observations.push(oxplow_db::EffortObservation {
            kind: o
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("an observation names its `kind`")?
                .to_string(),
            provenance: text("provenance", "observed"),
            source: text("source", "fixture"),
            metric_value: o.get("metric_value").and_then(Value::as_f64),
            payload_json: o.get("payload").map(Value::to_string),
            local_snapshot_id: None,
            created_at: oxplow_domain::Timestamp::now(),
        });
    }
    let mut deltas = Vec::new();
    for d in body
        .get("metric_deltas")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| d.get(key).and_then(Value::as_str).map(str::to_string);
        let num = |key: &str| d.get(key).and_then(Value::as_f64);
        let key = text("key").ok_or("a metric delta names its `key`")?;
        let current = num("current").ok_or("a metric delta names its `current`")?;
        deltas.push(oxplow_db::EffortMetricDelta {
            title: text("title").unwrap_or_else(|| key.clone()),
            key,
            unit: text("unit"),
            direction: text("direction").unwrap_or_else(|| "lower-better".into()),
            kind: text("kind").unwrap_or_else(|| "gauge".into()),
            category: text("category"),
            language: text("language"),
            agg: text("agg").unwrap_or_else(|| "files".into()),
            baseline: num("baseline"),
            current,
            delta: num("delta").or_else(|| num("baseline").map(|b| current - b)),
            changed: d.get("changed").and_then(Value::as_bool).unwrap_or(true),
            attributed_files: d.get("attributed_files").and_then(Value::as_i64),
            sample_count: d.get("sample_count").and_then(Value::as_i64).unwrap_or(1),
            target: num("target"),
            warn_at: num("warn_at"),
            fail_at: num("fail_at"),
            crossing: text("crossing"),
            latest_capture_id: None,
        });
    }
    host.svc
        .effort_evidence_store
        .replace(effort, deltas, observations, String::new())
        .await
        .map_err(|e| e.to_string())
}

pub(super) async fn observe_turn(host: &Host, body: &Value) -> Result<(), String> {
    use oxplow_app::hook_ingest::{HookEnvelope, LAST_ASSISTANT_MESSAGE};
    use oxplow_domain::hook::HookKind;
    for key in body.as_object().into_iter().flat_map(|m| m.keys()) {
        if ![
            "prompt",
            "during",
            "tools",
            "write",
            "answer",
            "compactions",
            "context",
            "thread",
            "session",
            "subagent",
            "peer",
            "conversation",
        ]
        .contains(&key.as_str())
        {
            return Err(format!(
                "`turn.{key}` — fix: `prompt`, `during`, `tools`, `write`, `answer`, `compactions`, `context`, `thread`, `session`, `subagent`, `peer` or `conversation`"
            ));
        }
    }
    let svc = &host.svc;
    let thread = oxplow_domain::ThreadId::try_from_str(&thread_text(body))
        .ok_or("`turn.thread` is a thread, like `thr2`")?;
    let session = match body.get("session").and_then(Value::as_str) {
        Some(s) => Some(
            oxplow_domain::AgentSessionId::try_from_str(s)
                .ok_or("`turn.session` is an agent session, like `ses2`")?,
        ),
        None => None,
    };
    // The conversation the hooks name, which the session is bound to; none
    // with `conversation: null`.
    let conversation = match body.get("conversation") {
        None => Some("fixture".to_string()),
        Some(Value::Null) => None,
        Some(c) => Some(
            c.as_str()
                .ok_or("`turn.conversation` is the harness's id for it, or null")?
                .to_string(),
        ),
    };
    let envelope = |kind: HookKind, payload: Value| HookEnvelope {
        session_title: None,
        turn_run: None,
        kind,
        thread_id: Some(thread),
        stream_id: None,
        agent_session_id: session,
        session_id: conversation.clone(),
        payload_json: payload.to_string(),
        prompt: None,
        decision: None,
        tool: None,
        subagent: None,
        peer: None,
    };
    let ingest = |env: HookEnvelope| async move {
        svc.hook_ingest
            .ingest(env)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    };
    // A turn that writes starts from a snapshot of the tree as it is.
    let capture = match body.get("write") {
        Some(_) => {
            let stream = svc
                .streams
                .list_streams()
                .await
                .map_err(|e| e.to_string())?
                .first()
                .map(|s| s.id)
                .ok_or("no stream")?;
            let capture = svc
                .snapshot_captures
                .get(&stream)
                .ok_or("no snapshot capture")?;
            capture
                .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Startup)
                .await
                .map_err(|e| e.to_string())?;
            Some(capture)
        }
        None => None,
    };
    let mut prompt = envelope(HookKind::UserPromptSubmit, json!({}));
    // No prompt (`prompt: null`) is a subagent's or a peer's hand-back.
    prompt.prompt = match body.get("prompt") {
        Some(Value::Null) => None,
        Some(p) => Some(p.as_str().unwrap_or_default().to_string()),
        None => Some("Work on it".to_string()),
    };
    if let Some(sub) = body.get("subagent") {
        prompt.subagent =
            Some(serde_json::from_value(sub.clone()).map_err(|e| format!("`turn.subagent`: {e}"))?);
    }
    prompt.peer = body.get("peer").and_then(Value::as_str).map(str::to_string);
    ingest(prompt).await?;
    for step in body
        .get("during")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let command = step
            .get("command")
            .and_then(Value::as_str)
            .ok_or("`turn.during` steps are `{ command, input?, as? }`")?;
        svc.commands
            .run(
                &step_actor(step)?,
                command,
                step.get("input").cloned().unwrap_or_else(|| json!({})),
                step.get("confirmed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            )
            .await
            .map_err(|e| format!("`turn.during` `{command}` failed: {e}"))?;
    }
    for tool in body
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        // `pre: true` is the call as it starts (PreToolUse), not as it ends.
        let mut tool = tool.clone();
        let pre = tool
            .as_object_mut()
            .and_then(|m| m.remove("pre"))
            .and_then(|p| p.as_bool())
            .unwrap_or(false);
        let used: oxplow_domain::agent::tool::ToolUse =
            serde_json::from_value(tool).map_err(|e| format!("a tool: {e}"))?;
        let mut call = envelope(
            if pre {
                HookKind::PreToolUse
            } else {
                HookKind::PostToolUse
            },
            json!({}),
        );
        call.tool = Some(used);
        ingest(call).await?;
    }
    // The calls are in the datasets before the turn ends, as the app's pump
    // has them by then: the end's checkpoint counts them.
    svc.event_pump
        .deliver_projections()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(write) = body.get("write") {
        let (Some(path), Some(text)) = (
            write.get("path").and_then(Value::as_str),
            write.get("body").and_then(Value::as_str),
        ) else {
            return Err("`turn.write` is `{ path, body }`".into());
        };
        let file = svc.layout.project_dir.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        capture
            .ok_or("no snapshot capture")?
            .enqueue_startup_diff()
            .await
            .map_err(|e| e.to_string())?;
    }
    for _ in 0..body.get("compactions").and_then(Value::as_u64).unwrap_or(0) {
        ingest(envelope(HookKind::PostCompact, json!({}))).await?;
    }
    let mut payload = json!({});
    if let Some(answer) = body.get("answer") {
        payload[LAST_ASSISTANT_MESSAGE] = answer.clone();
    }
    let mut stop = envelope(HookKind::Stop, payload);
    if let Some(context) = body.get("context") {
        stop.turn_run = Some(oxplow_domain::hook::TurnRun {
            context_tokens: context.get("tokens").and_then(Value::as_i64),
            context_window: context.get("window").and_then(Value::as_i64),
            ..Default::default()
        });
    }
    ingest(stop).await
}
