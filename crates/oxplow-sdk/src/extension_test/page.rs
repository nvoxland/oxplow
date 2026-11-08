use super::*;

/// Load component `id` as its frame would, then ask what the fixture asks
/// (`when: { component, items?: [title], invokes?: [{ command, input? }]
/// }`): `items` are work items filed first
/// (`oxplow:oxplow/work_item?id=tsk1&provider=oxplow`, …); each invoke's
/// outcome — what the command answers when the component calls it —
/// against `expect: { invokes?: [{ ok?, refused? }] }`. The data a
/// component draws is its page's loaders: a page example tests those. The
/// frame itself isn't drawn here.
pub(super) async fn component_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    id: &str,
    report: &mut TestReport,
) {
    use oxplow_app::viz_actions::{invoke_component_command, load_component, ComponentInvoke};
    let shown = ex.shown;
    // The component itself: it acts as itself, whichever page shows it.
    let key = format!("{}/{id}", ext.namespace);
    let version = match load_component(&host.svc, &host.root, &key) {
        Ok(v) => v,
        Err(e) => {
            report.errors.push(error(
                shown,
                None,
                "example.component_failed",
                format!("component `{id}` doesn't load: {e}"),
            ));
            return;
        }
    };
    for title in ex
        .when
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let made = host
            .svc
            .work_items_client()
            .create(
                &Actor::Human,
                oxplow_app::work_items::NewItem {
                    title: title.as_str().unwrap_or_default().into(),
                    ..Default::default()
                },
            )
            .await;
        if let Err(e) = made {
            report.errors.push(error(
                shown,
                None,
                "example.component_failed",
                format!("filing its item `{title}`: {e}"),
            ));
            return;
        }
    }
    let mut invokes = Vec::new();
    for i in ex
        .when
        .get("invokes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let out = invoke_component_command(
            &host.svc,
            &host.root,
            ComponentInvoke {
                component: key.clone(),
                command: i
                    .get("command")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                input: i.get("input").cloned().unwrap_or_else(|| json!({})),
                on_behalf_of: Actor::Human,
                confirmed: true,
                version: version.clone(),
            },
        )
        .await;
        invokes.push(match out {
            Ok(out) => json!({ "ok": true, "result": out.result }),
            // Denied or failed alike: it said no, and why.
            Err(oxplow_domain::CommandError::Denied { reason }) => {
                json!({ "ok": false, "refused": reason })
            }
            Err(e) => json!({ "ok": false, "refused": e.to_string() }),
        });
    }
    let got = json!({ "invokes": invokes });
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("component `{id}` answered {got} at `{path}`, the example expects {want}")).fix("the component's declarations, or the example's `expect` (`{ invokes: [{ ok, refused? }] }`)").mismatch(path, want, got));
    }
}

/// The kind of the page file `page` names (a slug of `ext`, or
/// `<namespace>/<slug>` of any enabled extension), when it is loaded.
fn kind_of_page(host: &Host, ext: &Extension, page: &str) -> Option<PageKind> {
    let id = if page.contains('/') {
        page.to_string()
    } else {
        format!("{}/{page}", ext.namespace)
    };
    ext.page_files
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.kind)
        .or_else(|| {
            host.svc
                .extension_catalog
                .find_page(&host.root, &id)
                .ok()
                .map(|p| p.kind)
        })
}

/// Whether the fixture's key fits what it names: `page:` runs a page,
/// `part:` a part, `query:` a query. The wrong key is a finding that says
/// which to write.
fn kind_matches(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    page: &str,
    wanted: PageKind,
    report: &mut TestReport,
) -> bool {
    let Some(kind) = kind_of_page(host, ext, page) else {
        return true;
    };
    if kind == wanted {
        return true;
    }
    let (key, right) = (wanted_key(wanted), wanted_key(kind));
    report.errors.push(
        error(
            ex.shown,
            ex.line("when"),
            "fixture.kind",
            format!("`{key}: {page}` names {}", kind.article()),
        )
        .fix(format!("write `when: {{ {right}: {page} }}`")),
    );
    false
}

fn wanted_key(kind: PageKind) -> &'static str {
    match kind {
        PageKind::Page => "page",
        PageKind::Part => "part",
        PageKind::Query => "query",
    }
}

