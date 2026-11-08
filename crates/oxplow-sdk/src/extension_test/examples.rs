use super::*;

/// Log the fixture's event (`when: { event: { type, payload, subject?,
/// anchors? }, model? }`) in the throwaway and deliver it: the extension's
/// effects — running in the throwaway host — react for real, their
/// commands run through the bus. What happened against `expect`:
/// `{ effects: [{ id, state? }], commands: [names] }` (`$any` matches).
pub(super) async fn event_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    if let Err(e) = start_effects(host, ext).await {
        report.errors.push(error(
            shown,
            None,
            "example.effects_failed",
            format!("starting its effects: {e}"),
        ));
        return;
    }
    let Some(view) = example_event(host, ex, report) else {
        return;
    };
    let mut env = match oxplow_domain::events::Envelope::new(
        view.event_type.clone(),
        view.v,
        "fixture",
        view.payload.clone(),
    ) {
        Ok(env) => env,
        Err(e) => {
            report.errors.push(error(
                shown,
                None,
                "fixture.event",
                format!("the event: {e}"),
            ));
            return;
        }
    };
    env.subject = view.subject.clone();
    env.anchors = view.anchors.clone();
    let seq = match host.svc.event_log_store.append(env).await {
        Ok(seq) => seq,
        Err(e) => {
            report.errors.push(error(
                shown,
                None,
                "example.event_failed",
                format!("logging the event: {e}"),
            ));
            return;
        }
    };
    let (effects, commands) = effect_outcome(host, seq, "=").await;
    let got = json!({ "effects": effects, "commands": commands });
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("the event led to {got} at `{path}`, the example expects {want}")).fix("the effects, or the example's `expect` (`{ effects: [{ id, state }], commands: [names] }`)").mismatch(path, want, got));
    }
}

/// `commands` as audited (name, audit id): a sequence — how an effect's
/// commands run — is replaced by its steps, in order.
pub(super) async fn without_sequences(
    host: &Host,
    audited: Vec<(String, Option<i64>)>,
) -> Vec<String> {
    let audits = oxplow_db::SqliteCommandAuditStore::new(host.svc.db.clone());
    let mut commands = Vec::new();
    for (name, id) in audited {
        let steps = match (id, name == "oxplow.command.sequence") {
            (Some(id), true) => audits
                .get(id)
                .await
                .ok()
                .flatten()
                .and_then(|a| a.result.and_then(|r| r.get("children").cloned())),
            _ => None,
        };
        match steps {
            Some(Value::Array(children)) => commands.extend(
                children
                    .iter()
                    .filter_map(|c| c.get("name").and_then(Value::as_str))
                    .map(str::to_string),
            ),
            _ => commands.push(name),
        }
    }
    commands
}

/// What the effects did after log position `seq`: each latest run on the
/// events at it (`at` is `=`: one event) or past it (`>`: whatever a job
/// logged), and the commands they ran, once the effects have heard the log.
pub(super) async fn effect_outcome(host: &Host, seq: i64, at: &str) -> (Vec<Value>, Vec<String>) {
    host.svc
        .event_pump
        .settle(
            &[oxplow_app::effect_triggers::NAME],
            std::time::Duration::from_secs(10),
        )
        .await;
    let rows = |sql: &str| {
        let sql = sql.to_string();
        let svc = host.svc.clone();
        async move {
            svc.sql
                .query_sql(&sql, vec![oxplow_db::SqlCell::Int(seq)], None)
                .await
                .map(|r| r.rows)
                .unwrap_or_default()
        }
    };
    let text = |c: &oxplow_db::SqlCell| match c {
        oxplow_db::SqlCell::Text(t) => t.clone(),
        other => format!("{other:?}"),
    };
    let effects: Vec<Value> = rows(&format!(
        "SELECT effect, state, reason FROM v_effect_run WHERE event_seq {at} ?1 AND latest = 1 \
         ORDER BY effect"
    ))
    .await
    .iter()
    .map(|r| json!({ "id": text(&r[0]), "state": text(&r[1]), "reason": text(&r[2]) }))
    .collect();
    let ran = rows(
        "SELECT json_extract(payload, '$.command'), json_extract(payload, '$.audit_id') FROM v_event \
         WHERE type = 'command.executed' AND seq > ?1 \
           AND json_extract(payload, '$.actor_kind') = 'effect' ORDER BY seq",
    )
    .await;
    let audited = ran
        .iter()
        .map(|r| match &r[1] {
            oxplow_db::SqlCell::Int(id) => (text(&r[0]), Some(*id)),
            _ => (text(&r[0]), None),
        })
        .collect();
    let commands = without_sequences(host, audited).await;
    (effects, commands)
}

