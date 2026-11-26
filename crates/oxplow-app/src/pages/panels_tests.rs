//! A left-nav panel as one thing: what each part runs with, the header's
//! count, the toggle a body's choice param gives, and the panel run whole.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;

use super::panels::*;
use super::test_support::write;
use crate::extension::extensions::{ExtensionPanel, PanelDataScope};
use crate::pages::spec::ViewContext;

/// A thread panel with every part: a body with a `mode` toggle, a collapsed
/// line, a count page and a badge that fires on two rows.
async fn project() -> (tempfile::TempDir, std::sync::Arc<crate::Services>) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = ".oxplow/extensions/acme";
    write(
        dir.path(),
        &format!("{ext}/extension.yaml"),
        "manifest: 2\nname: acme\nintent:\n  purpose: test\npanels:\n  - { id: things, title: Things, data_scope: thread, body: body, collapsed: line, count: n, badge: alarm }\n  - { id: plain, title: Plain, data_scope: project, body: plain }\n",
    );
    write(
        dir.path(),
        &format!("{ext}/parts/body.yaml"),
        r#"
title: Things
params:
  - { name: thread_id }
  - { name: mode, default: few, options: [{ value: few, label: Few }, { value: many, label: Many }] }
loaders:
  things: { sql: "SELECT :thread_id AS thread, :mode AS mode" }
view: { viz: table, from: things }
"#,
    );
    write(
        dir.path(),
        &format!("{ext}/parts/line.yaml"),
        "title: Things line\nparams: [{ name: thread_id }, { name: mode, default: few, options: [{ value: few, label: Few }, { value: many, label: Many }] }]\nloaders:\n  line: { sql: \"SELECT 'thread ' || :thread_id AS line, :mode AS mode\" }\nview: { viz: list, from: line }\n",
    );
    write(
        dir.path(),
        &format!("{ext}/parts/n.yaml"),
        "title: Things count\nparams: [{ name: thread_id }, { name: mode, default: few, options: [{ value: few, label: Few }, { value: many, label: Many }] }]\nloaders:\n  n: { sql: \"SELECT 1 AS x UNION ALL SELECT 2 UNION ALL SELECT 3\" }\nview: { viz: table, from: n }\n",
    );
    write(
        dir.path(),
        &format!("{ext}/parts/alarm.yaml"),
        "title: Things alarm\nparams: [{ name: thread_id }]\nloaders:\n  hot: { sql: \"SELECT 1 AS x UNION ALL SELECT 2\" }\nalert: { min_rows: 2, label: hot }\nview: { viz: table, from: hot }\n",
    );
    write(
        dir.path(),
        &format!("{ext}/parts/plain.yaml"),
        "title: Plain\nloaders:\n  one: { sql: \"SELECT 1 AS one\" }\nview: { viz: number, from: one }\n",
    );
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    // A project extension runs once a person deploys it.
    crate::extension::extension_deploy::deploy(&svc, "acme", true)
        .await
        .unwrap();
    (dir, svc)
}

fn panel(data_scope: PanelDataScope) -> ExtensionPanel {
    ExtensionPanel {
        id: "acme/things".into(),
        extension: "acme".into(),
        title: "Things".into(),
        icon: None,
        data_scope,
        body: "acme/body".into(),
        badge: Some("acme/alarm".into()),
        open: None,
        collapsed: Some("acme/line".into()),
        count: Some("acme/n".into()),
        dock: false,
    }
}

#[test]
fn each_part_names_its_page() {
    let p = panel(PanelDataScope::Thread);
    assert_eq!(page_of(&p, PanelPart::Body), Some("acme/body"));
    assert_eq!(page_of(&p, PanelPart::Collapsed), Some("acme/line"));
    assert_eq!(page_of(&p, PanelPart::Count), Some("acme/n"));
    assert_eq!(page_of(&p, PanelPart::Badge), Some("acme/alarm"));
    let bare = ExtensionPanel {
        badge: None,
        collapsed: None,
        count: None,
        ..p
    };
    assert_eq!(page_of(&bare, PanelPart::Badge), None);
}

