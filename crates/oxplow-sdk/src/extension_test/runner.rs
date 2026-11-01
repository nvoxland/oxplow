use super::*;

/// Run example `ex` on `host` as one test case: its `given` steps, what
/// its `when` names checked against `expect`, then its `steps`, each
/// `{ given }` or a `{ when?, expect }` run the same way (`when` over the
/// fixture's). A page runs its own scenario (`page_example`).
pub(super) async fn run_example(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    // A provider's command runs in its session (`test_provider`).
    let command = ex.when.get("command").and_then(Value::as_str);
    if command.is_some_and(|name| !ext.commands.iter().any(|c| c.name == name))
        && !ext.providers.is_empty()
    {
        return;
    }
    let mark = report.mark();
    report.ran.push(format!("example {}", ex.name));
    match model_replies(ex, report).map(|replies| host.answer_with(replies)) {
        Some(Ok(())) => {
            if !seed_temp(host, ex, report).await {
                report.finish_case(format!("example {}", ex.name), mark);
                return;
            }
            // A page file's example (a page, a part or a query) runs its own
            // scenario: the steps re-run it with params, press and submit.
            if ["page", "part", "query"]
                .iter()
                .any(|k| ex.when.get(k).is_some())
            {
                dispatch(host, ext, ex, report).await;
            } else {
                scenario(host, ext, ex, report).await;
            }
        }
        Some(Err(e)) => report.errors.push(error(
            shown,
            None,
            "fixture.model",
            format!("the fixture model: {e}"),
        )),
        None => {}
    }
    report.finish_case(format!("example {}", ex.name), mark);
}

/// A fixture that isn't a page's, run as [`run_example`] says.
pub(super) async fn scenario(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    let mut binds = Binds::default();
    if !ex.given.is_null()
        && !run_steps(host, ext, shown, "given", &ex.given, &mut binds, report).await
    {
        return;
    }
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
                .fix("`steps: [{ given: [..] } | { when?, expect }]`"),
            );
            return;
        }
    };
    if (ex.steps.is_none() || !ex.expect.is_null())
        && !step_run(host, ext, ex, (&ex.when, &ex.expect), &binds, "", report).await
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
            continue;
        }
        let Some(expect) = step.get("expect") else {
            return report.errors.push(
                error(
                    shown,
                    None,
                    "fixture.step",
                    format!("`{label}` has no `given` or `expect`"),
                )
                .fix("`{ given: [..] }` or `{ when?, expect }`"),
            );
        };
        // Its `when` keys over the fixture's — or, naming another kind,
        // its own.
        let when = match step.get("when") {
            Some(own) if kind_of(own) != kind_of(&ex.when) && kind_of(own).is_some() => own.clone(),
            over => {
                let mut when = ex.when.as_object().cloned().unwrap_or_default();
                when.extend(over.and_then(Value::as_object).cloned().unwrap_or_default());
                Value::Object(when)
            }
        };
        if !step_run(host, ext, ex, (&when, expect), &binds, &label, report).await {
            return;
        }
    }
}

/// The kind a `when` names: its one kind key (`event` also rides with an
/// effect's, a policy's or a collector's).
pub(super) fn kind_of(when: &Value) -> Option<&'static str> {
    let named: Vec<&'static str> = KINDS
        .iter()
        .map(|k| k.key)
        .filter(|key| when.get(key).is_some())
        .collect();
    named
        .iter()
        .copied()
        .find(|key| *key != "event" || named.len() == 1)
}