/// Start `ext`'s effects at the log's head, so they react to what the
/// example logs next.
pub(super) async fn start_effects(host: &Host, ext: &Extension) -> Result<(), String> {
    for decl in &ext.effects {
        oxplow_app::effects::start_at_head(&host.svc.db, &decl.name())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Record intent for job `id` through the extension's own command over
/// `jobs.write/enqueue` (`when: { job, intentKey, request, model?,
/// setup?, rows? }`: `setup` and `rows` first make what its tools read), the
/// job running in the throwaway host, and read its outcome through its
/// `jobs.read/snapshot` command once it ends: against `expect` — `{ state,
/// result?, text?, toolCalls?, output?, effects?, commands?, pages? }`
/// (`$any` matches): its streamed `text`, how many tools its model called
/// and its output rows (`[{ kind, … }]`, `v_job_run_output`). Its model calls
/// answer with the fixture model's replies; with `cancel: true` in its
/// input, the job is cancelled through the extension's `jobs.write/cancel`
/// command once its first output is written. An expectation of `effects`,
/// `commands` or `pages` delivers the `contribution.job.completed` the job
/// logs to the extension's effects, running there too: the
/// effects that reacted (`[{ id, state? }]`), the commands they ran
/// (`[names]`) and which of the named wiki pages (`[uri]`) now exist.
pub(super) async fn job_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::extension::extension_commands::CommandHandler;
    let shown = ex.shown;
    let by_op = |scope: &str, op: &str| {
        ext.commands
            .iter()
            .find(|c| matches!(&c.handler, CommandHandler::Scope { scope: s, op: o } if s == scope && o == op))
            .map(|c| c.name.clone())
    };
    let (Some(enqueue), Some(snapshot)) = (
        by_op("jobs.write", "enqueue"),
        by_op("jobs.read", "snapshot"),
    ) else {
        report.errors.push(error(shown, None, "example.job_needs_command", format!("job `{id}`: the extension declares no command over `jobs.write/enqueue` and `jobs.read/snapshot` to run it with")).fix("declare both (`scope: jobs.write, op: enqueue`; `scope: jobs.read, op: snapshot`)"));
        return;
    };
    let delivers = ["effects", "commands", "pages"]
        .iter()
        .any(|k| ex.expect.get(k).is_some());
    if delivers {
        if let Err(e) = start_effects(host, ext).await {
            report.errors.push(error(
                shown,
                None,
                "example.effects_failed",
                format!("starting its effects: {e}"),
            ));
            return;
        }
    }
    if !seed_rows(host, ext, ex, report).await || !run_setup(host, ext, ex, report).await {
        return;
    }
    let logged = host
        .svc
        .sql
        .query_sql("SELECT COALESCE(MAX(seq), 0) FROM v_event", vec![], None)
        .await
        .ok()
        .and_then(|r| match r.rows.first().and_then(|row| row.first()) {
            Some(oxplow_db::SqlCell::Int(n)) => Some(*n),
            _ => None,
        })
        .unwrap_or(0);
    let input = json!({
        "job": id,
        "intentKey": ex.when.get("intentKey").cloned().unwrap_or_else(|| json!(ex.name)),
        "request": ex.when.get("request").cloned().unwrap_or(Value::Null),
    });
    // A job asked for in a thread: the effects that react to its end run
    // for that thread, as a command that changes the workspace needs.
    let asker = if delivers {
        Actor::Agent {
            session_id: None,
            thread_id: Some(oxplow_domain::ThreadId::new(1)),
            stream_id: None,
        }
    } else {
        Actor::Human
    };
    let queued = match host.svc.commands.run(&asker, &enqueue, input, false).await {
        Ok(out) => out.result,
        Err(e) => {
            report.errors.push(error(
                shown,
                None,
                "example.job_failed",
                format!("job `{id}` wasn't recorded: {e}"),
            ));
            return;
        }
    };
    let Some(job) = queued.get("jobRun").and_then(Value::as_str).and_then(|r| {
        oxplow_domain::uri::build::id_of(r, oxplow_domain::uri::resources::JOB_RUN).ok()
    }) else {
        report.errors.push(error(
            shown,
            None,
            "example.job_failed",
            format!("job `{id}`: {queued} names no job"),
        ));
        return;
    };
    if ex.when.get("cancel") == Some(&json!(true)) {
        let Some(cancel) = by_op("jobs.write", "cancel") else {
            report.errors.push(
                error(
                    shown,
                    None,
                    "example.job_cancel_needs_command",
                    format!("job `{id}`: `cancel: true` needs a command over `jobs.write/cancel`"),
                )
                .fix("declare one (`scope: jobs.write, op: cancel`)"),
            );
            return;
        };
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let snap = host
                .svc
                .commands
                .run(
                    &Actor::Human,
                    &snapshot,
                    json!({ "job_run": &job.to_string() }),
                    false,
                )
                .await
                .map(|out| out.result)
                .unwrap_or(Value::Null);
            let started = snap
                .get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| !t.is_empty());
            let ended = snap.get("state").and_then(Value::as_str) != Some("running")
                && snap.get("state").and_then(Value::as_str) != Some("queued");
            if started || ended || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        if let Err(e) = host
            .svc
            .commands
            .run(
                &asker,
                &cancel,
                json!({ "job_run": &job.to_string() }),
                false,
            )
            .await
        {
            report.errors.push(error(
                shown,
                None,
                "example.job_failed",
                format!("cancelling job `{id}`: {e}"),
            ));
            return;
        }
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    let done = loop {
        let state = host
            .svc
            .commands
            .run(
                &Actor::Human,
                &snapshot,
                json!({ "job_run": &job.to_string() }),
                false,
            )
            .await
            .map(|out| out.result);
        match state {
            Ok(s)
                if ["succeeded", "failed", "cancelled", "interrupted"]
                    .contains(&s.get("state").and_then(Value::as_str).unwrap_or("")) =>
            {
                break s
            }
            Ok(s) if tokio::time::Instant::now() >= deadline => break s,
            Ok(_) => tokio::time::sleep(std::time::Duration::from_millis(50)).await,
            Err(e) => {
                report.errors.push(error(
                    shown,
                    None,
                    "example.job_failed",
                    format!("job `{id}`'s snapshot: {e}"),
                ));
                return;
            }
        }
    };
    let output = host
        .svc
        .sql
        .query_sql(
            "SELECT kind, data FROM v_job_run_output WHERE job_id = ?1 ORDER BY attempt, seq",
            vec![oxplow_db::SqlCell::Text(job.clone())],
            None,
        )
        .await
        .map(|r| {
            r.rows
                .iter()
                .map(|row| {
                    let mut out = match row.get(1) {
                        Some(oxplow_db::SqlCell::Text(data)) => {
                            serde_json::from_str(data).unwrap_or_else(|_| json!({}))
                        }
                        _ => json!({}),
                    };
                    if let (Some(oxplow_db::SqlCell::Text(kind)), Some(obj)) =
                        (row.first(), out.as_object_mut())
                    {
                        obj.insert("kind".into(), json!(kind));
                    }
                    out
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut ended = json!({
        "state": done.get("state"),
        "result": done.get("result"),
        "text": done.get("text"),
        "toolCalls": done.get("toolCalls"),
        "output": output,
    });
    if delivers {
        let (effects, commands) = effect_outcome(host, logged, ">").await;
        ended["effects"] = Value::Array(effects);
        ended["commands"] = json!(commands);
        let wanted: Vec<String> = ex
            .expect
            .get("pages")
            .and_then(Value::as_array)
            .map(|p| {
                p.iter()
                    .filter_map(|r| r.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut pages = Vec::new();
        for page in wanted {
            let found = host
                .svc
                .sql
                .query_sql(
                    "SELECT 1 FROM v_knowledge_page WHERE uri = ?1",
                    vec![oxplow_db::SqlCell::Text(page.clone())],
                    None,
                )
                .await
                .map(|r| !r.rows.is_empty())
                .unwrap_or(false);
            if found {
                pages.push(page);
            }
        }
        ended["pages"] = json!(pages);
    }
    let got = ended;
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("job `{id}` ended {got} at `{path}`, the example expects {want}")).fix("the job's request, or the example's `expect` (`{ state, result?, text?, toolCalls?, output?, effects?, commands?, pages? }`)").mismatch(path, want, got));
    }
}

/// Run `when: { command, input?, as?, confirmed? }` for real, through the
/// bus, and let what it set off settle: `{ ok: true, result }` or `{ ok:
/// false, refused }`, and `commands`, the ones that ran because of it (its
/// script's, an effect's), checked against `expect`.
pub(super) async fn command_run(
    host: &Host,
    ex: &Example<'_>,
    command: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    host.current().await;
    let actor = match step_actor(&ex.when) {
        Ok(actor) => actor,
        Err(e) => {
            return report
                .errors
                .push(error(shown, None, "example.failed", e.to_string()))
        }
    };
    let head = host
        .svc
        .sql
        .query_sql(
            "SELECT COALESCE(MAX(id), 0) FROM v_command_audit",
            vec![],
            None,
        )
        .await
        .ok()
        .and_then(|r| r.rows.first().and_then(|row| row.first()).cloned())
        .unwrap_or(oxplow_db::SqlCell::Int(0));
    let outcome = host
        .svc
        .commands
        .run(
            &actor,
            command,
            ex.when.get("input").cloned().unwrap_or_else(|| json!({})),
            ex.when
                .get("confirmed")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        )
        .await;
    host.settle().await;
    // A composite (a script's command) answers `{ result, children }`:
    // its own answer, and the commands it ran.
    let mut composed = Vec::new();
    let mut got = match outcome {
        Ok(out) => match (out.result.get("result"), out.result.get("children")) {
            (Some(result), Some(Value::Array(children))) => {
                composed = children
                    .iter()
                    .filter_map(|c| c.get("name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect();
                json!({ "ok": true, "result": result })
            }
            _ => json!({ "ok": true, "result": out.result }),
        },
        Err(e) => json!({ "ok": false, "refused": e.to_string() }),
    };
    let mut audited: Vec<(String, Option<i64>)> = host
        .svc
        .sql
        .query_sql(
            "SELECT command, id FROM v_command_audit WHERE id > ?1 ORDER BY id",
            vec![head],
            None,
        )
        .await
        .map(|r| r.rows)
        .unwrap_or_default()
        .iter()
        .filter_map(|row| match row.as_slice() {
            [oxplow_db::SqlCell::Text(name), oxplow_db::SqlCell::Int(id)] => {
                Some((name.clone(), Some(*id)))
            }
            _ => None,
        })
        .collect();
    if let Some(own) = audited.iter().position(|(name, _)| name == command) {
        audited.remove(own);
    }
    let mut ran = without_sequences(host, audited).await;
    composed.append(&mut ran);
    let ran = composed;
    got["commands"] = json!(ran);
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("command `{command}` gave {at} at `{path}`, the example expects {want}")).fix("the command's input or the steps before it, or the example's `expect` (`{ ok, result?, commands? }` or `{ ok: false, refused }`)").mismatch(path, want, at));
    }
}

/// Run advisory `id` for a thread (`when: { advisory, thread? }`, the
/// primary thread when none) as the app does on its `on:` — over that
/// thread's open effort — and check what the agent is told, line by line:
/// `expect: { lines: [..] }` (none: `[]`).
pub(super) async fn advisory_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let Some(advisory) = ext.advisories.iter().find(|a| a.id == id) else {
        return report.errors.push(
            error(
                shown,
                None,
                "fixture.unknown",
                format!("`{id}` isn't one of the extension's advisories"),
            )
            .fix("the example's `when.advisory`"),
        );
    };
    let thread = match ex.when.get("thread").and_then(Value::as_str) {
        None => oxplow_domain::ThreadId::new(1),
        Some(t) => match oxplow_domain::ThreadId::try_from_str(t) {
            Some(thread) => thread,
            None => {
                return report.errors.push(
                    error(
                        shown,
                        None,
                        "fixture.unknown",
                        format!("`when.thread: {t}` isn't a thread"),
                    )
                    .fix("`thr1`"),
                )
            }
        },
    };
    if let Err(e) = host.svc.streams.ensure_primary().await {
        return report.errors.push(error(
            shown,
            None,
            "example.thread_failed",
            format!("its thread: {e}"),
        ));
    }
    host.current().await;
    let hits =
        oxplow_app::advisories::for_thread(&host.svc.advisory_deps(), &thread, advisory.on, None)
            .await;
    let lines: Vec<String> = hits
        .iter()
        .filter(|hit| hit.id == id || hit.id.ends_with(&format!("/{id}")))
        .flat_map(|hit| hit.text.lines().map(str::to_string).collect::<Vec<_>>())
        .collect();
    let got = json!({ "lines": lines });
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.compare",
                format!("advisory `{id}` said {at} at `{path}`, the example expects {want}"),
            )
            .fix("its query, or the example's `given` and `expect` (`{ lines: [..] }`)")
            .mismatch(path, want, at),
        );
    }
}

