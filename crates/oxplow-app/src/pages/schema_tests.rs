//! The page file format: what parses, what it loads to, and what each
//! load-time check refuses.

use super::schema::*;
use crate::pages::spec::Viz;

fn page(text: &str) -> Result<Page, String> {
    parse_page(
        "acme",
        "acme",
        "busy",
        "extensions/acme/pages/busy.yaml",
        text,
    )
}

fn refused(text: &str) -> String {
    page(text).expect_err("refused")
}

/// A page's loaders are under `loaders:`; the old `data:` is refused, and
/// the refusal says what to write.
#[test]
fn a_pages_loaders_are_under_loaders_and_data_is_refused_naming_it() {
    let body = "view:\n  viz: table\n  from: q\n";
    let ok = page(&format!(
        "title: T\nloaders:\n  q: {{ rows: [{{ a: 1 }}] }}\n{body}"
    ))
    .expect("loads");
    assert_eq!(ok.loaders.len(), 1);
    let why = refused(&format!(
        "title: T\ndata:\n  q: {{ rows: [{{ a: 1 }}] }}\n{body}"
    ));
    assert!(why.contains("unknown key `data`"), "{why}");
    assert!(why.contains("write `loaders:`"), "{why}");
}

/// A loader's SQL is read with the one SQL tokenizer: an apostrophe in a
/// comment doesn't hide the loaders and params after it, a word or
/// `:param` in a comment or a string isn't read, and a quoted identifier
/// names its loader.
#[test]
fn a_loaders_sql_is_read_with_the_one_sql_tokenizer() {
    let p = page(
        "title: T\nloaders:\n  a: { rows: [{ x: 1 }] }\n  b: { rows: [{ y: 1 }] }\n  q:\n    sql: |\n      -- the block's fields, not b or :gone\n      SELECT x FROM \"a\" /* nor b's */\n      WHERE x = :kept AND 'b :not' <> x\nview: { viz: table, from: q }\n",
    )
    .expect("loads");
    let q = p.loader("q").unwrap();
    assert_eq!(q.reads_loaders, ["a"]);
    assert_eq!(q.reads_params, ["kept"]);
}

/// A page that shows one query: one loader, one viz node bound to it.
#[test]
fn a_one_to_one_page_is_a_loader_and_a_viz_node() {
    let p = page(
        r#"
title: Problems
icon: triangle-alert
category: Code
params:
  - { name: stream_id }
  - name: severity
    role: filter
    default: all
    options: [{ value: all, label: All }, { value: error, label: Errors }]
loaders:
  problems:
    sql: SELECT path, message FROM v_diagnostic WHERE stream_id = :stream_id
view:
  viz: table
  from: problems
  columns: [{ key: path }, { key: message, label: Problem }]
  empty: No problems.
"#,
    )
    .unwrap();
    assert_eq!(p.id, "acme/busy");
    assert_eq!(p.title, "Problems");
    assert_eq!(p.loaders.len(), 1);
    assert_eq!(p.loaders[0].name, "problems");
    assert!(
        matches!(&p.loaders[0].loader, Loader::Sql { sql, .. } if sql.contains("v_diagnostic"))
    );
    let NodeKind::Viz { from, config } = &p.view.kind else {
        panic!("{:?}", p.view)
    };
    assert_eq!(from.as_deref(), Some("problems"));
    assert_eq!(config.viz, Viz::Table);
    assert_eq!(config.columns.len(), 2);
    assert_eq!(config.empty.as_deref(), Some("No problems."));
    // A viz node's id defaults to the loader it shows.
    assert_eq!(p.view.id, "problems");
    // Its params are the page's, typed.
    assert_eq!(config.params.len(), 2);
}