#[tokio::test]
async fn a_thread_panel_runs_every_part_for_the_thread_and_the_toggle_only_on_the_body() {
    let (_dir, svc) = project().await;
    let ctx = ViewContext {
        stream_id: Some(1),
        thread_id: Some(1),
        device_id: None,
        active: None,
    };
    let run = run_panel(&svc, "acme/things", ctx.clone(), &BTreeMap::new())
        .await
        .unwrap();
    let cell = |r: &crate::pages::runner::PageRun, loader: &str, col: usize| {
        r.loaders[loader].result.rows[0][col].clone()
    };
    assert_eq!(
        cell(&run.body, "things", 0),
        SqlCell::Int(1),
        "the scope's id comes from the view"
    );
    assert_eq!(
        cell(&run.body, "things", 1),
        SqlCell::Text("few".into()),
        "the toggle's default"
    );
    assert_eq!(
        cell(run.collapsed.as_ref().unwrap(), "line", 0),
        SqlCell::Text("thread 1".into())
    );
    assert_eq!(
        run.choices,
        [PanelChoice {
            name: "mode".into(),
            label: "mode".into(),
            options: vec![("few", "Few"), ("many", "Many")]
                .into_iter()
                .map(|(v, l)| crate::pages::spec::ChoiceOption {
                    value: v.into(),
                    label: l.into(),
                    icon: None,
                })
                .collect(),
            value: "few".into(),
        }]
    );
    // The count page's rows win over the firing badge's; the badge still alerts.
    assert_eq!(run.count, Some(3));
    let alert = run.alert.clone().unwrap();
    assert!(alert.firing);
    assert_eq!(alert.count, 2);

    let picks = BTreeMap::from([("mode".to_string(), "many".to_string())]);
    let run = run_panel(&svc, "acme/things", ctx.clone(), &picks)
        .await
        .unwrap();
    assert_eq!(cell(&run.body, "things", 1), SqlCell::Text("many".into()));
    // The collapsed view is the same view, smaller: a toggle it declares
    // too has the body's value.
    assert_eq!(
        cell(run.collapsed.as_ref().unwrap(), "line", 1),
        SqlCell::Text("many".into())
    );
    assert_eq!(run.choices[0].value, "many");
    // A pick the body no longer offers falls back to the default, not an error.
    let gone = BTreeMap::from([("mode".to_string(), "gone".to_string())]);
    let run = run_panel(&svc, "acme/things", ctx, &gone).await.unwrap();
    assert_eq!(cell(&run.body, "things", 1), SqlCell::Text("few".into()));
}

#[tokio::test]
async fn a_project_panel_runs_with_no_scope_and_says_itself_as_text() {
    let (_dir, svc) = project().await;
    let run = run_panel(&svc, "acme/plain", ViewContext::default(), &BTreeMap::new())
        .await
        .unwrap();
    assert!(run.collapsed.is_none() && run.badge.is_none() && run.count_run.is_none());
    assert_eq!(run.count, None);
    let text = panel_text(&svc, &run).await.unwrap();
    assert!(text.starts_with("# Plain"), "{text}");
    let err = run_panel(&svc, "acme/nope", ViewContext::default(), &BTreeMap::new())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("acme/things"),
        "an unknown panel names the ones there are: {err}"
    );
}

#[test]
fn the_header_counts_by_the_count_page_else_the_badge() {
    let p = panel(PanelDataScope::Thread);
    assert_eq!(counting_part(&p), Some(PanelPart::Count));
    let badge_only = ExtensionPanel {
        count: None,
        ..p.clone()
    };
    assert_eq!(counting_part(&badge_only), Some(PanelPart::Badge));
    let bare = ExtensionPanel {
        badge: None,
        ..badge_only
    };
    assert_eq!(counting_part(&bare), None);
}

#[test]
fn a_badge_counts_its_firing_alert_else_its_value() {
    use crate::pages::spec::AlertState;
    let alert = |firing| AlertState {
        firing,
        count: 4,
        value: None,
        message: "4 rows".into(),
    };
    assert_eq!(badge_count(Some(&alert(true)), Some(9)), Some(4));
    // A quiet alert says nothing, whatever the value.
    assert_eq!(badge_count(Some(&alert(false)), Some(9)), None);
    // A badge with a value and no alert: its value.
    assert_eq!(badge_count(None, Some(7)), Some(7));
    assert_eq!(badge_count(None, None), None);
}