/// Read the published datasets with `when: { sql, params? }` as a page's
/// loader would — a dataset, a decorator's labels, a metric's grid — through
/// an inline page of one `sql` loader: `expect: { rows, columns? }`,
/// compared as a loader's.
pub(super) async fn sql_example(host: &Host, ex: &Example<'_>, sql: &str, report: &mut TestReport) {
    let shown = ex.shown;
    let params = ex.when.get("params").cloned().unwrap_or_else(|| json!({}));
    let declared: Vec<Value> = params
        .as_object()
        .into_iter()
        .flatten()
        .map(|(name, _)| json!({ "name": name }))
        .collect();
    let spec = json!({
        "title": "Query",
        "params": declared,
        "loaders": { "rows": { "sql": sql } },
        "view": { "viz": "table", "from": "rows" },
    });
    let page = serde_yaml::to_string(&spec)
        .map_err(|e| e.to_string())
        .and_then(|text| oxplow_app::pages::schema::parse_inline_page(&text));
    let page = match page {
        Ok(page) => page,
        Err(e) => {
            return report.errors.push(error(
                shown,
                None,
                "example.query_failed",
                format!("the query: {e}"),
            ))
        }
    };
    let Some(ctx) = view_context(host, ex, report).await else {
        return;
    };
    let at = PageAt {
        ex,
        host,
        shown,
        page: "query",
        target: oxplow_app::pages::runner::RunTarget::Spec(Arc::new(page)),
        ctx: &ctx,
    };
    let expect = json!({ "loaders": { "rows": ex.expect } });
    at.check(&params, &expect, &Binds::default(), "", report)
        .await;
}

