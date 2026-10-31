use super::*;

/// What a `ui` step list has changed since the page last ran: data it
/// changed, and the view params the frame set (merged; `null` drops one),
/// which the app's tab would make the page's params.
#[derive(Default)]
pub(super) struct UiState {
    /// The embeds the component's page is inside (`component_at`).
    pub(super) embed: Vec<String>,
    pub(super) changed: bool,
    pub(super) view: serde_json::Map<String, Value>,
    /// How many of the harness's records `view` has read.
    pub(super) seen: usize,
}

impl UiState {
    pub(super) fn merge_view(&mut self, change: &Value) {
        for (k, v) in change.as_object().into_iter().flatten() {
            let before = self.view.get(k).cloned();
            if v.is_null() {
                self.view.remove(k);
            } else {
                self.view.insert(k.clone(), v.clone());
            }
            if self.view.get(k) != before.as_ref() {
                self.changed = true;
            }
        }
    }

    /// The step's params with the view params over them.
    pub(super) fn params(&self, step: &Value) -> Value {
        let mut merged = step.as_object().cloned().unwrap_or_default();
        for (k, v) in &self.view {
            merged.insert(k.clone(), v.clone());
        }
        Value::Object(merged)
    }
}

/// `ui` steps that can't run here: a skip, or with `--require-ui` an error.
/// `why` names no step: it is the same in every fixture, so the report
/// says it once (`render`).
fn unrun(report: &mut TestReport, shown: &str, why: String, fix: &str) -> bool {
    if report.require_ui {
        report.errors.push(
            error(shown, None, "fixture.ui_unrun", why)
                .fix(format!("{fix}, or run without --require-ui")),
        );
    } else {
        report
            .skipped
            .push(warning(shown, None, "fixture.ui_unrun", why).fix(fix));
    }
    false
}