#[tokio::test]
async fn each_part_run_says_what_it_gives_its_panel() {
    let (_dir, svc) = project().await;
    let ctx = ViewContext {
        stream_id: Some(1),
        thread_id: Some(1),
        device_id: None,
        active: None,
    };
    let picks = BTreeMap::from([("mode".to_string(), "many".to_string())]);
    let run = run_panel(&svc, "acme/things", ctx, &picks).await.unwrap();
    let p = &run.panel;
    let count = part_view(p, PanelPart::Count, run.count_run.as_ref().unwrap());
    assert!(count.counts);
    assert_eq!(count.count, Some(3));
    // The badge alerts, but the header counts by the count page.
    let badge = part_view(p, PanelPart::Badge, run.badge.as_ref().unwrap());
    assert!(!badge.counts);
    assert_eq!(badge.count, None);
    // The body says the toggles, with the value it ran with.
    let body = part_view(p, PanelPart::Body, &run.body);
    assert!(!body.counts);
    assert_eq!(body.choices, run.choices);
    assert_eq!(body.choices[0].value, "many");
}

#[tokio::test]
async fn an_open_part_run_carries_its_view_on_every_snapshot() {
    let (_dir, svc) = project().await;
    let ctx = ViewContext {
        stream_id: Some(1),
        thread_id: Some(1),
        device_id: None,
        active: None,
    };
    let p = find_panel(&svc, "acme/things").await.unwrap();
    let req = part_request(&svc, &p, PanelPart::Count, ctx, &BTreeMap::new())
        .await
        .unwrap()
        .unwrap();
    let (id, _) = svc
        .page_runs
        .open_part(&svc, req, "win", &p, PanelPart::Count)
        .await
        .unwrap();
    let snap = svc.page_runs.snapshot(&id).unwrap();
    let view = snap.panel.expect("a panel part's snapshot says its view");
    assert!(view.counts);
    assert_eq!(view.count, Some(3));
    // A plain page's run has none.
    let plain = crate::pages::runner::RunRequest {
        target: crate::pages::runner::RunTarget::Page("acme/plain".into()),
        params: BTreeMap::new(),
        ctx: ViewContext::default(),
        viewer: oxplow_domain::Actor::Human,
    };
    let (id, _) = svc.page_runs.open(&svc, plain, "win").await.unwrap();
    assert!(svc.page_runs.snapshot(&id).unwrap().panel.is_none());
}

/// What the window opens for each part: the body with the toggles in force
/// (a pick it no longer offers is its default), every other part with no
/// param of its own, and nothing for a part the panel hasn't.
#[tokio::test]
async fn a_part_request_names_its_page_and_every_part_that_declares_a_toggle_takes_it() {
    use crate::pages::runner::RunTarget;
    let (_dir, svc) = project().await;
    let things = find_panel(&svc, "acme/things").await.unwrap();
    let ctx = ViewContext {
        stream_id: Some(1),
        thread_id: Some(1),
        device_id: None,
        active: None,
    };
    let gone = BTreeMap::from([("mode".to_string(), "gone".to_string())]);
    let body = part_request(&svc, &things, PanelPart::Body, ctx.clone(), &gone)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&body.target, RunTarget::Page(p) if p == "acme/body"));
    assert_eq!(
        body.params,
        BTreeMap::from([("mode".to_string(), SqlCell::Text("few".into()))])
    );
    let line = part_request(&svc, &things, PanelPart::Collapsed, ctx.clone(), &gone)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        line.params,
        BTreeMap::from([("mode".to_string(), SqlCell::Text("few".into()))])
    );
    let count = part_request(&svc, &things, PanelPart::Count, ctx.clone(), &gone)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&count.target, RunTarget::Page(p) if p == "acme/n"));
    // Every part that declares the toggle follows the header: the count
    // counts what the body shows.
    assert_eq!(
        count.params,
        BTreeMap::from([("mode".to_string(), SqlCell::Text("few".into()))])
    );
    let alarm = part_request(&svc, &things, PanelPart::Badge, ctx.clone(), &gone)
        .await
        .unwrap()
        .unwrap();
    assert!(alarm.params.is_empty(), "the badge declares no toggle");
    let plain = find_panel(&svc, "acme/plain").await.unwrap();
    assert!(part_request(&svc, &plain, PanelPart::Badge, ctx, &gone)
        .await
        .unwrap()
        .is_none());
}