/// Dry-run one of the extension's own `commands:` on the fixture's
/// `input` (and `answers`, standing in for its scope calls' — per
/// scope, in call order): what it composes, checked against the
/// throwaway's registry, and what it answers, against `expect` — `{
/// commands: [names] }` and/or `{ result }` (only the keys it names), or
/// `{ refused: <part of the reason> }`. Nothing runs.
pub(super) async fn command_example(
    host: &Host,
    ex: &Example<'_>,
    cmd: &oxplow_app::extension::extension_commands::ExtensionCommand,
    report: &mut TestReport,
) {
    use oxplow_app::extension::extension_commands::{call_names, dry_run, Composed};
    let shown = ex.shown;
    let Some(answers) = answers(ex, &format!("command `{}`", cmd.name), report) else {
        return;
    };
    let input = ex.when.get("input").cloned().unwrap_or_else(|| json!({}));
    let decided = dry_run(
        &host.svc.sql,
        cmd,
        &input,
        &answers,
        host.svc.commands.as_ref(),
    )
    .await;
    let problem = match (decided, ex.expect.get("refused").and_then(Value::as_str)) {
        (Err(e), _) => Some(format!("failed: {e}")),
        (Ok(Composed::Refused(why)), Some(want)) if why.contains(want) => None,
        (Ok(Composed::Refused(why)), _) => Some(format!("refused ({why})")),
        (Ok(Composed::Run { calls, .. }), Some(want)) => Some(format!(
            "composed [{}] but the example expects it to refuse ({want})",
            call_names(&calls).join(", ")
        )),
        (Ok(Composed::Run { calls, result, .. }), None) => {
            let got = json!({ "commands": call_names(&calls), "result": result });
            compare(&ex.expect, &got).map(|(path, want, got)| {
                let did = if path.starts_with("/result") {
                    "answered"
                } else {
                    "composed"
                };
                format!("{did} {got} at `{path}`, the example expects {want}")
            })
        }
    };
    if let Some(p) = problem {
        report.errors.push(error(shown, ex.line("expect"), "example.command", format!("command `{}` {p}", cmd.name)).fix("its script, or the example's `expect` (`{ commands: [names], result: { … } }` or `{ refused: <part of the reason> }`)"));
    }
}