/// Run query `query` (`<slug | ext/slug>[#loader]`) with the example's
/// params and compare its rows — its last loader's, or the named one's —
/// with `expect: { rows, columns? }`, as a `sql` example is compared.
pub(super) async fn query_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    query: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let (page, loader) = oxplow_app::pages::refs::split(query);
    // A bare `query: <slug>` names a query; `<page>#<loader>` names a
    // loader of any page file (a page's own option list, say).
    if loader.is_none() && !kind_matches(host, ext, ex, page, PageKind::Query, report) {
        return;
    }
    let id = if page.contains('/') {
        page.to_string()
    } else {
        format!("{}/{page}", ext.namespace)
    };
    let found = ext
        .page_files
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .or_else(|| host.svc.extension_catalog.find_page(&host.root, &id).ok());
    let loader = match loader {
        Some(l) => l.to_string(),
        None => match found.as_ref().and_then(|p| p.default_loader()) {
            Some(l) => l.name.clone(),
            None => {
                report.errors.push(
                    error(
                        shown,
                        ex.line("when"),
                        "fixture.unknown_page",
                        format!("`query: {query}` isn't a query of this extension"),
                    )
                    .fix("name one of `queries/`, or `<ext>/<slug>` of another extension"),
                );
                return;
            }
        },
    };
    let Some(ctx) = view_context(host, ex, report).await else {
        return;
    };
    let mut binds = Binds::default();
    if !ex.given.is_null()
        && !run_steps(host, ext, shown, "given", &ex.given, &mut binds, report).await
    {
        return;
    }
    host.current().await;
    // Only that loader's chain runs, as an option list or a form's open
    // runs it: nothing else on the page.
    let supplied = ex
        .when
        .get("params")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let want = match binds.substitute(&ex.expect) {
        Ok(want) => want,
        Err(e) => {
            report
                .errors
                .push(error(shown, ex.line("expect"), "fixture.step", e));
            return;
        }
    };
    let result = match oxplow_app::pages::options::source_rows(
        &host.svc,
        &format!("{id}#{loader}"),
        &supplied,
        &ctx,
        &oxplow_domain::Actor::Human,
    )
    .await
    {
        Ok(result) => result,
        Err(e) => {
            report.errors.push(
                error(
                    shown,
                    None,
                    "example.page_failed",
                    format!("`{query}`: {e}"),
                )
                .fix("the query, or the example's `params` and `given`"),
            );
            return;
        }
    };
    let got = result_got(&result, &want);
    if let Some((path, want, got)) = compare(&want, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("`{query}` returned {got} at `{path}`, the example expects {want}")).fix("the query, or the example's `expect` (`{ rows: n | $any | [..] | { contains: [..] }, columns? }`)").mismatch(path, want, got));
    }
}

/// Open page `slug`'s form as a person would (`when: { form, node?,
/// params? }`): `expect: { values?, fixed?, fields? }` against the values
/// the fields start from (the form's `defaults`, its loader's first row
/// laid over them), the fields shown read-only, and the fields shown
/// (`null`: all).
pub(super) async fn form_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    slug: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let params = ex
        .when
        .get("params")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let start = match ext.page_files.iter().find(|p| p.slug == slug) {
        Some(page) => page_form_start(host, page, ex.when.get("node"), params).await,
        None => Err(format!("`{slug}` isn't one of this extension's pages")),
    };
    let start = match start {
        Ok(start) => start,
        Err(e) => {
            report.errors.push(error(shown, None, "example.form_failed", format!("form `{slug}` didn't open: {e}")).fix("the form (a `viz: form` node with a `form:` naming a command; `node` names which), or the example's params"));
            return;
        }
    };
    let got = json!({ "values": start.values, "fixed": start.fixed, "fields": start.fields });
    if let Some((path, want, got)) = compare(&ex.expect, &got) {
        report.errors.push(error(shown, ex.line("expect"), "fixture.compare", format!("form `{slug}` opened with {got} at `{path}`, the example expects {want}")).fix("the form's `defaults`, `fixed` or `fields`, or the example's `expect` (`{ values?, fixed?, fields? }`)").mismatch(path, want, got));
    }
}

/// Open a page's form: its `node` (else its first form node) on a run of
/// the page with `params`.
pub(super) async fn page_form_start(
    host: &Host,
    page: &oxplow_app::pages::schema::Page,
    node: Option<&Value>,
    params: std::collections::BTreeMap<String, oxplow_db::SqlCell>,
) -> Result<oxplow_app::viz_actions::FormStart, String> {
    use oxplow_app::pages::schema::NodeKind;
    let node = match node.and_then(Value::as_str) {
        Some(id) => id.to_string(),
        None => page
            .nodes()
            .into_iter()
            .find(|n| {
                matches!(&n.kind, NodeKind::Viz { config: cfg, .. }
                    if cfg.viz == oxplow_app::pages::spec::Viz::Form)
            })
            .map(|n| n.id.clone())
            .ok_or_else(|| "the page has no form node".to_string())?,
    };
    let (run_id, _) = host
        .svc
        .page_runs
        .open(
            &host.svc,
            oxplow_app::pages::runner::RunRequest {
                target: oxplow_app::pages::runner::RunTarget::Page(page.id.clone()),
                params,
                ctx: oxplow_app::pages::spec::ViewContext::default(),
                viewer: oxplow_domain::Actor::Human,
            },
            "extension-test",
        )
        .await
        .map_err(|e| e.to_string())?;
    let start =
        oxplow_app::pages::actions::page_form(&host.svc, &host.svc.page_runs, &run_id, &node)
            .await
            .map_err(|e| e.to_string());
    host.svc.page_runs.close(&run_id);
    start
}