/// Run `when` (as the fixture `ex`'s kind would) and check it against
/// `expect`, both read through `binds`; its findings name `label` (a step),
/// when it has one. False when it found anything.
pub(super) async fn step_run(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    (when, expect): (&Value, &Value),
    binds: &Binds,
    label: &str,
    report: &mut TestReport,
) -> bool {
    let shown = ex.shown;
    let at = report.errors.len();
    match (binds.substitute(when), binds.substitute(expect)) {
        (Ok(when), Ok(expect)) => {
            let run = Example {
                when,
                expect,
                given: Value::Null,
                steps: None,
                ..ex.clone()
            };
            // A page's scenario runs steps, so this nests.
            Box::pin(dispatch(host, ext, &run, report)).await;
        }
        (Err(e), _) | (_, Err(e)) => {
            report
                .errors
                .push(error(shown, None, "example.failed", e.to_string()))
        }
    }
    if !label.is_empty() {
        for found in &mut report.errors[at..] {
            if found.file.as_deref() == Some(shown) {
                found.message = format!("{label}: {}", found.message);
            }
        }
    }
    report.errors.len() == at
}

/// Run what example `ex`'s `when` names and check it against its `expect`.
pub(super) async fn dispatch(
    host: &Host,
    ext: &Extension,
    ex: &Example<'_>,
    report: &mut TestReport,
) {
    let shown = ex.shown;
    if let Some(page) = ex.when.get("page").and_then(Value::as_str) {
        page_example(host, ext, ex, page, PageKind::Page, report).await;
    } else if let Some(part) = ex.when.get("part").and_then(Value::as_str) {
        page_example(host, ext, ex, part, PageKind::Part, report).await;
    } else if let Some(query) = ex.when.get("query").and_then(Value::as_str) {
        query_example(host, ext, ex, query, report).await;
    } else if let Some(slug) = ex.when.get("form").and_then(Value::as_str) {
        form_example(host, ext, ex, slug, report).await;
    } else if let Some(id) = ex.when.get("job").and_then(Value::as_str) {
        job_example(host, ext, ex, id, report).await;
    } else if let Some(id) = ex.when.get("component").and_then(Value::as_str) {
        component_example(host, ext, ex, id, report).await;
    } else if ex.when.get("event").is_some()
        && ex.when.get("effect").is_none()
        && ex.when.get("implementation").is_none()
        && ex.when.get("collector").is_none()
    {
        event_example(host, ext, ex, report).await;
    } else if let Some(id) = ex.when.get("effect").and_then(Value::as_str) {
        effect_example(host, ext, ex, id, report).await;
    } else if let Some(id) = ex.when.get("implementation").and_then(Value::as_str) {
        policy_example(host, ext, ex, id, report).await;
    } else if let Some(t) = ex.when.get("event_type").and_then(Value::as_str) {
        event_type_example(ext, ex, t, report);
    } else if let Some(id) = ex.when.get("advisory").and_then(Value::as_str) {
        advisory_example(host, ext, ex, id, report).await;
    } else if let Some(sql) = ex.when.get("sql").and_then(Value::as_str) {
        sql_example(host, ex, sql, report).await;
    } else if let Some(link) = ex.when.get("wikilink").and_then(Value::as_str) {
        wikilink_example(ext, ex, link, report);
    } else if let Some(id) = ex.when.get("collector").and_then(Value::as_str) {
        collector_example(host, ext, ex, id, report).await;
    } else if let Some(cmd) = ex
        .when
        .get("command")
        .and_then(Value::as_str)
        .and_then(|name| ext.commands.iter().find(|c| c.name == name))
        .filter(|_| ex.when.get("answers").is_some())
    {
        command_example(host, ex, cmd, report).await;
    } else if let Some(command) = ex.when.get("command").and_then(Value::as_str) {
        command_run(host, ex, command, report).await;
    } else if ex.when.get("command").is_none() || ext.providers.is_empty() {
        report.errors.push(
            error(
                shown,
                None,
                "fixture.when",
                format!("example `{}`'s `when` names no kind of fixture", ex.name),
            )
            .fix(format!("one of {}", kinds_hint())),
        );
    }
}

/// A kind of fixture: the key its `when` names it by, the `when` it takes
/// and what its `expect` checks.
pub(super) struct Kind {
    pub(super) key: &'static str,
    pub(super) when: &'static str,
    pub(super) expect: &'static str,
}