/// Dry-run effect `id` on the fixture's event (`when: { effect, event: {
/// type, payload, subject? }, rows? }`): whether it reacts (`on`/`where`),
/// and what its script composes — checked against the throwaway's
/// registry — against `expect`: `{ commands: [names] }`, `{ skip: <part of
/// the reason> }` or `{ reacts: false }`. Nothing runs.
pub(super) async fn effect_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::effects::{dry_run, reacts_to};
    let (shown, example) = (ex.shown, ex.name);
    let Some(decl) = ext.effects.iter().find(|e| e.id == id) else {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.unknown",
                format!(
                    "example `{example}` names effect `{id}`, which `{}` doesn't declare",
                    ext.name
                ),
            )
            .fix("the effect's `id`"),
        );
        return;
    };
    let Some(event) = example_event(host, ex, report) else {
        return;
    };
    let event_type = event.event_type.as_str();
    let reacts = reacts_to(decl, event_type, &event.payload);
    let want_reacts = ex
        .expect
        .get("reacts")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if reacts != want_reacts {
        report.errors.push(error(shown, None, "example.effect_trigger", format!("effect `{id}` {} a `{event_type}` event like this one, the example expects it {}", if reacts {
                "reacts to"
            } else {
                "doesn't react to"
            }, if want_reacts { "to react" } else { "not to" })).fix("its `on`/`where`, or the example's `expect`"));
        return;
    }
    if !reacts {
        return;
    }
    let event = event.to_json();
    let Some(answers) = answers(ex, &format!("effect `{id}`"), report) else {
        return;
    };
    let decided = dry_run(
        &host.svc.sql,
        decl,
        &decl.script,
        event,
        &answers,
        Some(host.svc.commands.as_ref()),
    )
    .await;
    if let Some(p) = reaction_problem(&ex.expect, decided) {
        report.errors.push(error(shown, ex.line("expect"), "example.effect", format!("effect `{id}` {p}")).fix("its script, or the example's `expect` (`{ commands: [names] }`, `{ skip: <part of the reason> }` or `{ reacts: false }`)"));
    }
}