/// Later loaders read earlier ones as tables; a stack holds nodes, an
/// embed and a slot; ids are unique across the tree.
#[test]
fn a_composite_page_shares_its_loaders() {
    let p = page(
        r#"
title: Commit
label: "{{param.sha|short}}"
for: oxplow/commit
default: true
params: [{ name: uri }, { name: sha }, { name: stream_id }]
loaders:
  change:
    sql: SELECT id AS change_id, base_revision FROM v_change WHERE target = :sha
    columns: [{ key: change_id }, { key: base_revision }]
  files:
    sql: SELECT f.path, c.base_revision FROM v_commit_file f, change c WHERE f.sha = :sha
view:
  stack:
    - { id: files, title: Files, viz: tree, from: files, tree: { id: path, parent: path, label: path } }
    - { embed: { page: other/review, params: { change: { from: change.change_id } } } }
    - { slot: sections }
    - { markdown: "Some *notes*." , id: notes }
"#,
    )
    .unwrap();
    assert_eq!(p.renders, vec!["oxplow/commit".to_string()]);
    assert!(p.default);
    let Loader::Sql { .. } = &p.loaders[1].loader else {
        panic!()
    };
    assert_eq!(p.loaders[1].reads_loaders, vec!["change".to_string()]);
    let NodeKind::Stack { children } = &p.view.kind else {
        panic!()
    };
    assert_eq!(children.len(), 4);
    let NodeKind::Embed { page, params } = &children[1].kind else {
        panic!()
    };
    assert_eq!(page, "other/review");
    assert_eq!(
        params.get("change"),
        Some(&ArgSource::Column {
            from: "change.change_id".into()
        })
    );
    assert_eq!(children[1].id, "review");
    assert!(matches!(&children[2].kind, NodeKind::Slot { name } if name == "sections"));
    assert_eq!(children[2].id, "sections");
    assert!(matches!(&children[3].kind, NodeKind::Markdown { text } if text.contains("notes")));
}

#[test]
fn command_and_rows_loaders_and_a_component_node() {
    let p = page(
        r#"
title: PR
params: [{ name: number }]
loaders:
  checks:
    command: acme.ci.checks
    args: { pr: { param: number }, limit: 5 }
    reads: [v_acme_check]
  colors:
    rows: [{ state: failed, tone: danger }, { state: passed, tone: success }]
view:
  stack:
    - { id: timeline, component: { id: pr-timeline, props: { compact: true } } }
    - { viz: list, from: colors }
badge: { loader: checks }
answers:
  - { question: "Which checks failed on this PR?", params: { number: 12 } }
"#,
    )
    .unwrap();
    let Loader::Command { command, args, .. } = &p.loaders[0].loader else {
        panic!()
    };
    assert_eq!(command, "acme.ci.checks");
    assert_eq!(
        args.get("pr"),
        Some(&ArgSource::Param {
            param: "number".into()
        })
    );
    assert_eq!(
        args.get("limit"),
        Some(&ArgSource::Literal(serde_json::json!(5)))
    );
    assert_eq!(p.loaders[0].reads, vec!["v_acme_check".to_string()]);
    let Loader::Rows { columns, rows } = &p.loaders[1].loader else {
        panic!()
    };
    assert_eq!(columns, &vec!["state".to_string(), "tone".to_string()]);
    assert_eq!(rows.len(), 2);
    assert_eq!(p.badge.as_ref().map(|b| b.loader.as_str()), Some("checks"));
    assert_eq!(p.answers.len(), 1);
}

#[test]
fn a_switch_shows_one_child_by_a_filter_param() {
    let p = page(
        r#"
title: Settings
params:
  - { name: area, role: filter, default: agents }
loaders:
  agents: { sql: SELECT 1 AS n }
  ai: { sql: SELECT 2 AS n }
view:
  switch:
    param: area
    children:
      - { id: agents, viz: number, from: agents }
      - { id: ai, viz: number, from: ai }
"#,
    )
    .unwrap();
    let NodeKind::Switch { param, children } = &p.view.kind else {
        panic!()
    };
    assert_eq!(param, "area");
    assert_eq!(children.len(), 2);
}

// --- what is refused ---------------------------------------------------

#[test]
fn a_loader_names_exactly_one_kind() {
    let e = refused(
        "title: T\nloaders:\n  x: { sql: SELECT 1, rows: [] }\nview: { viz: table, from: x }\n",
    );
    assert!(
        e.contains("loader `x`") && e.contains("exactly one of"),
        "{e}"
    );
    let e = refused("title: T\nloaders:\n  x: {}\nview: { viz: table, from: x }\n");
    assert!(e.contains("exactly one of"), "{e}");
}