impl PageAt<'_> {
    /// Run a step's `ui:` list against the page's custom component — its
    /// `node:`, else the first component node — in the app's own webview
    /// (`component_harness`), on this oxplow. False when the scenario stops
    /// here: it found anything, or the window draws nothing.
    pub(super) async fn ui(
        &self,
        ext: &Extension,
        params: &Value,
        (step, ui): (&Value, &Value),
        binds: &mut Binds,
        label: &str,
        report: &mut TestReport,
    ) -> bool {
        use crate::component_harness::{shell, Harness};
        let (shown, page) = (self.shown, self.page);
        let fail = |report: &mut TestReport, why: String| {
            report
                .errors
                .push(error(shown, None, "fixture.ui", format!("{label}: {why}")));
            false
        };
        let Some(shell) = shell() else {
            return unrun(
                report,
                shown,
                "its `ui` steps, and the steps after them, aren't run: no oxplow app here to \
                 draw them"
                    .into(),
                "install the Oxplow app, or set OXPLOW_SHELL to it",
            );
        };
        let Some(steps) = ui.as_array() else {
            return fail(report, "`ui` is a list of steps".into());
        };
        let params = match binds.substitute(params) {
            Ok(params) => params,
            Err(e) => return fail(report, e),
        };
        let run = match run_page(self.host, self.target.clone(), &params, self.ctx).await {
            Ok(run) => run,
            Err(e) => return fail(report, format!("page `{page}`: {e}")),
        };
        // The component the step drives, by its node — in the page, or in
        // a page it embeds (a replacement, a mount).
        let named = step.get("node").and_then(Value::as_str);
        let Some((embed, run, node, component)) = component_at(&run, named) else {
            return fail(
                report,
                format!(
                    "page `{page}` has no component node to drive{}",
                    named.map(|n| format!(" named `{n}`")).unwrap_or_default()
                ),
            );
        };
        let owner = run
            .page
            .id
            .split('/')
            .next()
            .unwrap_or(&ext.name)
            .to_string();
        let key = if component.contains('/') {
            component
        } else {
            format!("{owner}/{component}")
        };
        let version =
            match oxplow_app::viz_actions::load_component(&self.host.svc, &self.host.root, &key) {
                Ok(version) => version,
                Err(e) => return fail(report, format!("component `{key}` doesn't load: {e}")),
            };
        // A component that is the whole page is told it is one, and has a view.
        let whole = run.page.view.id == node;
        let start = json!({
            "version": version,
            "run": run,
            "node": node,
            "page": whole.then(|| json!({ "streamId": null, "threadId": null })),
            "view": whole.then(|| json!({ "viewParams": {}, "historyState": {} })),
        });
        let harness = match Harness::open(
            &shell,
            self.host.svc.clone(),
            self.host.root.clone(),
            key.clone(),
            version,
            start,
        )
        .await
        {
            Ok(harness) => harness,
            Err(e) => return fail(report, e),
        };
        // A window that draws no frames (the display locked or asleep)
        // can't lay a component out: its steps aren't run, never failed, nor
        // are the steps after them, which read what they did.
        match harness.drawing(std::time::Duration::from_secs(10)).await {
            Some(true) => {}
            Some(false) => {
                harness.close().await;
                return unrun(
                    report,
                    shown,
                    "its `ui` steps, and the steps after them, aren't run: the oxplow app's \
                     window draws nothing while the display is locked or asleep"
                        .into(),
                    "run it again with the display awake",
                );
            }
            None => {
                harness.close().await;
                return fail(report, "the harness's page never started".into());
            }
        }
        let ok = self
            .drive(
                &harness,
                ext,
                (&params, steps, &embed),
                binds,
                label,
                report,
            )
            .await;
        harness.close().await;
        ok
    }

    /// The `ui` steps on an open harness.
    pub(super) async fn drive(
        &self,
        harness: &crate::component_harness::Harness,
        ext: &Extension,
        (params, steps, embed): (&Value, &[Value], &[String]),
        binds: &mut Binds,
        label: &str,
        report: &mut TestReport,
    ) -> bool {
        let shown = self.shown;
        let wait = std::time::Duration::from_secs(10);
        // It starts: the frame says `ready`.
        let started = std::time::Instant::now();
        while !harness.records().iter().any(|(kind, _)| kind == "ready") {
            if started.elapsed() > std::time::Duration::from_secs(30) {
                let page = harness
                    .op(json!({ "op": "dom", "action": { "page": true } }), wait)
                    .await
                    .unwrap_or_default();
                report.errors.push(error(
                    shown,
                    None,
                    "fixture.ui",
                    format!(
                        "{label}: the component didn't start (it never connected) — the frame: {}",
                        page["value"]
                    ),
                ));
                return false;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let mut state = UiState {
            embed: embed.to_vec(),
            ..UiState::default()
        };
        for (index, item) in steps.iter().enumerate() {
            let at = format!("{label}: ui[{index}]");
            let item = match binds.substitute(item) {
                Ok(item) => item,
                Err(e) => {
                    report
                        .errors
                        .push(error(shown, None, "fixture.ui", format!("{at}: {e}")));
                    return false;
                }
            };
            let dom = ["click", "fill", "key", "drag"]
                .into_iter()
                .any(|k| item.get(k).is_some());
            let done = if dom {
                match harness
                    .op(json!({ "op": "dom", "action": item }), wait)
                    .await
                {
                    Ok(out) if out["ok"] == true => Ok(()),
                    Ok(out) => Err(out["error"].as_str().unwrap_or("it failed").to_string()),
                    Err(e) => Err(e),
                }
            } else if let Some(id) = item.get("press").and_then(Value::as_str) {
                harness
                    .op(json!({ "op": "press", "id": id }), wait)
                    .await
                    .map(|_| ())
            } else if let Some(view) = item.get("view_params") {
                state.merge_view(view);
                harness
                    .op(json!({ "op": "viewParams", "params": view }), wait)
                    .await
                    .map(|_| ())
            } else if let Some(given) = item.get("given") {
                state.changed = true;
                if run_steps(
                    self.host,
                    ext,
                    shown,
                    &format!("{at}.given"),
                    given,
                    binds,
                    report,
                )
                .await
                {
                    Ok(())
                } else {
                    return false;
                }
            } else if let Some(yes) = item.get("confirm").and_then(Value::as_bool) {
                harness.confirm_next(yes);
                Ok(())
            } else if let Some(form) = item.get("form") {
                harness.submit_next(form.get("submit").cloned());
                Ok(())
            } else if let Some(want) = item.get("expect") {
                self.expect_ui(harness, params, want, &mut state).await
            } else {
                Err(
                    "a `ui` step is `click`, `fill`, `key`, `drag`, `press`, `view_params`, \
                     `given`, `confirm`, `form` or `expect`"
                        .into(),
                )
            };
            if let Err(why) = done {
                let page = harness
                    .op(json!({ "op": "dom", "action": { "page": true } }), wait)
                    .await
                    .unwrap_or_default();
                report.errors.push(error(
                    shown,
                    None,
                    "fixture.ui",
                    format!(
                        "{at}: {why} — the frame: {} {}",
                        page["value"]["html"].as_str().unwrap_or("(unreadable)"),
                        match page["value"]["errors"].as_array() {
                            Some(errors) if !errors.is_empty() =>
                                format!("— its errors: {}", json!(errors)),
                            _ => String::new(),
                        }
                    ),
                ));
                return false;
            }
        }
        true
    }

    /// Wait (up to 5 s) for what the frame draws and asked the shell to be
    /// what `want` says; the page runs again and the frame hears its rows
    /// whenever a command ran or a step changed data.
    pub(super) async fn expect_ui(
        &self,
        harness: &crate::component_harness::Harness,
        params: &Value,
        want: &Value,
        state: &mut UiState,
    ) -> Result<(), String> {
        let wait = std::time::Duration::from_secs(10);
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            // The view params the frame set are the page's next run's.
            let records = harness.records();
            for (kind, change) in records.iter().skip(state.seen) {
                if kind == "viewParams" {
                    state.merge_view(change);
                }
            }
            state.seen = records.len();
            if harness.take_acted() || std::mem::take(&mut state.changed) {
                self.host.settle().await;
                let run = run_page(
                    self.host,
                    self.target.clone(),
                    &state.params(params),
                    self.ctx,
                )
                .await?;
                let run = embedded_at(&run, &state.embed)
                    .ok_or("the page no longer embeds the component's page")?;
                harness
                    .op(json!({ "op": "update", "run": run }), wait)
                    .await?;
            }
            let (wanted, got) = self.ui_seen(harness, want, wait).await?;
            match compare(&wanted, &got) {
                None => return Ok(()),
                Some(_) if std::time::Instant::now() < until => {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                Some((path, want, at)) => {
                    return Err(format!(
                        "{at} at `{path}`, the example expects {want} — fix: the component, or \
                         the step's `expect` (`{{ visible, count, text, title, actions, \
                         view_params, invoked, navigated, confirmed, forms, shown }}`)"
                    ))
                }
            }
        }
    }

    /// What `want` names, as the frame draws it and as it asked the shell:
    /// `want` as compared (`visible: sel` as `{ sel: true }`, a text's
    /// `{ contains }` kept when it holds) and what was seen.
    pub(super) async fn ui_seen(
        &self,
        harness: &crate::component_harness::Harness,
        want: &Value,
        wait: std::time::Duration,
    ) -> Result<(Value, Value), String> {
        let query = |selector: String| {
            let op = json!({ "op": "dom", "action": { "query": selector } });
            async move {
                let out = harness.op(op, wait).await?;
                Ok::<_, String>((
                    out["value"]["count"].as_u64().unwrap_or(0),
                    out["value"]["texts"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                ))
            }
        };
        // A text `{ contains }` holds when the text has it.
        let text = |want: &Value, got: Option<&str>| match (want.get("contains"), got) {
            (Some(Value::String(part)), Some(got)) if got.contains(part.as_str()) => want.clone(),
            (_, got) => json!(got),
        };
        let records = harness.records();
        let of = |kind: &str| -> Vec<Value> {
            records
                .iter()
                .filter(|(k, _)| k == kind)
                .map(|(_, v)| v.clone())
                .collect()
        };
        let (mut wanted, mut got) = (serde_json::Map::new(), serde_json::Map::new());
        for (key, w) in want.as_object().into_iter().flatten() {
            let seen = match key.as_str() {
                "visible" => {
                    let selectors: Vec<String> = match w {
                        Value::String(s) => vec![s.clone()],
                        Value::Array(list) => list
                            .iter()
                            .filter_map(|s| s.as_str().map(str::to_string))
                            .collect(),
                        _ => return Err("`visible` is a selector or a list of them".into()),
                    };
                    let mut shown = serde_json::Map::new();
                    for selector in &selectors {
                        shown.insert(
                            selector.clone(),
                            json!(query(selector.clone()).await?.0 > 0),
                        );
                    }
                    wanted.insert(
                        key.clone(),
                        json!(selectors
                            .iter()
                            .map(|s| (s.clone(), json!(true)))
                            .collect::<serde_json::Map<_, _>>()),
                    );
                    Value::Object(shown)
                }
                "count" | "text" => {
                    let mut seen = serde_json::Map::new();
                    for (selector, w) in w.as_object().into_iter().flatten() {
                        let (count, texts) = query(selector.clone()).await?;
                        seen.insert(
                            selector.clone(),
                            if key == "count" {
                                json!(count)
                            } else {
                                text(w, texts.first().and_then(Value::as_str))
                            },
                        );
                    }
                    Value::Object(seen)
                }
                "title" => text(w, of("title").last().and_then(Value::as_str)),
                "actions" => json!(of("actions")
                    .last()
                    .and_then(Value::as_array)
                    .map(|a| a.iter().map(|a| a["id"].clone()).collect::<Vec<_>>())
                    .unwrap_or_default()),
                "view_params" => {
                    let mut view = serde_json::Map::new();
                    for change in of("viewParams") {
                        for (k, v) in change.as_object().into_iter().flatten() {
                            if v.is_null() {
                                view.remove(k);
                            } else {
                                view.insert(k.clone(), v.clone());
                            }
                        }
                    }
                    Value::Object(view)
                }
                "invoked" => json!(of("invoked")),
                "navigated" => json!(of("navigate")),
                "confirmed" => json!(of("confirm")),
                "forms" => json!(of("openForm")
                    .into_iter()
                    .chain(of("askForm"))
                    .collect::<Vec<_>>()),
                "shown" => of("showView").last().cloned().unwrap_or(Value::Null),
                other => {
                    return Err(format!(
                        "`expect.{other}` isn't something a `ui` step reads"
                    ))
                }
            };
            if !wanted.contains_key(key) {
                wanted.insert(key.clone(), w.clone());
            }
            got.insert(key.clone(), seen);
        }
        Ok((Value::Object(wanted), Value::Object(got)))
    }
}