/// What a loader's result is, shaped as the example's `rows` (and
/// `columns`) expect: each row as its columns by name — or its cells,
/// where the example lists that row as a list; a count where it gives one.
fn result_got(result: &oxplow_db::SqlQueryResult, want: &Value) -> Value {
    let rows = &result.rows;
    let listed = want.get("rows").and_then(Value::as_array);
    let got_rows = match want.get("rows") {
        Some(Value::Number(_)) => json!(rows.len()),
        _ => Value::Array(
            rows.iter()
                .enumerate()
                .map(|(i, row)| match listed.and_then(|l| l.get(i)) {
                    Some(Value::Array(_)) => json!(row),
                    _ => Value::Object(result.json_row(row)),
                })
                .collect(),
        ),
    };
    let mut got = json!({ "rows": got_rows });
    if want.get("columns").is_some() {
        got["columns"] = json!(result.columns);
    }
    got
}

/// Loader `name` of a run: its own, or — `<embed>/<loader>` — one of the
/// page an embed node or a slot mount (`<extension>.<page>`) runs there.
pub(super) fn loader_at<'r>(
    run: &'r oxplow_app::pages::runner::PageRun,
    name: &str,
) -> Option<&'r oxplow_app::pages::runner::LoaderRun> {
    match name.split_once('/') {
        None => run.loaders.get(name),
        Some((embed, rest)) => embed_of(run, embed).and_then(|sub| loader_at(sub, rest)),
    }
}

/// The run of the page node `id` of `run` embeds (an embed, a slot's
/// mount, a replacement), in its view or its rail.
pub(super) fn embed_of<'r>(
    run: &'r oxplow_app::pages::runner::PageRun,
    id: &str,
) -> Option<&'r oxplow_app::pages::runner::PageRun> {
    fn node<'r>(
        at: &'r oxplow_app::pages::runner::RunNode,
        id: &str,
    ) -> Option<&'r oxplow_app::pages::runner::RunNode> {
        if at.id == id {
            return Some(at);
        }
        at.children.iter().find_map(|c| node(c, id))
    }
    node(&run.tree, id)
        .or_else(|| run.rail.as_ref().and_then(|rail| node(rail, id)))
        .and_then(|n| n.embed.as_deref())
}

/// The run `path` (embed node ids, outermost first) leads to from `run`.
pub(super) fn embedded_at(
    run: &oxplow_app::pages::runner::PageRun,
    path: &[String],
) -> Option<oxplow_app::pages::runner::PageRun> {
    match path.split_first() {
        None => Some(run.clone()),
        Some((head, rest)) => embedded_at(embed_of(run, head)?, rest),
    }
}

/// The component node a `ui` step drives in `run`: the one `named` names
/// (`<embed>/<node>` inside an embed, a mount or a replacement), else the
/// page's first component node — or, with none, the first in a page it
/// embeds. The embed path to the run holding it, that run, the node and
/// its component.
pub(super) fn component_at(
    run: &oxplow_app::pages::runner::PageRun,
    named: Option<&str>,
) -> Option<(
    Vec<String>,
    oxplow_app::pages::runner::PageRun,
    String,
    String,
)> {
    let inside = |embed: &str, rest: Option<&str>| {
        let (mut path, inner, node, component) = component_at(embed_of(run, embed)?, rest)?;
        path.insert(0, embed.to_string());
        Some((path, inner, node, component))
    };
    if let Some((embed, rest)) = named.and_then(|n| n.split_once('/')) {
        return inside(embed, Some(rest));
    }
    let own = run.page.nodes().into_iter().find_map(|n| {
        let component = match &n.kind {
            oxplow_app::pages::schema::NodeKind::Component { component, .. } => component.clone(),
            oxplow_app::pages::schema::NodeKind::Viz { config, .. }
                if config.viz == oxplow_app::pages::spec::Viz::Custom =>
            {
                config.custom.as_ref()?.component.clone()?
            }
            _ => return None,
        };
        (named.is_none_or(|id| id == n.id)).then(|| (n.id.clone(), component))
    });
    if let Some((node, component)) = own {
        return Some((Vec::new(), run.clone(), node, component));
    }
    if named.is_some() {
        return None;
    }
    fn embeds(at: &oxplow_app::pages::runner::RunNode, out: &mut Vec<String>) {
        if at.embed.is_some() {
            out.push(at.id.clone());
        }
        at.children.iter().for_each(|c| embeds(c, out));
    }
    let mut ids = Vec::new();
    embeds(&run.tree, &mut ids);
    ids.iter().find_map(|id| inside(id, None))
}

