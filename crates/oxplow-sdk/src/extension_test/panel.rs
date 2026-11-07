//! A panel fixture (`when: { panel }`): the panel run whole, as the rail
//! shows it (`oxplow_app::pages::panels`). Its scope id is the view's, never a
//! param: `thread` and `stream` say which (the stream's first thread, the
//! project's first stream when it names none), `choices` the viewer's picks
//! for the body's toggles.

use std::collections::BTreeMap;

use super::*;

/// The keys a `panel` fixture's `when` takes.
const WHEN_KEYS: &[&str] = &["panel", "thread", "stream", "choices"];

/// The keys its `expect` takes: a part's run (checked as a page's is), or
/// what the panel says as a whole.
const EXPECT_KEYS: &[&str] = &[
    "body",
    "collapsed",
    "count_page",
    "badge",
    "count",
    "alert",
    "choices",
    "text",
];

pub(super) async fn panel_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    panel: &str,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let id = if panel.contains('/') {
        panel.to_string()
    } else {
        format!("{}/{panel}", ext.name)
    };
    if let Some(key) = ex
        .when
        .as_object()
        .and_then(|o| o.keys().find(|k| !WHEN_KEYS.contains(&k.as_str())))
    {
        let fix = if key == "params" {
            "a panel's params are its scope's and its toggles': `thread: thr1`, `stream: str1`, `choices: { mode: top }`".to_string()
        } else {
            format!("a panel's `when` is {}", keys(WHEN_KEYS))
        };
        return report.errors.push(
            error(
                shown,
                ex.line("when"),
                "fixture.unknown_key",
                format!("`{key}` isn't a key of a panel's `when`"),
            )
            .fix(fix),
        );
    }
    if let Some(key) = ex
        .expect
        .as_object()
        .and_then(|o| o.keys().find(|k| !EXPECT_KEYS.contains(&k.as_str())))
    {
        return report.errors.push(
            error(
                shown,
                ex.line("expect"),
                "fixture.unknown_key",
                format!("`{key}` isn't a key of a panel's `expect`"),
            )
            .fix(format!("a panel's `expect` is {}", keys(EXPECT_KEYS))),
        );
    }
    let (Some(stream), Some(thread)) = (
        scope_id(ex.when.get("stream"), oxplow_domain::StreamId::try_from_str),
        scope_id(ex.when.get("thread"), oxplow_domain::ThreadId::try_from_str),
    ) else {
        return report.errors.push(
            error(
                shown,
                ex.line("when"),
                "fixture.view",
                "a panel's `stream` or `thread` isn't one",
            )
            .fix("`stream: str1`, `thread: thr1`"),
        );
    };
    let picks: BTreeMap<String, String> = ex
        .when
        .get("choices")
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map_or_else(|| v.to_string(), str::to_string),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    host.current().await;
    let ctx = oxplow_app::pages::spec::view_context(&host.svc, stream, thread).await;
    let run = match oxplow_app::pages::panels::run_panel(&host.svc, &id, ctx.clone(), &picks).await
    {
        Ok(run) => run,
        Err(e) => {
            return report.errors.push(
                error(
                    shown,
                    ex.line("when"),
                    "example.panel_failed",
                    format!("panel `{id}`: {e}"),
                )
                .fix("the panel, or the example's `when`"),
            )
        }
    };
    let parts = [
        ("body", Some(&run.body)),
        ("collapsed", run.collapsed.as_ref()),
        ("count_page", run.count_run.as_ref()),
        ("badge", run.badge.as_ref()),
    ];
    for (part, part_run) in parts {
        let Some(part_run) = part_run else { continue };
        if let Some((loader, e)) = part_run
            .loaders
            .iter()
            .find_map(|(name, l)| l.error.as_ref().map(|e| (name, e)))
        {
            report.errors.push(
                error(
                    shown,
                    None,
                    "example.page_failed",
                    format!(
                        "panel `{id}`'s {part} `{}`: loader `{loader}` failed: {e}",
                        part_run.page.id
                    ),
                )
                .fix("the part, or the example's `given`"),
            );
            return;
        }
    }
    for (part, part_run) in parts {
        let Some(want) = ex.expect.get(part) else {
            continue;
        };
        let Some(part_run) = part_run else {
            report.errors.push(
                error(
                    shown,
                    ex.line("expect"),
                    "fixture.unknown",
                    format!("panel `{id}` has no {part}"),
                )
                .fix("the panel's declaration, or the example's `expect`"),
            );
            continue;
        };
        let page = part_run.page.id.clone();
        let at = PageAt {
            host,
            shown,
            ex,
            page: &page,
            target: oxplow_app::pages::runner::RunTarget::Page(page.clone()),
            ctx: &ctx,
        };
        at.check_run(
            part_run.clone(),
            want,
            &format!("panel `{id}`'s {part}: "),
            report,
        )
        .await;
    }
    let choices: Vec<Value> = run
        .choices
        .iter()
        .map(
            |c| json!({ "name": c.name, "label": c.label, "value": c.value, "options": c.options }),
        )
        .collect();
    let whole = [
        ("count", json!(run.count)),
        ("alert", json!(run.alert)),
        ("choices", Value::Array(choices)),
    ];
    for (key, got) in whole {
        let Some(want) = ex.expect.get(key) else {
            continue;
        };
        if let Some((path, want, got)) = compare(want, &got) {
            // The value itself, not one inside it: no trailing `/`.
            let path = if path == "/" { String::new() } else { path };
            report.errors.push(
                error(
                    shown,
                    ex.line("expect"),
                    "fixture.compare",
                    format!("panel `{id}`'s {key} is {got} at `{key}{path}`, the example expects {want}"),
                )
                .fix(format!("the panel, or `expect.{key}`"))
                .mismatch(format!("/{key}{path}"), want, got),
            );
        }
    }
    if let Some(want) = ex.expect.get("text") {
        match oxplow_app::pages::panels::panel_text(&host.svc, &run).await {
            Err(e) => report.errors.push(error(
                shown,
                None,
                "example.page_text",
                format!("panel `{id}` has no text: {e}"),
            )),
            Ok(text) => {
                let needles: Vec<&str> = want
                    .get("contains")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .collect();
                let exact_wrong = matches!(want, Value::String(exact) if *exact != text);
                let missing: Vec<&&str> = needles.iter().filter(|n| !text.contains(**n)).collect();
                if exact_wrong || !missing.is_empty() {
                    report.errors.push(
                        error(
                            shown,
                            ex.line("expect"),
                            "fixture.compare",
                            format!("panel `{id}`'s text is {text:?}, the example expects {want}"),
                        )
                        .fix(
                            "the panel, or `expect.text` (`{ contains: [..] }` or the exact text)",
                        ),
                    );
                }
            }
        }
    }
}

/// A `stream` or `thread` id: absent is `Some(None)` (the view's default),
/// one that isn't an id `None`.
fn scope_id<T>(v: Option<&Value>, parse: fn(&str) -> Option<T>) -> Option<Option<T>> {
    match v {
        None | Some(Value::Null) => Some(None),
        Some(Value::String(s)) => parse(s).map(Some),
        Some(_) => None,
    }
}

fn keys(list: &[&str]) -> String {
    list.iter()
        .map(|k| format!("`{k}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
