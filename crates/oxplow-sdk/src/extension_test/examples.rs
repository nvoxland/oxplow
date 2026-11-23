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

/// `commands` as audited (name, run id): a sequence — how an effect's
/// commands run — is replaced by its steps, in order.
pub(super) async fn without_sequences(
    host: &Host,
    audited: Vec<(String, Option<i64>)>,
) -> Vec<String> {
    let audits = oxplow_db::SqliteCommandRunStore::new(host.svc.db.clone());
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
        "SELECT effect, CASE
           WHEN proposal_id IS NOT NULL THEN 'proposed'
           WHEN outcome IS NULL THEN 'started'
           WHEN outcome IN ('ok', 'skipped') THEN outcome
           ELSE 'failed' END, error
         FROM v_effect_run WHERE event_seq {at} ?1 AND latest = 1 \
         ORDER BY effect"
    ))
    .await
    .iter()
    .map(|r| json!({ "id": text(&r[0]), "state": text(&r[1]), "reason": text(&r[2]) }))
    .collect();
    let ran = rows(
        "SELECT json_extract(payload, '$.command'), json_extract(payload, '$.command_run_id') FROM v_event \
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
/// (`[names]`) and which of the named knowledge pages (`[uri]`) now exist.
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
                    "SELECT 1 FROM v_oxplow_knowledge_page WHERE uri = ?1",
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

/// Run `when: { undo: <command>, as?, confirmed? }` for real: the undo of
/// that command's newest run that was not undone yet (the inverse its
/// command run record kept, run through the bus as a run of its own), checked
/// against `expect`: `{ ok: true }` or `{ ok: false, refused }`.
pub(super) async fn undo_run(
    host: &Host,
    ex: &Example<'_>,
    command: &str,
    report: &mut TestReport,
) {
    use oxplow_domain::events::schema::CommandOutcome;
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
    let rows = host
        .svc
        .commands
        .command_run_store()
        .list_recent(1000)
        .await
        .unwrap_or_default();
    let Some(row) = rows.iter().find(|r| {
        r.command == command
            && r.outcome == CommandOutcome::Ok
            && r.undone_by.is_none()
            && r.inverse.is_some()
    }) else {
        return report.errors.push(
            error(
                shown,
                ex.line("when"),
                "example.failed",
                format!("`{command}` has no run to undo: none ran, all were undone, or its runs keep no inverse"),
            )
            .fix("run the command first (`given:` or an earlier step), and have its script return an `inverse`"),
        );
    };
    let confirmed = ex
        .when
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let outcome = host.svc.commands.undo(&actor, row.id, confirmed).await;
    host.settle().await;
    let got = match outcome {
        Ok(_) => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "refused": e.to_string() }),
    };
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("the undo of `{command}` gave {at} at `{path}`, the example expects {want}")).fix("the steps before it, or the example's `expect` (`{ ok: true }` or `{ ok: false, refused }`)").mismatch(path, want, at));
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
            "SELECT COALESCE(MAX(id), 0) FROM v_command_run",
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
        Ok(out) => {
            let mut got = match (out.result.get("result"), out.result.get("children")) {
                (Some(result), Some(Value::Array(children))) => {
                    composed = children
                        .iter()
                        .filter_map(|c| c.get("name").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect();
                    json!({ "ok": true, "result": result })
                }
                _ => json!({ "ok": true, "result": out.result }),
            };
            // The run's undo, as a dry run shows it (`{ commands: [{ name, input }] }`).
            if let Some(inverse) = &out.inverse {
                got["inverse"] =
                    json!({ "commands": [{ "name": inverse.name, "input": inverse.input }] });
            }
            got
        }
        Err(e) => json!({ "ok": false, "refused": e.to_string() }),
    };
    // The result already lists the composed steps. Read the other audited
    // writes here, including sequences started by effects.
    let mut audited: Vec<(String, Option<i64>)> = host
        .svc
        .sql
        .query_sql(
            "SELECT child.command, child.id
             FROM v_command_run child
             LEFT JOIN v_command_run parent
               ON parent.run_id = child.parent_run_id AND parent.id > ?1
             WHERE child.id > ?1 AND child.changed = 1 AND parent.id IS NULL
             ORDER BY child.id",
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
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("command `{command}` gave {at} at `{path}`, the example expects {want}")).fix("the command's input or the steps before it, or the example's `expect` (`{ ok, result?, inverse?, commands? }` or `{ ok: false, refused }`)").mismatch(path, want, at));
    }
}