/// A page run's node ids (`<embed>/<id>` inside an embed) with their kinds.
/// What an `expect.nodes` entry may say about a node.
pub(super) const NODE_KEYS: [&str; 4] = ["id", "kind", "viz", "actions"];

pub(super) fn run_nodes(run: &oxplow_app::pages::runner::PageRun) -> Vec<(String, Value)> {
    fn walk(
        run: &oxplow_app::pages::runner::PageRun,
        node: &oxplow_app::pages::runner::RunNode,
        prefix: &str,
        out: &mut Vec<(String, Value)>,
    ) {
        let declared = run
            .page
            .nodes()
            .into_iter()
            .find(|n| n.id == node.id)
            .and_then(|n| serde_json::to_value(n).ok())
            .unwrap_or(Value::Null);
        out.push((format!("{prefix}{}", node.id), declared));
        if let Some(sub) = &node.embed {
            walk(sub, &sub.tree, &format!("{prefix}{}/", node.id), out);
        }
        for child in &node.children {
            walk(run, child, prefix, out);
        }
    }
    let mut out = Vec::new();
    walk(run, &run.tree, "", &mut out);
    if let Some(rail) = &run.rail {
        walk(run, rail, "", &mut out);
    }
    out
}

/// Run `target` (a page, `<extension>/<slug>`, or an inline one) with
/// `params` as a person would open it from `ctx`; the failure to run, or a
/// loader that failed, as a message.
pub(super) async fn run_page(
    host: &Host,
    target: oxplow_app::pages::runner::RunTarget,
    params: &Value,
    ctx: &oxplow_app::pages::spec::ViewContext,
) -> Result<oxplow_app::pages::runner::PageRun, String> {
    use oxplow_app::pages::runner::{PageRunner, RunRequest};
    host.current().await;
    let params = params
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        })
        .unwrap_or_default();
    let run = PageRunner::of(&host.svc)
        .run(RunRequest {
            target,
            params,
            ctx: ctx.clone(),
            viewer: Actor::Human,
        })
        .await
        .map_err(|e| e.to_string())?;
    if let Some((name, error)) = run
        .loaders
        .iter()
        .find_map(|(name, l)| l.error.as_ref().map(|e| (name, e)))
    {
        return Err(format!("loader `{name}` failed: {error}"));
    }
    Ok(run)
}

/// The example's `view: { stream?, thread? }` (`str1`, `thr1` or their
/// numbers) as the context its page runs in — the project's first stream
/// when it names none, so a page that binds `stream_id` needs no param; `None`, with the error
/// reported, when it isn't that shape.
pub(super) async fn view_context(
    host: &Host,
    ex: &Example<'_>,
    report: &mut TestReport,
) -> Option<oxplow_app::pages::spec::ViewContext> {
    /// A view's id: its number, or its ref read by `parse` (`thr1`).
    fn id(v: Option<&Value>, parse: fn(&str) -> Option<i64>) -> Result<Option<i64>, ()> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => n.as_i64().map(Some).ok_or(()),
            Some(Value::String(s)) => parse(s).map(Some).ok_or(()),
            Some(_) => Err(()),
        }
    }
    let view = &ex.view;
    if !(view.is_null() || view.is_object()) {
        report.errors.push(error(
            ex.shown,
            None,
            "fixture.view",
            "`view` isn't `{ stream?, thread? }`",
        ));
        return None;
    }
    match (
        id(view.get("stream"), |s| {
            oxplow_domain::StreamId::try_from_str(s).map(|i| i.value())
        }),
        id(view.get("thread"), |s| {
            oxplow_domain::ThreadId::try_from_str(s).map(|i| i.value())
        }),
    ) {
        (Ok(stream_id), Ok(thread_id)) => Some(oxplow_app::pages::spec::ViewContext {
            // A page is viewed from the project's first stream unless the
            // fixture says another.
            stream_id: match stream_id {
                Some(id) => Some(id),
                None => first_stream(host).await.ok().map(|s| s.value()),
            },
            thread_id,
            active: None,
        }),
        _ => {
            report.errors.push(
                error(
                    ex.shown,
                    None,
                    "fixture.view",
                    "`view` names a stream or thread that isn't one",
                )
                .fix("`{ stream: str1, thread: thr1 }`"),
            );
            None
        }
    }
}