/// What's wrong with a script's reaction against an example's `expect` —
/// `{ commands: [names] }`, or `{ skip: <part of the reason> | $any }` —
/// or `None` when it matches.
pub(super) fn reaction_problem(
    expect: &Value,
    decided: Result<oxplow_app::effects::Reaction, String>,
) -> Option<String> {
    use oxplow_app::effects::Reaction;
    match (decided, expect.get("skip").and_then(Value::as_str)) {
        (Err(e), _) => Some(format!("failed: {e}")),
        (Ok(Reaction::Skip(why)), Some(want)) if want == "$any" || why.contains(want) => None,
        (Ok(Reaction::Skip(why)), _) => Some(format!("skipped ({why})")),
        (Ok(r @ Reaction::Run { .. }), Some(want)) => Some(format!(
            "composed [{}] but the example expects it to skip ({want})",
            r.command_names().join(", ")
        )),
        (Ok(r @ Reaction::Run { .. }), None) => {
            compare(expect, &json!({ "commands": r.command_names() })).map(|(path, want, got)| {
                format!("composed {got} at `{path}`, the example expects {want}")
            })
        }
    }
}

/// The example's `event:` as the event envelope the runtime would hand
/// over (`EventView::fixture`, through the throwaway's vocabulary); one
/// that couldn't happen is a finding at the fixture.
pub(super) fn example_event(
    host: &Host,
    ex: &Example<'_>,
    report: &mut TestReport,
) -> Option<EventView> {
    let given = ex.when.get("event").cloned().unwrap_or_else(|| json!({}));
    match EventView::fixture(&given, &host.svc.vocabulary.current()) {
        Ok(view) => Some(view),
        Err(e) => {
            report.errors.push(
                error(
                    ex.shown,
                    None,
                    "fixture.event",
                    format!("example `{}`: {}", ex.name, e.message()),
                )
                .fix("the fixture's `event` (extensions reference: the event envelope)"),
            );
            None
        }
    }
}