/// Run advisory `id` for a thread (`when: { advisory, thread? }`, the
/// primary thread when none) as the app does on its `on:` — over that
/// thread — and check what the agent is told, line by line:
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

/// Run agent context `id` for a thread (`when: { context, thread? }`, the
/// primary thread when none) as the app does on a prompt, and check what
/// the agent would be told, uncut: `expect: { text: <string> | null }`
/// (`null`: nothing to say). A text over the per-context budget is a
/// warning: the agent gets it cut.
pub(super) async fn context_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    if !ext.contexts.iter().any(|c| c.id == id) {
        return report.errors.push(
            error(
                shown,
                None,
                "fixture.unknown",
                format!("`{id}` isn't one of the extension's agent contexts"),
            )
            .fix("the example's `when.context`"),
        );
    }
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
    let from = format!("{}/{id}", ext.namespace);
    use oxplow_app::agent_context::Reading;
    let reading = match oxplow_app::agent_context::extension_contexts(&host.svc, &thread).await {
        Ok(contexts) => contexts
            .into_iter()
            .find(|c| c.from == from)
            .map_or(Reading::Silent, |c| c.reading),
        Err(why) => Reading::Failed(why),
    };
    let text = match reading {
        Reading::Says(text) => Some(text),
        Reading::Silent => None,
        // A failure is the fixture's error, never "nothing to say".
        Reading::Failed(why) => {
            return report.errors.push(error(
                shown,
                None,
                "context.failed",
                format!("context `{id}` couldn't be read: {why}"),
            ));
        }
    };
    if let Some(t) = &text {
        let over = t.chars().count();
        if over > oxplow_app::agent_context::CONTEXT_BUDGET {
            report.warnings.push(
                warning(
                    shown,
                    None,
                    "context.over_budget",
                    format!(
                        "context `{id}` says {over} characters: the agent gets the first {}",
                        oxplow_app::agent_context::CONTEXT_BUDGET
                    ),
                )
                .fix("say less: the facts the agent needs now"),
            );
        }
    }
    let got = json!({ "text": text });
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.compare",
                format!("context `{id}` said {at} at `{path}`, the example expects {want}"),
            )
            .fix("its query, or the example's `given` and `expect` (`{ text: <string> | null }`)")
            .mismatch(path, want, at),
        );
    }
}

/// `when: { action_point: <id>, context? }`: what the named action point
/// offers for the context among the commands on this oxplow, as the shell
/// would draw it — `expect: { draws: none | button | menu, commands?: [ids] }`.
/// A bare name is this extension's own point.
pub(super) async fn action_point_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    point: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let id = if point.contains('/') {
        point.to_string()
    } else {
        format!("{}/{point}", ext.namespace)
    };
    let context = ex.when.get("context").cloned().unwrap_or_else(|| json!({}));
    host.current().await;
    let offer = match oxplow_app::commands::action_point_offers::action_point_offer(
        &host.svc, &id, &context,
    )
    .await
    {
        Ok(offer) => offer,
        Err(e) => {
            return report
                .errors
                .push(error(shown, None, "example.failed", e).fix("the example's `when`"))
        }
    };
    let got = json!({
        "draws": offer.draws,
        "commands": offer.commands.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
    });
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.compare",
                format!("action point `{id}` offered {at} at `{path}`, the example expects {want}"),
            )
            .fix("the commands' `ui.at`, or the example's `context` and `expect`")
            .mismatch(path, want, at),
        );
    }
}