#[test]
fn a_loader_name_is_a_plain_identifier_and_not_a_dataset() {
    let e = refused(
        "title: T\nloaders:\n  v_commit: { sql: SELECT 1 }\nview: { viz: table, from: v_commit }\n",
    );
    assert!(e.contains("`v_commit`") && e.contains("dataset"), "{e}");
    let e = refused(
        "title: T\nloaders:\n  Bad-Name: { sql: SELECT 1 }\nview: { viz: table, from: Bad-Name }\n",
    );
    assert!(e.contains("`Bad-Name`"), "{e}");
}

#[test]
fn a_loader_reads_only_earlier_loaders() {
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT * FROM b }\n  b: { sql: SELECT 1 AS n }\nview: { viz: table, from: a }\n",
    );
    assert!(
        e.contains("loader `a` reads `b`") && e.contains("before"),
        "{e}"
    );
    let e = refused(
        "title: T\nloaders:\n  a: { command: acme.ci.x, args: { n: { from: b.n } }, reads: [v_x] }\n  b: { sql: SELECT 1 AS n }\nview: { viz: table, from: a }\n",
    );
    assert!(e.contains("loader `a`") && e.contains("`b`"), "{e}");
}

#[test]
fn a_node_binds_a_loader_that_exists() {
    let e =
        refused("title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: nope }\n");
    assert!(e.contains("`nope`") && e.contains("no loader"), "{e}");
    let e = refused("title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table }\n");
    assert!(e.contains("needs `from`"), "{e}");
}

#[test]
fn a_command_loader_says_what_refreshes_it() {
    let e =
        refused("title: T\nloaders:\n  a: { command: acme.ci.x }\nview: { viz: table, from: a }\n");
    assert!(e.contains("`reads`") && e.contains("`refresh`"), "{e}");
    page("title: T\nloaders:\n  a: { command: acme.ci.x, refresh: manual }\nview: { viz: table, from: a }\n")
        .unwrap();
}

#[test]
fn an_arg_comes_from_a_param_an_earlier_column_or_a_literal() {
    let e = refused(
        "title: T\nloaders:\n  a: { command: acme.ci.x, args: { n: { param: nope } }, reads: [v_x] }\nview: { viz: table, from: a }\n",
    );
    assert!(e.contains("param `nope`"), "{e}");
}

#[test]
fn node_ids_are_unique() {
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview:\n  stack:\n    - { viz: table, from: a }\n    - { viz: list, from: a }\n",
    );
    assert!(e.contains("node id `a`") && e.contains("twice"), "{e}");
}

#[test]
fn a_switch_param_is_a_declared_filter_and_its_children_have_ids() {
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { switch: { param: area, children: [{ id: a, viz: table, from: a }] } }\n",
    );
    assert!(e.contains("param `area`"), "{e}");
}

#[test]
fn a_select_names_a_page_param() {
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 AS n }\nview: { viz: table, from: a, columns: [{ key: n, select: nope }] }\n",
    );
    assert!(e.contains("nope") && e.contains("select"), "{e}");
}

#[test]
fn a_viz_node_has_what_its_viz_needs() {
    let e = refused("title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: tree, from: a }\n");
    assert!(e.contains("tree"), "{e}");
}

#[test]
fn badges_and_answers_name_what_the_page_has() {
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: a }\nbadge: { loader: nope }\n",
    );
    assert!(e.contains("badge") && e.contains("`nope`"), "{e}");
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: a }\nanswers: [{ question: Why, params: { nope: 1 } }]\n",
    );
    assert!(e.contains("answer") && e.contains("`nope`"), "{e}");
}

#[test]
fn unknown_keys_are_refused() {
    let e = refused("title: T\nquery: SELECT 1\nview: { markdown: hi, id: m }\n");
    assert!(e.contains("query"), "{e}");
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: a, sections: [] }\n",
    );
    assert!(e.contains("sections"), "{e}");
}