/// Run page `page` after the fixture's `given` steps, with its `params`,
/// and check it against `expect`; then each of its `steps:` in order — a
/// `given` changes the state, an `expect` (with `params` over the
/// example's, and `bind` to keep a value of the run) runs the page again
/// and checks it. `expect: { loaders?: { <name>: { rows: n | $any | [ {
/// col: val } | [ val, … ] ], columns? } }, nodes?: [{ id, kind?, actions?
/// }], text?, alert?, badge?, label?, inactive?, actions?, see_also? }`. A row map matches when
/// every column it names equals; a row list matches the loader's rows in
/// order. Anything may use what was bound (`$name.path`).
pub(super) async fn page_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    page: &str,
    wanted: PageKind,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    if !kind_matches(host, ext, ex, page, wanted, report) {
        return;
    }
    let mut binds = Binds::default();
    let Some(ctx) = view_context(host, ex, report).await else {
        return;
    };
    if !ex.given.is_null()
        && !run_steps(host, ext, shown, "given", &ex.given, &mut binds, report).await
    {
        return;
    }
    if !seed_rows(host, ext, ex, report).await {
        return;
    }
    let id = if page.contains('/') {
        page.to_string()
    } else {
        format!("{}/{page}", ext.namespace)
    };
    let params = ex.when.get("params").cloned().unwrap_or(Value::Null);
    let at = PageAt {
        ex,
        host,
        shown,
        page,
        target: oxplow_app::pages::runner::RunTarget::Page(id.clone()),
        ctx: &ctx,
    };
    let steps = match &ex.steps {
        None => Vec::new(),
        Some(Value::Array(steps)) => steps.clone(),
        Some(_) => {
            report.errors.push(
                error(
                    shown,
                    ex.line("steps"),
                    "fixture.step",
                    "`steps` isn't a list",
                )
                .fix("`steps: [{ given: [..] } | { expect, params?, bind? }]`"),
            );
            return;
        }
    };
    if (ex.steps.is_none() || !ex.expect.is_null())
        && at
            .check(&params, &ex.expect, &binds, "", report)
            .await
            .is_none()
    {
        return;
    }
    for (index, step) in steps.iter().enumerate() {
        let label = format!("steps[{index}]");
        if let Some(given) = step.get("given") {
            if !run_steps(
                host,
                ext,
                shown,
                &format!("{label}.given"),
                given,
                &mut binds,
                report,
            )
            .await
            {
                return;
            }
        }
        let mut run_params = params.as_object().cloned().unwrap_or_default();
        if let Some(over) = step.get("params").and_then(Value::as_object) {
            run_params.extend(over.clone());
        }
        if let Some(ui) = step.get("ui") {
            let params = Value::Object(run_params);
            if !at
                .ui(ext, &params, (step, ui), &mut binds, &label, report)
                .await
            {
                return;
            }
            continue;
        }
        if let Some(when) = step.get("when") {
            let expect = step.get("expect").cloned().unwrap_or(Value::Null);
            if !step_run(host, ext, ex, (when, &expect), &binds, &label, report).await {
                return;
            }
            continue;
        }
        if step.get("press").is_some() || step.get("submit").is_some() {
            let Some(outcome) = at
                .act(&Value::Object(run_params), step, &binds, &label, report)
                .await
            else {
                return;
            };
            if !bind_from(step, &outcome, &mut binds, shown, &label, report) {
                return;
            }
            continue;
        }
        let Some(expect) = step.get("expect") else {
            if step.get("given").is_none() {
                report.errors.push(
                    error(
                        shown,
                        None,
                        "fixture.step",
                        format!("`{label}` has no `given` or `expect`"),
                    )
                    .fix("`{ given: [..] }` or `{ expect, params?, bind? }`"),
                );
                return;
            }
            continue;
        };
        let Some(run) = at
            .check(
                &Value::Object(run_params),
                expect,
                &binds,
                &format!("{label}: "),
                report,
            )
            .await
        else {
            return;
        };
        if !bind_from(step, &run_json(&run), &mut binds, shown, &label, report) {
            return;
        }
    }
}

/// Where a page example runs: its host, fixture, page and view.
pub(super) struct PageAt<'a> {
    pub(super) host: &'a Host,
    pub(super) shown: &'a str,
    /// The fixture, to find a key's line in.
    pub(super) ex: &'a Example<'a>,
    /// The page as the fixture names it.
    pub(super) page: &'a str,
    /// What runs: the page (`<extension>/<slug>`), or an inline one.
    pub(super) target: oxplow_app::pages::runner::RunTarget,
    pub(super) ctx: &'a oxplow_app::pages::spec::ViewContext,
}