/// Dry-run the extension's effort policy script `id` on the fixture's
/// event (`when: { implementation, event: { type, payload, subject?,
/// anchors? }, answers? }`, the answers standing in for its scope calls'):
/// what it composes, checked against the throwaway's registry, against
/// `expect` as an effect's is. A policy emits nothing. Nothing runs.
pub(super) async fn policy_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::effects::{run_over, Reaction};
    let (shown, example) = (ex.shown, ex.name);
    let Some((decl, script)) = ext
        .implementations
        .iter()
        .filter(|d| d.id == id && d.capability == EFFORT_POLICY)
        .find_map(|d| d.script.as_deref().map(|s| (d, s)))
    else {
        report.errors.push(error(shown, None, "fixture.unknown", format!("example `{example}` names effort policy `{id}`, which `{}` doesn't declare as a script", ext.name)).fix(format!("an `implementations:` entry with `capability: effort_policy`, `id: {id}` and a `.star` entry")));
        return;
    };
    let Some(event) = example_event(host, ex, report) else {
        return;
    };
    let event = event.to_json();
    let Some(answers) = answers(ex, &format!("effort policy `{id}`"), report) else {
        return;
    };
    let run = oxplow_app::run_context::for_dry_run();
    let decided = run_over(
        &host.svc.sql,
        &run,
        &decl.needs,
        script,
        event,
        Some(&answers),
        &oxplow_app::services::Sources::default(),
    )
    .await
    .and_then(|reaction| match &reaction {
        Reaction::Run { events, .. } if !events.is_empty() => {
            Err("it appends events, and a policy emits nothing".to_string())
        }
        Reaction::Run { calls, .. } => oxplow_app::extension::extension_commands::check_calls(
            host.svc.commands.as_ref(),
            calls,
        )
        .map(|()| reaction),
        Reaction::Skip(_) => Ok(reaction),
    });
    if let Some(p) = reaction_problem(&ex.expect, decided) {
        report.errors.push(error(shown, ex.line("expect"), "example.effort_policy", format!("effort policy `{id}` {p}")).fix("its script, or the example's `expect` (`{ commands: [names] }` or `{ skip: <part of the reason> | $any }`)"));
    }
}