/// Every kind of fixture the runner runs, in the order `dispatch` tries
/// them. The one statement of the grammar: the runner's hints and the
/// skill's table (`testing.md`, held to it by a test) are made from it.
pub(super) const KINDS: &[Kind] = &[
    Kind {
        key: "page",
        when: "{ page: <slug | ext/slug>, params? }",
        expect:
            "{ loaders?, nodes?, text?, alert?, badge?, label?, inactive?, actions?, see_also? }",
    },
    Kind {
        key: "part",
        when: "{ part: <slug | ext/slug>, params? }",
        expect:
            "{ loaders?, nodes?, text?, alert?, badge?, label?, inactive?, actions?, see_also? }",
    },
    Kind {
        key: "query",
        when: "{ query: <slug | ext/slug>, params? } or { query: <page>#<loader>, params? } (any page file's loader)",
        expect: "{ rows, columns? }",
    },
    Kind {
        key: "form",
        when: "{ form: <page | part>, node?, params? }",
        expect: "{ values?, fixed?, fields? }",
    },
    Kind {
        key: "job",
        when: "{ job: <id>, intentKey, request, cancel?, setup?, rows? }",
        expect: "{ state, result?, text?, toolCalls?, output?, effects?, commands?, pages? }",
    },
    Kind {
        key: "component",
        when: "{ component: <id>, items?, invokes?: [{ command, input? }] }",
        expect: "{ invokes: [{ ok, result? } | { ok: false, refused }] }",
    },
    Kind {
        key: "event",
        when: "{ event: { type, v?, payload, subject?, anchors? } }",
        expect: "{ effects?: [{ id, state }], commands?: [names] }",
    },
    Kind {
        key: "effect",
        when: "{ effect: <id>, event, answers? } (a dry run)",
        expect: "{ commands } | { skip } | { reacts: false }",
    },
    Kind {
        key: "implementation",
        when: "{ implementation: <id>, event, answers? } (a dry run)",
        expect: "{ commands } | { skip }",
    },
    Kind {
        key: "event_type",
        when: "{ event_type, v?, payload }",
        expect: "{ valid, upcast? }",
    },
    Kind {
        key: "advisory",
        when: "{ advisory: <id>, thread? }",
        expect: "{ lines: [text] }",
    },
    Kind {
        key: "sql",
        when: "{ sql, params? }",
        expect: "{ rows, columns? }",
    },
    Kind {
        key: "wikilink",
        when: "{ wikilink }",
        expect: "{ uri }",
    },
    Kind {
        key: "collector",
        when: "{ collector: <id>, rows?, event? } | { collector: <id>, files }",
        expect: "{ entities?, events? } | { facts }",
    },
    Kind {
        key: "command",
        when: "{ command, input?, as?, confirmed? } (with answers: the extension's own, a dry run)",
        expect: "{ ok, result?, commands? } | { ok: false, refused }",
    },
];