impl PageAt<'_> {
    /// Act on a run of the page with `params`, as a person would: the
    /// step's `press` (`{ action, node?, row?, input?, confirmed? }`, one of
    /// its actions) or `submit` (`{ node, input, confirmed? }`, a form
    /// node), read through `binds`. What it gave is `{ ok: true, result }`,
    /// or `{ ok: false, refused }` when it said no — checked against the
    /// step's `expect` when it has one; without, one that says no is a
    /// finding. `None` when the step can't go on.
    pub(super) async fn act(
        &self,
        params: &Value,
        step: &Value,
        binds: &Binds,
        label: &str,
        report: &mut TestReport,
    ) -> Option<Value> {
        let (shown, page) = (self.shown, self.page);
        let submit = step.get("submit").is_some();
        let kind = if submit { "submit" } else { "press" };
        let (params, what, expect) = match (
            binds.substitute(params),
            binds.substitute(&step[kind]),
            step.get("expect").map(|e| binds.substitute(e)).transpose(),
        ) {
            (Ok(p), Ok(what), Ok(e)) => (p, what, e),
            (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
                report.errors.push(error(
                    shown,
                    self.ex.line("steps"),
                    "fixture.step",
                    format!("{label}: {e}"),
                ));
                return None;
            }
        };
        let Some(action) = what
            .get(if submit { "node" } else { "action" })
            .and_then(Value::as_str)
        else {
            report.errors.push(error(shown, self.ex.line("steps"), "fixture.step", format!("{label}: `press` is `{{ action, node?, row?, input?, confirmed? }}`, `submit` is `{{ node, input, confirmed? }}`")));
            return None;
        };
        let cells = |v: &Value| -> std::collections::BTreeMap<String, oxplow_db::SqlCell> {
            v.as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                .collect()
        };
        let runs = oxplow_app::pages::open_runs::PageRuns::default();
        let opened = runs
            .open(
                &self.host.svc,
                oxplow_app::pages::runner::RunRequest {
                    target: self.target.clone(),
                    params: cells(&params),
                    ctx: self.ctx.clone(),
                    viewer: Actor::Human,
                },
                "fixture",
            )
            .await;
        let run_id = match opened {
            Ok((run_id, _)) => run_id,
            Err(e) => {
                report.errors.push(error(
                    shown,
                    None,
                    "example.page_failed",
                    format!("{label}: page `{page}`: {e}"),
                ));
                return None;
            }
        };
        let confirmed = what
            .get("confirmed")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let input = what.get("input").cloned();
        let outcome = if submit {
            oxplow_app::pages::actions::submit_page_form(
                &self.host.svc,
                &runs,
                &run_id,
                action,
                input.unwrap_or_else(|| json!({})),
                Actor::Human,
                confirmed,
            )
            .await
        } else {
            oxplow_app::pages::actions::run_page_action(
                &self.host.svc,
                &runs,
                oxplow_app::pages::actions::PageActionCall {
                    run_id,
                    node: what.get("node").and_then(Value::as_str).map(str::to_string),
                    action: action.to_string(),
                    row: what.get("row").map(cells),
                    input,
                    drop: None,
                    on_behalf_of: Actor::Human,
                    confirmed,
                },
            )
            .await
        };
        self.host.settle().await;
        let got = match outcome {
            Ok(out) => json!({ "ok": true, "result": out.result }),
            Err(e) => json!({ "ok": false, "refused": e.to_string() }),
        };
        match expect {
            Some(expect) => {
                if let Some((path, want, at)) = compare(&expect, &got) {
                    report.errors.push(error(shown, self.ex.line("expect"), "fixture.compare", format!("{label}: {kind} `{action}` on page `{page}` gave {at} at `{path}`, the example expects {want}")).fix("the action, or the step's `expect` (`{ ok, result? }` or `{ ok: false, refused }`)").mismatch(path, want, at));
                    return None;
                }
            }
            None if got["ok"] == false => {
                report.errors.push(
                    error(
                        shown,
                        None,
                        "example.action_refused",
                        format!(
                            "{label}: {kind} `{action}` on page `{page}` said no: {}",
                            got["refused"]
                        ),
                    )
                    .fix("the action, or `expect: { ok: false, refused }` on the step"),
                );
                return None;
            }
            None => {}
        }
        Some(got)
    }

    /// Run the page with `params` and check it against `expect`, both
    /// read through `binds`; findings start with `label`. The run, unless
    /// it couldn't run.
    pub(super) async fn check(
        &self,
        params: &Value,
        expect: &Value,
        binds: &Binds,
        label: &str,
        report: &mut TestReport,
    ) -> Option<oxplow_app::pages::runner::PageRun> {
        let (shown, page) = (self.shown, self.page);
        let (params, expect) = match (binds.substitute(params), binds.substitute(expect)) {
            (Ok(params), Ok(expect)) => (params, expect),
            (Err(e), _) | (_, Err(e)) => {
                report.errors.push(error(
                    shown,
                    self.ex.line("steps"),
                    "fixture.step",
                    format!("{label}{e}"),
                ));
                return None;
            }
        };
        let run = match run_page(self.host, self.target.clone(), &params, self.ctx).await {
            Ok(run) => run,
            Err(e) => {
                report.errors.push(
                    error(
                        shown,
                        None,
                        "example.page_failed",
                        format!("{label}page `{page}`: {e}"),
                    )
                    .fix("the page, or the example's `params` and `given`"),
                );
                return None;
            }
        };
        self.check_run(run, &expect, label, report).await
    }

    /// Check `run`, a run of this page already made, against `expect`
    /// (already read through the binds): its loaders, its own parts, its
    /// text and its nodes, each mismatch a finding. The run comes back, for
    /// the binds.
    pub(super) async fn check_run(
        &self,
        run: oxplow_app::pages::runner::PageRun,
        expect: &Value,
        label: &str,
        report: &mut TestReport,
    ) -> Option<oxplow_app::pages::runner::PageRun> {
        let (shown, page) = (self.shown, self.page);
        if let Some(loaders) = expect.get("loaders").and_then(Value::as_object) {
            for (name, want) in loaders {
                let Some(loader) = loader_at(&run, name) else {
                    report.errors.push(
                        error(
                            shown,
                            None,
                            "fixture.unknown",
                            format!("{label}page `{page}` has no loader `{name}`"),
                        )
                        .fix("the example's `expect.loaders`"),
                    );
                    continue;
                };
                let got = result_got(&loader.result, want);
                if let Some((path, want, got)) = compare(want, &got) {
                    report.errors.push(error(shown, self.ex.line("expect"), "fixture.compare", format!("{label}page `{page}` loader `{name}` returned {got} at `{path}`, the example expects {want}")).fix(format!("the page, or the example's `expect.loaders.{name}` (`{{ rows: n | $any | [..] | {{ contains: [..] }}, columns? }}`)")).mismatch(path, want, got));
                }
            }
        }
        // The run's own parts, each compared on what the example names.
        let parts = [
            ("alert", json!(run.alert)),
            ("badge", json!(run.badge)),
            ("label", json!(oxplow_app::pages::label::run_label(&run))),
            ("inactive", json!(run.inactive)),
            ("see_also", json!(run.see_also)),
            ("actions", Value::Null),
        ];
        for (key, got) in parts {
            let Some(want) = expect.get(key) else {
                continue;
            };
            let got = match key {
                "actions" => actions_like(want, &run.page.actions),
                _ => got,
            };
            if let Some((path, want, got)) = compare(want, &got) {
                report.errors.push(error(shown, self.ex.line("expect"), "fixture.compare", format!("{label}page `{page}`'s {key} is {got} at `{key}{path}`, the example expects {want}")).fix(format!("the page, or `expect.{key}`")).mismatch(format!("/{key}{path}"), want, got));
            }
        }
        if let Some(want) = expect.get("text") {
            match oxplow_app::pages::text::page_text(&self.host.svc, &self.host.root, &run).await {
                Err(e) => report.errors.push(
                    error(
                        shown,
                        None,
                        "example.page_text",
                        format!("{label}page `{page}` has no text: {e}"),
                    )
                    .fix("the page"),
                ),
                Ok(text) => {
                    let missing: Vec<&str> = want
                        .get("contains")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|needle| !text.contains(*needle))
                        .collect();
                    if matches!(want, Value::String(exact) if *exact != text) {
                        report.errors.push(error(shown, self.ex.line("expect"), "fixture.compare", format!("{label}page `{page}`'s text is {text:?}, the example expects {want}")).fix("the page, or `expect.text` (`{ contains: [..] }` or the exact text)").mismatch("/text", want.clone(), Value::String(text.clone())));
                    }
                    for needle in missing {
                        report.errors.push(
                            error(
                                shown,
                                None,
                                "example.page_text",
                                format!("{label}page `{page}`'s text doesn't contain {needle:?}"),
                            )
                            .fix(format!(
                                "the page, or `expect.text.contains`; the text is:\n{text}"
                            )),
                        );
                    }
                }
            }
        }
        if let Some(nodes) = expect.get("nodes").and_then(Value::as_array) {
            let present = run_nodes(&run);
            for want in nodes {
                let Some(id) = want.get("id").and_then(Value::as_str) else {
                    report.errors.push(error(
                        shown,
                        None,
                        "fixture.nodes",
                        format!("{label}`expect.nodes` entries are `{{ id, kind?, actions? }}`"),
                    ));
                    continue;
                };
                if let Some(key) = want
                    .as_object()
                    .and_then(|o| o.keys().find(|k| !NODE_KEYS.contains(&k.as_str())))
                {
                    report.errors.push(
                        error(
                            shown,
                            None,
                            "fixture.unknown_key",
                            format!("{label}`{key}` isn't a key of `expect.nodes`"),
                        )
                        .fix(format!(
                            "a node is {}",
                            NODE_KEYS.map(|k| format!("`{k}`")).join(", ")
                        )),
                    );
                    continue;
                }
                let Some((_, declared)) = present.iter().find(|(have, _)| have == id) else {
                    report.errors.push(
                        error(
                            shown,
                            None,
                            "example.page_node",
                            format!(
                                "{label}page `{page}` has no node `{id}` in this run (it has {})",
                                present
                                    .iter()
                                    .map(|(n, _)| format!("`{n}`"))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            ),
                        )
                        .fix("the example, or the page's view"),
                    );
                    continue;
                };
                let has = |key: &str| {
                    declared
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                };
                if let Some(k) = want.get("kind").and_then(Value::as_str) {
                    if k != has("kind") {
                        report.errors.push(error(shown, None, "example.page_node", format!("{label}page `{page}` node `{id}` is a `{}`, the example expects `{k}`", has("kind"))));
                    }
                }
                // A viz node's viz is in its config.
                let viz = declared
                    .pointer("/config/viz")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(v) = want.get("viz").and_then(Value::as_str) {
                    if v != viz {
                        report.errors.push(error(shown, None, "example.page_node", format!("{label}page `{page}` node `{id}` is a `{viz}` viz, the example expects `{v}`")).fix("the page, or the node's `viz` in `expect.nodes`"));
                    }
                }
                if let Some(want) = want.get("actions") {
                    let got = actions_like(want, &node_actions(&run, id));
                    if let Some((path, want, got)) = compare(want, &got) {
                        report.errors.push(error(shown, self.ex.line("expect"), "fixture.compare", format!("{label}page `{page}` node `{id}`'s actions are {got} at `{path}`, the example expects {want}")).fix("the page, or the node's `actions` in `expect.nodes`").mismatch(path, want, got));
                    }
                }
            }
        }
        Some(run)
    }
}