/// Check a payload against one of the extension's declared event types
/// (`when: { event_type, v?, payload }`, the newest version when `v` is
/// left out): `expect: { valid: true | false, upcast?: <payload at the
/// newest version> }`.
pub(super) fn event_type_example(
    ext: &Extension,
    ex: &Example<'_>,
    event_type: &str,
    report: &mut TestReport,
) {
    use oxplow_domain::events::schema::EventSchemaRegistry;
    let shown = ex.shown;
    let mut registry = EventSchemaRegistry::new();
    for d in &ext.event_types.types {
        // What doesn't register is `check`'s finding already.
        let _ = registry.register_declared(&ext.name, &ext.namespace, d.declared());
    }
    let Some(v) = ex
        .when
        .get("v")
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .or_else(|| registry.latest(event_type))
    else {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.unknown",
                format!(
                    "example `{}` names event type `{event_type}`, which `{}` doesn't declare",
                    ex.name, ext.name
                ),
            )
            .fix("the type, or its `event_types:`"),
        );
        return;
    };
    let payload = ex.when.get("payload").cloned().unwrap_or_else(|| json!({}));
    let valid = registry.validate(event_type, v, &payload);
    let want_valid = ex
        .expect
        .get("valid")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if valid.is_ok() != want_valid {
        report.errors.push(
            error(
                shown,
                None,
                "example.event_payload",
                format!(
                    "`{event_type}@{v}` {} this payload{}, the example expects {}",
                    if valid.is_ok() { "accepts" } else { "refuses" },
                    valid.err().map(|e| format!(" ({e})")).unwrap_or_default(),
                    if want_valid { "it valid" } else { "it refused" }
                ),
            )
            .fix("its schema, or the example"),
        );
        return;
    }
    if let Some(want) = ex.expect.get("upcast") {
        match registry.upcast_to_latest(event_type, v, payload) {
            Ok((_, got)) => {
                if let Some((path, want, got)) = compare(want, &got) {
                    report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("`{event_type}@{v}` upcasts to {got} at `{path}`, the example expects {want}")).fix("its upcast, or the example's `expect.upcast`").mismatch(path, want, got));
                }
            }
            Err(e) => report.errors.push(
                error(
                    shown,
                    None,
                    "example.upcast_failed",
                    format!("`{event_type}@{v}` doesn't upcast: {e}"),
                )
                .fix("its upcast"),
            ),
        }
    }
}

/// What a `[[…]]` names with the extension's resources beside
/// oxplow-foundation's (`when: { wikilink: "pr:12" }`):
/// `expect: { uri: "oxplow:acme/pr?id=12" | null }`.
pub(super) fn wikilink_example(
    ext: &Extension,
    ex: &Example<'_>,
    link: &str,
    report: &mut TestReport,
) {
    let mut resources = oxplow_domain::uri::resources::foundation().clone();
    for r in &ext.resources {
        let _ = resources.register(r.decl.clone());
    }
    let got = oxplow_domain::uri::canonical_wikilink(&resources, link).map(|u| u.to_string());
    if let Some((path, want, got)) = compare(&ex.expect, &json!({ "uri": got })) {
        report.errors.push(
            error(
                ex.shown,
                None,
                "fixture.compare",
                format!("`[[{link}]]` names {got} at `{path}`, the example expects {want}"),
            )
            .fix("the resource's `params`/`wikilink`, or the example's `expect`")
            .mismatch(path, want, got),
        );
    }
}