/// The kinds, for a finding's fix: each `when` and its `expect`.
pub(super) fn kinds_hint() -> String {
    KINDS
        .iter()
        .map(|k| format!("{}: `when: {}`, `expect: {}`", k.key, k.when, k.expect))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The skill's table of fixture kinds (`testing.md`).
#[cfg(test)]
pub(super) fn kinds_table() -> String {
    let mut out = String::from("| kind | `when` | `expect` |\n|---|---|---|\n");
    for k in KINDS {
        // A `|` inside a cell would split it.
        let cell = |text: &str| text.replace('|', "\\|");
        out.push_str(&format!(
            "| {} | `{}` | `{}` |\n",
            k.key,
            cell(k.when),
            cell(k.expect)
        ));
    }
    out
}

/// Whether example `ex` changes its oxplow's data (and what reacts to
/// that): it has steps or rows, or runs a job, logs an event or files a
/// component's items. A dry run (a command's, an effect's, a policy's) or a
/// collector run that stores nothing doesn't.
pub(super) fn makes_data(ex: &Example<'_>) -> bool {
    let input = &ex.when;
    let has = |key: &str| input.get(key).is_some();
    !ex.given.is_null()
        || ex.steps.is_some()
        || has("rows")
        || has("setup")
        || has("job")
        || has("component")
        || has("advisory")
        || (has("event") && !has("effect") && !has("implementation") && !has("collector"))
}

/// Why an example's target isn't here: the page, form, job, component,
/// effect, collector or command it names was withheld because an import is
/// unavailable. `None` when it's here, or wasn't withheld.
pub(super) fn withheld_target(ext: &Extension, input: &Value) -> Option<String> {
    use oxplow_domain::contribution::ContributionKind as Kind;
    let named = |key: &str| input.get(key).and_then(Value::as_str);
    let command = named("command").map(|name| {
        name.strip_prefix(&format!("{}.", ext.namespace))
            .unwrap_or(name)
    });
    let target = [
        (Kind::Page, named("page")),
        (Kind::Page, named("part")),
        (
            Kind::Page,
            named("query").map(|q| q.split('#').next().unwrap_or(q)),
        ),
        (Kind::Page, named("form")),
        (Kind::Job, named("job")),
        (Kind::CustomComponent, named("component")),
        (Kind::Effect, named("effect")),
        (Kind::Collector, named("collector")),
        (Kind::Command, command),
    ];
    let withheld = ext.public_contracts.withheld.iter().find(|withheld| {
        target.iter().any(|(kind, local)| {
            withheld.contribution.kind == *kind
                && Some(withheld.contribution.local.as_str()) == *local
        })
    })?;
    let id = &withheld.contribution;
    if withheld.imports.is_empty() {
        return Some(format!(
            "`{id}` is withheld because what it mounts from another extension was withheld"
        ));
    }
    let lines = crate::import_lines(ext);
    let why: Vec<_> = withheld
        .imports
        .iter()
        .map(|binding| {
            let state = ext
                .public_contracts
                .bindings
                .iter()
                .zip(&lines)
                .find(|(b, _)| &b.id == binding)
                .and_then(|(_, line)| line.split_once(": ").map(|(_, state)| state.to_owned()))
                .unwrap_or_else(|| "unavailable".to_owned());
            format!("import `{binding}` is {state}")
        })
        .collect();
    Some(format!("`{id}` is withheld because {}", why.join(", and ")))
}

/// The example's `model: { replies: [...] }` — what the fixture model
/// answers its model calls with, in turn (a string, or `{ text?,
/// toolCalls?, chunkDelayMs? }`); none when it gives none. `None`, with
/// the error reported, when it isn't that shape.
pub(super) fn model_replies(
    ex: &Example<'_>,
    report: &mut TestReport,
) -> Option<Vec<FixtureReply>> {
    if ex.model.is_null() {
        return Some(Vec::new());
    }
    let model = &ex.model;
    let replies = model
        .get("replies")
        .filter(|r| r.is_array())
        .map(|r| serde_json::from_value::<Vec<FixtureReply>>(r.clone()));
    match replies {
        Some(Ok(replies)) => Some(replies),
        Some(Err(e)) => {
            report.errors.push(error(ex.shown, ex.line("model"), "fixture.model", format!("example `{}`: a model reply is a string or `{{ text?, toolCalls?: [{{ name, arguments }}], chunkDelayMs? }}`: {e}", ex.name)));
            None
        }
        None => {
            report.errors.push(error(ex.shown, ex.line("model"), "fixture.model", format!("example `{}`: `model` must be `{{ replies: [ … ] }}` — what the fixture model answers, in turn", ex.name)));
            None
        }
    }
}

/// What `ext` contributes that only a person sees in the app — this host
/// renders nothing, so these aren't run here.
pub(super) fn unsupported(ext: &Extension) -> Vec<String> {
    let mut out = Vec::new();
    let mut note = |n: usize, what: &str| {
        if n > 0 {
            out.push(format!("{n} {what}"));
        }
    };
    note(ext.panels.len(), "panel(s)");
    note(ext.ui.decorators.len(), "decorator(s)");
    note(ext.ui.replacements.len(), "replacement(s)");
    note(ext.skills.len(), "skill(s)");
    out
}