/// An inline page (a markdown block's, an agent's one-off) only reads.
#[test]
fn an_inline_page_only_reads() {
    let ok = parse_inline_page(
        "title: T\nloaders:\n  a: { sql: SELECT 1 AS n }\nview: { viz: bar, from: a, chart: { x: n, y: n } }\n",
    )
    .unwrap();
    assert_eq!(ok.id, "inline");
    for (text, why) in [
        (
            "title: T\nloaders:\n  a: { command: acme.ci.x, refresh: manual }\nview: { viz: table, from: a }\n",
            "command",
        ),
        (
            "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: a }\nactions: [{ id: x, label: X, command: acme.ci.x, input: {} }]\n",
            "actions",
        ),
        (
            "title: T\nview: { id: c, component: { id: x } }\n",
            "component",
        ),
        ("title: T\nview: { slot: sections }\n", "slot"),
        (
            "title: T\nview: { viz: form, form: { command: acme.ci.x } }\n",
            "form",
        ),
    ] {
        let e = parse_inline_page(text).expect_err(why);
        assert!(e.contains(why), "{why}: {e}");
    }
}

/// An extension's `pages/*.yaml` load as its page files; a broken one is an
/// error naming its file, and the rest still load.
#[test]
fn an_extensions_page_files_load_and_a_broken_one_names_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let ext = dir.path().join("acme");
    std::fs::create_dir_all(ext.join("pages")).unwrap();
    std::fs::write(
        ext.join("extension.yaml"),
        "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("pages/busy.yaml"),
        "title: Busy\nloaders:\n  a: { sql: SELECT 1 AS n }\nview: { viz: table, from: a }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("pages/broken.yaml"),
        "title: Broken\nview: { viz: table }\n",
    )
    .unwrap();
    let loaded = crate::extension::extensions::load_extension_at(&ext, "acme");
    assert_eq!(
        loaded
            .page_files
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["acme/busy"]
    );
    assert!(
        loaded
            .errors
            .iter()
            .any(|e| e.contains("pages/broken.yaml") && e.contains("needs `from`")),
        "{:?}",
        loaded.errors
    );
}

/// A query binds an earlier loader's column (or a literal) by name, as
/// a composite's row once reached its sections.
#[test]
fn a_sql_loader_binds_named_values_from_earlier_loaders() {
    let p = page(
        "title: T\nloaders:\n  row: { sql: SELECT 'abc' AS head }\n  files:\n    sql: SELECT :head AS h\n    params: { head: { from: row.head } }\nview: { viz: table, from: files }\n",
    )
    .unwrap();
    let Loader::Sql { params, .. } = &p.loaders[1].loader else {
        panic!()
    };
    assert_eq!(
        params.get("head"),
        Some(&ArgSource::Column {
            from: "row.head".into()
        })
    );
    assert_eq!(p.loaders[1].reads_loaders, ["row"]);
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT :x, params: { x: { from: b.x } } }\n  b: { sql: SELECT 1 AS x }\nview: { viz: table, from: a }\n",
    );
    assert!(e.contains("loader `a`") && e.contains("`b`"), "{e}");
}

/// A panel's parts may be its extension's pages: the body, a badge page
/// that declares a `badge`, a stream panel's pages taking `stream_id`.
#[test]
fn a_panel_names_its_extensions_pages() {
    let dir = tempfile::tempdir().unwrap();
    let ext = dir.path().join("acme");
    std::fs::create_dir_all(ext.join("pages")).unwrap();
    std::fs::write(
        ext.join("extension.yaml"),
        "manifest: 2\nname: acme\nintent:\n  purpose: test\npanels:\n  - { id: work, title: Work, scope: stream, body: list, badge: count }\n  - { id: bad, title: Bad, scope: project, body: list, badge: list }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("pages/list.yaml"),
        "title: List\nparams: [{ name: stream_id }]\nloaders:\n  a: { sql: SELECT 1 AS n }\nview: { viz: table, from: a }\n",
    )
    .unwrap();
    std::fs::write(
        ext.join("pages/count.yaml"),
        "title: Count\nparams: [{ name: stream_id }]\nloaders:\n  c: { sql: SELECT 3 AS n }\nview: { viz: number, from: c }\nbadge: { loader: c }\n",
    )
    .unwrap();
    let loaded = crate::extension::extensions::load_extension_at(&ext, "acme");
    let work = loaded
        .panels
        .iter()
        .find(|p| p.id == "acme/work")
        .unwrap_or_else(|| panic!("{:?}", loaded.errors));
    assert_eq!(work.body, "acme/list");
    assert_eq!(work.badge.as_deref(), Some("acme/count"));
    assert!(
        loaded
            .errors
            .iter()
            .any(|e| e.contains("panel `bad`") && e.contains("no `badge` or `alert`")),
        "{:?}",
        loaded.errors
    );
}