/// `actions` as an example names them: an id where it names an id, else
/// the action's fields (spelled as the page YAML spells them) that the
/// example names.
pub(super) fn actions_like(want: &Value, actions: &[oxplow_app::pages::spec::PageAction]) -> Value {
    let named = want.as_array();
    Value::Array(
        actions
            .iter()
            .enumerate()
            .map(|(i, action)| match named.and_then(|w| w.get(i)) {
                Some(Value::Object(_)) => {
                    let fields = match json!(action) {
                        Value::Object(fields) => fields
                            .into_iter()
                            .map(|(k, v)| (snake_case(&k), v))
                            .collect(),
                        _ => serde_json::Map::new(),
                    };
                    Value::Object(fields)
                }
                _ => json!(action.id),
            })
            .collect(),
    )
}

/// `oneRow` → `one_row`.
pub(super) fn snake_case(name: &str) -> String {
    name.chars()
        .flat_map(|c| {
            let lower = c.to_ascii_lowercase();
            (c.is_ascii_uppercase().then_some('_').into_iter()).chain([lower])
        })
        .collect()
}

/// The actions node `id` of a run's page offers (a viz's), in the order
/// it declares them.
pub(super) fn node_actions(
    run: &oxplow_app::pages::runner::PageRun,
    id: &str,
) -> Vec<oxplow_app::pages::spec::PageAction> {
    let local = id.rsplit('/').next().unwrap_or(id);
    run.page
        .nodes()
        .into_iter()
        .find(|n| n.id == local)
        .and_then(|n| match &n.kind {
            oxplow_app::pages::schema::NodeKind::Viz { config, .. } => Some(config.actions.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Every page's `answers:` entry runs with its params: the page opens and
/// none of its loaders fails. A test case per page.
pub(super) async fn page_answers(host: &Host, ext: &Extension, report: &mut TestReport) {
    let mut any = false;
    // An answer may name the primary stream's thread.
    if let Err(e) = host.svc.streams.ensure_primary().await {
        report.errors.push(Diagnostic::error(
            "example.thread_failed",
            format!("the primary thread: {e}"),
        ));
        return;
    }
    let ctx = oxplow_app::pages::spec::ViewContext::default();
    for page in &ext.page_files {
        if page.answers.is_empty() {
            continue;
        }
        any = true;
        let mark = report.mark();
        for answer in &page.answers {
            let params = json!(answer.params);
            if let Err(e) = run_page(
                host,
                oxplow_app::pages::runner::RunTarget::Page(page.id.clone()),
                &params,
                &ctx,
            )
            .await
            {
                report.errors.push(
                    error(
                        &page.file(),
                        None,
                        "example.answer_failed",
                        format!("the answer to \"{}\" doesn't run: {e}", answer.question),
                    )
                    .contribution(page.id.clone())
                    .fix("the answer's `params`, or the page"),
                );
            }
        }
        report.finish_case(format!("answers {}", page.slug), mark);
    }
    if any {
        report.ran.push("answers".into());
    }
}

/// A fixture's `answers`: per scope, its calls' answers in call order,
/// standing in for the scopes' own (none: each is served for real);
/// `None`, with the error reported, when they aren't that shape.
pub(super) fn answers(
    ex: &Example<'_>,
    what: &str,
    report: &mut TestReport,
) -> Option<std::collections::BTreeMap<String, Vec<Value>>> {
    match ex.when.get("answers").cloned().map(serde_json::from_value) {
        None => Some(Default::default()),
        Some(Ok(answers)) => Some(answers),
        Some(Err(e)) => {
            report.errors.push(error(
                ex.shown,
                None,
                "fixture.answers",
                format!("{what}: `answers` must map each scope to its answers in call order: {e}"),
            ));
            None
        }
    }
}