/// `when: { item_commands: <uri> }`: the commands a person is offered
/// about the item `uri` (their `ui.about`, its store filter and
/// `about_when` applied) — `expect: { commands: [ids] }`.
pub(super) async fn item_commands_example(
    host: &Host,
    ex: &Example<'_>,
    uri: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    host.current().await;
    let ids = match oxplow_app::commands::ref_commands::ref_commands(&host.svc, uri).await {
        Ok(ids) => ids,
        Err(e) => {
            return report
                .errors
                .push(error(shown, None, "example.failed", e).fix("the example's `when`"))
        }
    };
    let got = json!({ "commands": ids });
    if let Some((path, want, at)) = compare(&ex.expect, &got) {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.compare",
                format!("`{uri}` was offered {at} at `{path}`, the example expects {want}"),
            )
            .fix("the commands' `ui.about` and `accepts`, or the example's `expect`")
            .mismatch(path, want, at),
        );
    }
}

/// A language's analysis of one file, as the fixture's oxplow reads it
/// with the languages its extensions declare: the file's definitions
/// (symbols), its functions' metrics and its imports.
pub(super) async fn language_example(
    host: &Host,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let languages = host.svc.languages.current();
    let Some(language) = languages.by_name(id).filter(|l| l.analyzable()) else {
        return report.errors.push(
            error(
                shown,
                None,
                "fixture.unknown",
                format!("`{id}` isn't a language the fixture's oxplow parses"),
            )
            .fix("the example's `when.language`, or the pack that declares it in `requires:`"),
        );
    };
    let text = |key: &str| ex.when.get(key).and_then(Value::as_str);
    let (Some(path), Some(body)) = (text("path"), text("body")) else {
        return report.errors.push(
            error(
                shown,
                None,
                "fixture.when",
                "a `language` example names the file's `path` and its `body`".to_string(),
            )
            .fix("`when: { language: rust, path: src/a.rs, body: \"fn a() {}\\n\" }`"),
        );
    };
    let symbols: Vec<Value> = oxplow_code_metrics::extract_with_language(body, language)
        .into_iter()
        .map(|d| {
            json!({
                "kind": d.kind.as_str(),
                "id": d.id,
                "name": d.name,
                "container": (!d.container.is_empty()).then(|| d.container.join("::")),
                "line": d.name_line,
            })
        })
        .collect();
    let functions: Vec<Value> = oxplow_code_metrics::analyze_with_language(path, body, language)
        .into_iter()
        .map(|f| {
            json!({
                "name": f.name,
                "id": f.id,
                "complexity": f.complexity,
                "length": f.length,
                "parameter_count": f.parameter_count,
                "visibility": format!("{:?}", f.visibility).to_lowercase(),
                "has_doc": f.has_doc,
                "start_line": f.start_line,
            })
        })
        .collect();
    let imports: Vec<Value> = oxplow_code_deps::extract_with_language(path, body, language)
        .into_iter()
        .map(|i| {
            json!({
                "module": i.module,
                "kind": format!("{:?}", i.kind).to_lowercase(),
                "line": i.start_line,
            })
        })
        .collect();
    let got = json!({ "symbols": symbols, "functions": functions, "imports": imports });
    if let Some((at, want, found)) = compare(&ex.expect, &got) {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.compare",
                format!("`{id}` read {found} at `{at}`, the example expects {want}"),
            )
            .fix("the language's analysis.yaml or symbols.scm, or the example's `expect`"),
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
/// commands: [names] }`, `{ result }` and/or `{ inverse: { commands: [{
/// name, input }] } }` (what its script returns to undo the run; only the
/// keys it names), or `{ refused: <part of the reason> }`. Nothing runs.
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
        host.svc.languages.current(),
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
        (
            Ok(Composed::Run {
                calls,
                result,
                inverse,
                ..
            }),
            None,
        ) => {
            let inverse = inverse.map(|calls| json!({ "commands": calls }));
            let got = json!({
                "commands": call_names(&calls),
                "result": result,
                "inverse": inverse,
            });
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
        host.svc.languages.current(),
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