/// oxplow's own extensions share the core page names
/// (`oxplow:oxplow/page?page=<slug>`): two declaring one slug would make
/// that URI ambiguous, so the later one loses its page and is told.
#[test]
fn two_core_named_extensions_cannot_declare_one_page() {
    let d = tempfile::tempdir().unwrap();
    for name in ["first", "second"] {
        super::test_support::write(
            d.path(),
            &format!(".oxplow/extensions/{name}/extension.yaml"),
            &format!("manifest: 2\nname: {name}\nintent:\n  purpose: p\n"),
        );
        super::test_support::write(
            d.path(),
            &format!(".oxplow/extensions/{name}/pages/comments.yaml"),
            "title: Comments\nloaders:\n  rows:\n    sql: SELECT 1 AS n\nview:\n  viz: table\n  from: rows\n",
        );
    }
    let mut exts: Vec<crate::extension::extensions::Extension> =
        crate::extension::extensions::load_extensions(d.path())
            .into_iter()
            .filter(|e| e.origin == "project")
            .collect();
    for e in &mut exts {
        e.namespace = oxplow_domain::OXPLOW_NAMESPACE.into();
    }
    refuse_shared_core_pages(&mut exts);
    assert_eq!(exts[0].page_files.len(), 1, "the first keeps it");
    assert!(exts[0].errors.is_empty(), "{:?}", exts[0].errors);
    assert!(exts[1].page_files.is_empty(), "the second loses it");
    let errors = exts[1].errors.join("\n");
    assert!(
        errors.contains("page `comments` is also `first`'s"),
        "{errors}"
    );
}

/// A param a viz node's column `select:`s is the page's selection: it
/// rides in the URI's `#`, so a pick is a history entry, not a new page.
#[test]
fn a_param_a_column_selects_is_a_selection() {
    let p = page(
        r#"
title: Commits
params:
  - { name: commit }
  - { name: path }
loaders:
  commits:
    sql: SELECT sha FROM v_commit
view:
  stack:
    - id: list
      viz: table
      from: commits
      columns: [{ key: sha, select: commit }]
"#,
    )
    .unwrap();
    let role = |name: &str| p.params.iter().find(|q| q.name == name).unwrap().role;
    assert_eq!(
        role("commit"),
        Some(crate::pages::spec::ParamRole::Selection)
    );
    assert_eq!(role("path"), None);
}

/// A node another extension's page may stand in for names a known target,
/// once per page.
#[test]
fn a_replaceable_node_names_a_known_target_once() {
    let page_with = |a: &str, b: &str| {
        format!(
            "title: T\nloaders:\n  q: {{ sql: \"SELECT 1 AS n\" }}\nview:\n  stack:\n    - {{ id: a, viz: table, from: q, replaceable: {a} }}\n    - {{ id: b, viz: table, from: q, replaceable: {b} }}\n"
        )
    };
    assert!(refused(&page_with("nope.thing", "work_item.board"))
        .contains("isn't a replaceable component"));
    assert!(refused(&page_with("work_item.board", "work_item.board")).contains("placed once"));
}

/// A page's params are checked though no viz node reads them, and an
/// inline page can't open a web page.
#[test]
fn params_are_checked_on_any_page_and_inline_pages_hold_no_web_page() {
    let why = refused("title: T\nparams:\n  - { name: section }\nview:\n  markdown: hi\n");
    assert!(why.contains("view key"), "{why}");
    let why = parse_inline_page(
        "title: T\nloaders:\n  q: { sql: \"SELECT 'https://x' AS url\" }\nview:\n  viz: embed\n  from: q\n  embed: { url: url }\n",
    )
    .expect_err("refused");
    assert!(why.contains("a web page"), "{why}");
}
