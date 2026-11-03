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
        PageKind::Page,
    )
}

fn refused(text: &str) -> String {
    page(text).expect_err("refused")
}

/// A page's loaders are under `loaders:`; any other key is refused.
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
    assert!(why.contains("unknown field `data`"), "{why}");
}

/// A section — a child of a `stack`, `grid` or `switch`, in the view or the
/// rail — needs a `title`; a missing one and `''` are refused naming the
/// node and the fix. `hide_title` is how its heading is kept off the page.
#[test]
fn a_section_needs_a_title_and_hide_title_keeps_it_unseen() {
    let head = "title: T\nloaders:\n  q: { rows: [{ a: 1 }] }\n";
    let why = refused(&format!(
        "{head}view:\n  stack:\n    - {{ id: facts, viz: table, from: q }}\n"
    ));
    assert!(why.contains("node `facts`"), "{why}");
    assert!(why.contains("needs a `title`"), "{why}");
    assert!(why.contains("hide_title: true"), "{why}");
    let why = refused(&format!(
        "{head}view:\n  stack:\n    - {{ id: facts, title: '', viz: table, from: q }}\n"
    ));
    assert!(why.contains("node `facts`"), "{why}");
    let why = refused(&format!(
        "{head}view: {{ viz: table, from: q }}\nrail:\n  stack:\n    - {{ id: side, markdown: hi }}\n"
    ));
    assert!(why.contains("node `side`"), "{why}");
    let why = refused(&format!(
        "{head}view:\n  stack:\n    - title: One\n      grid:\n        - {{ id: tile, viz: table, from: q }}\n"
    ));
    assert!(why.contains("node `tile`"), "{why}");
    // A page whose view is one node, and an embed or a slot (titled by the
    // page it shows), need none.
    page(&format!("{head}view: {{ viz: table, from: q }}\n")).expect("one node");
    page(&format!(
        "{head}view:\n  stack:\n    - {{ slot: after }}\n    - {{ embed: {{ page: acme/other }} }}\n"
    ))
    .expect("slot and embed");
    let ok = page(&format!(
        "{head}view:\n  stack:\n    - {{ id: facts, title: Facts, hide_title: true, viz: table, from: q }}\n    - {{ id: more, title: More, viz: table, from: q }}\n"
    ))
    .expect("loads");
    let by_id = |id: &str| {
        ok.nodes()
            .into_iter()
            .find(|n| n.id == id)
            .unwrap()
            .hide_title
    };
    assert!(by_id("facts"));
    assert!(!by_id("more"));
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

/// A loader isn't named for a SQL keyword SQLite won't take as a table
/// name (`order`, not `row`): the SQL that reads it couldn't name it, and
/// a later loader's `ORDER BY` would read as reading it.
#[test]
fn a_loader_named_for_a_sql_keyword_is_refused() {
    let why = refused(
        "title: T\nloaders:\n  order: { rows: [{ a: 1 }] }\nview: { viz: table, from: order }\n",
    );
    assert!(why.contains("loader `order`"), "{why}");
    assert!(why.contains("SQL keyword"), "{why}");
    page(
        "title: T\nloaders:\n  orders: { rows: [{ a: 1 }] }\nview: { viz: table, from: orders }\n",
    )
    .expect("a name that only starts like one is fine");
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
    - { title: Notes, markdown: "Some *notes*." , id: notes }
"#,
    )
    .unwrap();
    assert_eq!(p.renders, vec!["oxplow/commit".to_string()]);
    assert!(p.default);
    // The system's `content` comes first; the author's follow.
    assert_eq!(p.loaders[0].name, "content");
    let Loader::Sql { .. } = &p.loaders[2].loader else {
        panic!()
    };
    assert_eq!(p.loaders[2].reads_loaders, vec!["change".to_string()]);
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
    - { id: timeline, title: Timeline, component: { id: pr-timeline, props: { compact: true } } }
    - { title: Colors, viz: list, from: colors }
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
      - { id: agents, title: Agents, viz: number, from: agents }
      - { id: ai, title: AI, viz: number, from: ai }
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
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview:\n  stack:\n    - { title: A, viz: table, from: a }\n    - { title: B, viz: list, from: a }\n",
    );
    assert!(e.contains("node id `a`") && e.contains("twice"), "{e}");
}

#[test]
fn a_switch_param_is_a_declared_filter_and_its_children_have_ids() {
    let e = refused(
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { switch: { param: area, children: [{ id: a, title: A, viz: table, from: a }] } }\n",
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
        "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: a, bogus: [] }\n",
    );
    assert!(e.contains("bogus"), "{e}");
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
    // A command loader's refusal says how to try one: a draft page file.
    let e = parse_inline_page(
        "title: T\nloaders:\n  a: { command: acme.ci.x, refresh: manual }\nview: { viz: table, from: a }\n",
    )
    .unwrap_err();
    assert!(e.contains("draft: true"), "{e}");
}

/// A temp page (an agent's shown view, a file on `oxplow:temp`) has the
/// inline rules, is `temp/<id>`, runs from its temp file, and is never a
/// resource page.
#[test]
fn a_temp_page_is_temp_slash_id_with_the_inline_rules() {
    let ok = parse_temp_page(
        "ab12-cd",
        "title: T\nloaders:\n  a: { sql: SELECT 1 AS n }\nview: { viz: table, from: a }\n",
    )
    .unwrap();
    assert_eq!(ok.id, "temp/ab12-cd");
    assert_eq!(ok.slug, "ab12-cd");
    assert_eq!(ok.origin.extension, "temp");
    assert_eq!(
        ok.origin.running_uri.as_deref(),
        Some("oxplow:oxplow/file?filesystem=oxplow:temp&path=ab12-cd.yaml")
    );
    assert!(
        ok.origin.running.ends_with("ab12-cd.yaml"),
        "{}",
        ok.origin.running
    );
    for (text, why) in [
        (
            "title: T\nloaders:\n  a: { command: acme.ci.x, refresh: manual }\nview: { viz: table, from: a }\n",
            "command",
        ),
        (
            "title: T\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: table, from: a }\nactions: [{ id: x, label: X, command: acme.ci.x, input: {} }]\n",
            "actions",
        ),
        ("title: T\nview: { slot: sections }\n", "slot"),
        (
            "title: T\nfor: oxplow/file\nparams: [{ name: uri }]\nloaders:\n  q: { rows: [{ a: 1 }] }\nview: { viz: table, from: q }\n",
            "for:",
        ),
    ] {
        let e = parse_temp_page("x", text).expect_err(why);
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
        loaded.errors.iter().any(|e| {
            let e = loaded.shown(e);
            e.contains("pages/broken.yaml") && e.contains("needs `from`")
        }),
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
            .any(|e| e.message.contains("panel `bad`")
                && e.message.contains("no `badge` or `alert`")),
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
    let errors = exts[1]
        .errors
        .iter()
        .map(|d| exts[1].shown(d))
        .collect::<Vec<_>>()
        .join("\n");
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
      title: Commits
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

/// `role: view` is state the page or the system sets: it rides the `#`
/// like a filter.
#[test]
fn a_view_role_parses_and_rides_the_view() {
    let p = page(
        r#"
title: Shown
params:
  - { name: mode, role: view, default: a }
view: { markdown: Hi }
"#,
    )
    .unwrap();
    let role = p.params[0].role;
    assert_eq!(role, Some(crate::pages::spec::ParamRole::View));
    assert!(role.unwrap().rides_view());
    assert!(!crate::pages::spec::ParamRole::Resource.rides_view());
}

/// A node another extension's page may stand in for names a known target,
/// once per page.
#[test]
fn a_replaceable_node_names_a_known_target_once() {
    let page_with = |a: &str, b: &str| {
        format!(
            "title: T\nloaders:\n  q: {{ sql: \"SELECT 1 AS n\" }}\nview:\n  stack:\n    - {{ id: a, title: A, viz: table, from: q, replaceable: {a} }}\n    - {{ id: b, title: B, viz: table, from: q, replaceable: {b} }}\n"
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

const BODY: &str = "view: { viz: table, from: q }\nloaders:\n  q: { rows: [{ a: 1 }] }\n";

/// A page with `for:` is a resource page: the system declares its
/// `content` loader first, configured by `content:`, reading its `uri`.
#[test]
fn a_resource_page_is_handed_a_content_loader_first() {
    let p = page(&format!(
        "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}]\n{BODY}"
    ))
    .unwrap();
    assert!(p.is_resource_page());
    let names: Vec<&str> = p.loaders.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["content", "q"]);
    assert_eq!(
        p.loaders[0].loader,
        Loader::Content {
            text: false,
            max_bytes: DEFAULT_CONTENT_BYTES
        }
    );
    assert_eq!(p.loaders[0].reads_params, ["uri"]);
    // "The page's row" is its first authored loader's.
    assert_eq!(p.first_loader().unwrap().name, "q");

    let p = page(
        "title: T\nfor: oxplow/file\nparams: [{ name: uri }]\ncontent: { text: true, max_bytes: 4096 }\nview: { viz: table, from: content }\n",
    )
    .unwrap();
    assert_eq!(
        p.loaders[0].loader,
        Loader::Content {
            text: true,
            max_bytes: 4096
        }
    );
    assert_eq!(p.first_loader().unwrap().name, "content");
}

/// A later loader reads `content` as a table or through `from:`.
#[test]
fn a_resource_pages_loaders_read_content() {
    let p = page(
        "title: T\nfor: oxplow/file\nparams: [{ name: uri }]\nloaders:\n  \
         a: { sql: 'SELECT path FROM content' }\n  \
         b: { sql: 'SELECT :p AS p', params: { p: { from: content.path } } }\n\
         view: { viz: table, from: a }\n",
    )
    .unwrap();
    assert_eq!(p.loaders[1].reads_loaders, ["content"]);
    assert_eq!(p.loaders[2].reads_loaders, ["content"]);
}

#[test]
fn a_resource_page_is_never_listed_so_it_takes_no_listing_keys() {
    for (key, line) in [
        ("category", "category: Work"),
        ("keywords", "keywords: a b"),
        ("badge", "badge: { loader: q }"),
    ] {
        let why = refused(&format!(
            "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}]\n{line}\n{BODY}"
        ));
        assert!(why.contains(&format!("takes no `{key}`")), "{why}");
        assert!(why.contains("never listed on its own"), "{why}");
    }
    // The same keys are a standalone page's.
    page(&format!("title: T\ncategory: Work\nkeywords: a\n{BODY}")).unwrap();
}

fn of_kind(kind: PageKind, text: &str) -> Result<Page, String> {
    parse_page(
        "acme",
        "acme",
        "busy",
        &format!("extensions/acme/{}/busy.yaml", kind.folder()),
        text,
        kind,
    )
}

/// A part is drawn only inside a host page: it has no tab, no launcher
/// entry and no agent-facing identity of its own, so the keys that give a
/// page those are refused, each naming what a part is.
#[test]
fn a_part_takes_no_listing_or_tab_keys() {
    for (key, line) in [
        ("category", "category: Work"),
        ("keywords", "keywords: a b"),
        ("label", "label: '{{param.x}}'"),
        ("icon", "icon: git-branch"),
        ("for", "for: oxplow/file"),
        ("accepts", "accepts: { path: present }"),
        ("default", "default: true"),
        ("pinned", "pinned: true"),
        ("answers", "answers: [{ question: q, params: {} }]"),
        ("tool", "tool: busy"),
        ("see_also", "see_also: [{ page: other }]"),
        ("rail", "rail: { viz: table, from: q }"),
        ("comments", "comments: true"),
        ("refresh", "refresh: true"),
    ] {
        let why = of_kind(
            PageKind::Part,
            &format!("title: T\nparams: [{{ name: x }}]\n{line}\n{BODY}"),
        )
        .expect_err(key);
        assert!(why.contains(&format!("takes no `{key}`")), "{key}: {why}");
        assert!(why.contains("is a part"), "{key}: {why}");
    }
    // What a part keeps: a title, params, loaders, a view, actions, an
    // alert, a badge and needs.
    let part = of_kind(
        PageKind::Part,
        &format!(
            "title: T\nneeds: [vcs]\nbadge: {{ loader: q }}\nalert: {{ min_rows: 1 }}\nactions: [{{ id: go, label: Go, command: oxplow.x.y }}]\n{BODY}"
        ),
    )
    .expect("a part loads");
    assert_eq!(part.kind, PageKind::Part);
    assert_eq!(part.file(), "parts/busy.yaml");
}

/// A query is rows only: its loaders and params, no view and nothing a
/// person looks at. Its default loader is the last one declared, and the
/// loaded page carries a synthesized table over it so the runner, the text
/// rendering and the fixtures need no case of their own.
#[test]
fn a_query_is_rows_only_and_has_no_view() {
    let why = of_kind(PageKind::Query, &format!("title: T\n{BODY}")).expect_err("a view");
    assert!(why.contains("a query has no `view:`"), "{why}");
    let why = of_kind(PageKind::Query, "title: T\n").expect_err("no loaders");
    assert!(why.contains("a query needs `loaders:`"), "{why}");
    for (key, line) in [
        (
            "actions",
            "actions: [{ id: go, label: Go, command: oxplow.x.y }]",
        ),
        ("badge", "badge: { loader: q }"),
        ("alert", "alert: { min_rows: 1 }"),
        ("category", "category: Work"),
        ("answers", "answers: [{ question: q, params: {} }]"),
    ] {
        let why = of_kind(
            PageKind::Query,
            &format!("title: T\n{line}\nloaders:\n  q: {{ rows: [{{ a: 1 }}] }}\n"),
        )
        .expect_err(key);
        assert!(why.contains(&format!("takes no `{key}`")), "{key}: {why}");
        assert!(why.contains("is a query"), "{key}: {why}");
    }
    let query = of_kind(
        PageKind::Query,
        "title: T\ntool: busy\nneeds: [vcs]\nparams: [{ name: x }]\nloaders:\n  first: { rows: [{ a: 1 }] }\n  last: { sql: 'SELECT a AS value, a AS label FROM first WHERE :x IS NULL' }\n",
    )
    .expect("a query loads");
    assert_eq!(query.kind, PageKind::Query);
    assert_eq!(query.file(), "queries/busy.yaml");
    assert_eq!(
        query.default_loader().map(|l| l.name.as_str()),
        Some("last")
    );
    assert_eq!(query.first_loader().map(|l| l.name.as_str()), Some("first"));
    assert_eq!(query.tool.as_deref(), Some("acme_busy"));
    match &query.view.kind {
        NodeKind::Viz { config, from, .. } => {
            assert_eq!(config.viz, Viz::Table);
            assert_eq!(from.as_deref(), Some("last"));
        }
        other => panic!("a query's view is a table over its last loader: {other:?}"),
    }
    // A page's default loader is its first.
    let page = page(
        "title: T\nview: { viz: table, from: a }\nloaders:\n  a: { rows: [{ a: 1 }] }\n  b: { rows: [{ b: 1 }] }\n",
    )
    .unwrap();
    assert_eq!(page.kind, PageKind::Page);
    assert_eq!(page.file(), "pages/busy.yaml");
    assert_eq!(page.default_loader().map(|l| l.name.as_str()), Some("a"));
}

/// `hidden` is gone: a page with no `category` isn't listed, and a thing
/// never opened on its own is a part or a query. The error says so.
#[test]
fn hidden_is_gone_and_the_error_says_what_replaces_it() {
    let why = refused(&format!("title: T\nhidden: true\n{BODY}"));
    assert!(why.contains("`hidden` is no longer a key"), "{why}");
    assert!(
        why.contains("a page with no `category` isn't listed"),
        "{why}"
    );
    assert!(why.contains("a part (parts/)"), "{why}");
}

/// A page and a part show something, so each needs a `view:`.
#[test]
fn a_page_and_a_part_need_a_view() {
    for kind in [PageKind::Page, PageKind::Part] {
        let why =
            of_kind(kind, "title: T\nloaders:\n  q: { rows: [{ a: 1 }] }\n").expect_err("view");
        assert!(why.contains("needs a `view:`"), "{kind:?}: {why}");
    }
}

#[test]
fn a_resource_page_declares_the_uri_it_is_given() {
    let why = refused(&format!("title: T\nfor: oxplow/file\n{BODY}"));
    assert!(why.contains("declare `params: [{ name: uri }]`"), "{why}");
    let why = refused(&format!(
        "title: T\nfor: oxplow/file\nparams: [{{ name: path }}]\n{BODY}"
    ));
    assert!(why.contains("declare `params: [{ name: uri }]`"), "{why}");
}

#[test]
fn content_is_the_systems_name() {
    let why = refused(
        "title: T\nfor: oxplow/file\nparams: [{ name: uri }]\nview: { viz: table, from: content }\nloaders:\n  content: { rows: [{ a: 1 }] }\n",
    );
    assert!(
        why.contains("loader `content`") && why.contains("name it something else"),
        "{why}"
    );
    let why = refused(&format!("title: T\nparams: [{{ name: content }}]\n{BODY}"));
    assert!(why.contains("param `content`"), "{why}");
    let why = refused(&format!(
        "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}, {{ name: content }}]\n{BODY}"
    ));
    assert!(why.contains("param `content`"), "{why}");
}

#[test]
fn a_standalone_page_has_no_content() {
    let why = refused("title: T\ncontent: { text: true }\nview: { viz: table, from: q }\nloaders:\n  q: { rows: [{ a: 1 }] }\n");
    assert!(why.contains("this page has no `for:`"), "{why}");
    let why = refused("title: T\nloaders:\n  q: { sql: 'SELECT * FROM content' }\nview: { viz: table, from: q }\n");
    assert!(why.contains("a page with no `for:` has none"), "{why}");
    let why = refused(
        "title: T\nloaders:\n  q: { sql: 'SELECT 1 AS a' }\n  r: { sql: 'SELECT :a AS a', params: { a: { from: content.path } } }\nview: { viz: table, from: q }\n",
    );
    assert!(why.contains("a page with no `for:` has none"), "{why}");
    // A column that happens to be called `content` is not the loader.
    page("title: T\nloaders:\n  q: { sql: 'SELECT content FROM v_thing' }\nview: { viz: table, from: q }\n").unwrap();
}

#[test]
fn content_asks_for_a_sane_size() {
    for bad in ["0", "4194305"] {
        let why = refused(&format!(
            "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}]\ncontent: {{ max_bytes: {bad} }}\n{BODY}"
        ));
        assert!(why.contains("must be between 1 and"), "{why}");
    }
    page(&format!(
        "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}]\ncontent: {{ text: true, max_bytes: 4194304 }}\n{BODY}"
    ))
    .unwrap();
}

#[test]
fn a_tab_says_its_label_filled_from_params_and_its_row() {
    use super::label::tab_label;
    use oxplow_db::SqlCell;
    use std::collections::BTreeMap;
    let p = page(&format!(
        "title: Thing\nlabel: '{{{{param.path|basename}}}} {{{{row.n}}}}'\nparams: [{{ name: path }}]\n{BODY}"
    ))
    .unwrap();
    let params = BTreeMap::from([("path".to_string(), SqlCell::Text("src/a/b.rs".into()))]);
    let row = BTreeMap::from([("n".to_string(), SqlCell::Int(3))]);
    assert_eq!(tab_label(&p, &params, Some(&row)), "b.rs 3");
    // It names the row before there is one: the title.
    assert_eq!(tab_label(&p, &params, None), "Thing");
    let p = page(&format!(
        "title: Thing\nlabel: '{{{{param.sha|short}}}}'\nparams: [{{ name: sha }}]\n{BODY}"
    ))
    .unwrap();
    let params = BTreeMap::from([("sha".to_string(), SqlCell::Text("abcdef0123".into()))]);
    assert_eq!(tab_label(&p, &params, None), "abcdef0");
    assert_eq!(tab_label(&p, &BTreeMap::new(), None), "Thing");
    let p = page(&format!(
        "title: Web\nlabel: '{{{{param.u|url}}}}'\nparams: [{{ name: u }}]\n{BODY}"
    ))
    .unwrap();
    let params = BTreeMap::from([(
        "u".to_string(),
        SqlCell::Text("https://example.com/a/b".into()),
    )]);
    assert_eq!(tab_label(&p, &params, None), "example.com/a/b");
}

/// `accepts: { content: <loader> }` names one of the page's own loaders;
/// the page is offered only when that loader has a row. It is a key of
/// `accepts` that is not a URI param, so it is kept apart.
#[test]
fn accepts_content_names_a_loader_of_the_page() {
    let p = page(
        "title: T\nfor: oxplow/file\nparams: [{ name: uri }]\naccepts: { path: '^[a-z]+\\.yaml$', content: yes }\n\
         loaders:\n  yes: { sql: \"SELECT 1 AS n FROM content\" }\nview: { viz: table, from: yes }\n",
    )
    .unwrap();
    assert_eq!(p.accepts_content.as_deref(), Some("yes"));
    // The other `accepts` keys stay URI params.
    assert_eq!(p.accepts.keys().collect::<Vec<_>>(), ["path"]);
    // The system's own loader is one of the page's.
    let p = page(&format!(
        "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}]\naccepts: {{ content: content }}\n{BODY}"
    ))
    .unwrap();
    assert_eq!(p.accepts_content.as_deref(), Some("content"));
}

#[test]
fn accepts_content_refuses_what_cannot_be_asked_cheaply() {
    let with = |accepts: &str, loaders: &str| {
        format!(
            "title: T\nfor: oxplow/file\nparams: [{{ name: uri }}]\naccepts: {{ {accepts} }}\nloaders:\n{loaders}view: {{ viz: table, from: content }}\n"
        )
    };
    let why = refused(&with("content: nope", "  a: { rows: [{ n: 1 }] }\n"));
    assert!(
        why.contains("`accepts: content: nope`") && why.contains("isn't a loader"),
        "{why}"
    );
    let why = refused(&with(
        "content: borrowed",
        "  borrowed: { page: acme/other, loader: q }\n",
    ));
    assert!(why.contains("`page` loader"), "{why}");
    // The chain it reads is checked too.
    let why = refused(&with(
        "content: after",
        "  borrowed: { page: acme/other, loader: q }\n  after: { sql: \"SELECT * FROM borrowed\" }\n",
    ));
    assert!(
        why.contains("`page` loader") && why.contains("borrowed"),
        "{why}"
    );
    // A standalone page has no resource to look at.
    let why = refused(
        "title: T\naccepts: { content: q }\nloaders:\n  q: { rows: [{ n: 1 }] }\nview: { viz: table, from: q }\n",
    );
    assert!(
        why.contains("accepts: content") && why.contains("for:"),
        "{why}"
    );
}

/// A loader's dependency chain: itself and everything it reads, declared order.
#[test]
fn a_loaders_chain_is_what_it_reads_and_nothing_else() {
    let p = page(
        "title: T\nfor: oxplow/file\nparams: [{ name: uri }]\nloaders:\n  a: { sql: \"SELECT 1 AS n FROM content\" }\n  b: { sql: \"SELECT n FROM a\" }\n  c: { rows: [{ x: 1 }] }\n  d: { sql: \"SELECT n FROM b\" }\nview: { viz: table, from: d }\n",
    )
    .unwrap();
    assert_eq!(p.loader_chain("d"), ["content", "a", "b", "d"]);
    assert_eq!(p.loader_chain("c"), ["c"]);
}

/// A `sections` viz parses its role block beside `columns` and `empty`; it
/// is a row viz, so a whole-node action can't name it.
#[test]
fn a_sections_viz_parses_its_role_block() {
    let p = page(
        "title: T\nloaders:\n  q: { rows: [{ id: a, name: A }] }\nview:\n  viz: sections\n  from: q\n  sections: { title: name, id: id, link: uri, body: note }\n  columns: [{ key: kind }]\n  empty: Nothing here\n",
    )
    .expect("loads");
    let crate::pages::schema::NodeKind::Viz { config, .. } = &p.view.kind else {
        panic!("a viz")
    };
    assert_eq!(config.viz, Viz::Sections);
    let s = config.sections.clone().unwrap();
    assert_eq!(s.title.as_deref(), Some("name"));
    assert_eq!(s.id.as_deref(), Some("id"));
    assert_eq!(s.link.as_deref(), Some("uri"));
    assert_eq!(s.body.as_deref(), Some("note"));
    assert_eq!(config.empty.as_deref(), Some("Nothing here"));
    assert_eq!(
        config
            .role_columns()
            .iter()
            .map(|(b, k)| format!("{b}:{k}"))
            .collect::<Vec<_>>(),
        [
            "sections:name",
            "sections:id",
            "sections:uri",
            "sections:note"
        ]
    );
}

/// A `sections` viz with no `title` column is refused at load, naming the
/// block to write.
#[test]
fn a_sections_viz_without_a_title_column_is_refused() {
    let why = refused(
        "title: T\nloaders:\n  q: { rows: [{ a: 1 }] }\nview: { viz: sections, from: q }\n",
    );
    assert!(
        why.contains("viz `sections` needs `sections: { title: <column> }`"),
        "{why}"
    );
    let why = refused(
        "title: T\nloaders:\n  q: { rows: [{ a: 1 }] }\nview: { viz: sections, from: q, sections: { body: a } }\n",
    );
    assert!(why.contains("title: <column>"), "{why}");
}

/// A column's `format: items` says its value is a list of items; it loads on
/// a table, a list, a detail and a sections viz, and a made-up format is
/// refused.
#[test]
fn a_column_can_hold_a_list_of_items() {
    for viz in [
        "table",
        "list",
        "detail",
        "sections, sections: { title: a }",
    ] {
        let text = format!(
            "title: T\nloaders:\n  q: {{ rows: [{{ a: 1, p: '[]' }}] }}\nview: {{ viz: {viz}, from: q, columns: [{{ key: p, format: items }}] }}\n"
        );
        let ok = page(&text);
        assert!(ok.is_ok(), "{viz}: {ok:?}");
    }
    let why = refused(
        "title: T\nloaders:\n  q: { rows: [{ a: 1 }] }\nview: { viz: table, from: q, columns: [{ key: a, format: itemz }] }\n",
    );
    assert!(why.contains("itemz"), "{why}");
}

/// `sections` takes a `group` column and a `group_link` column (checked
/// against the loader's columns with the other role columns, at load).
#[test]
fn a_sections_viz_can_group_its_rows() {
    let p = page(
        "title: T\nloaders:\n  q: { rows: [{ name: A, ext: x, ext_uri: u }] }\nview: { viz: sections, from: q, sections: { title: name, group: ext, group_link: ext_uri } }\n",
    )
    .expect("loads");
    let crate::pages::schema::NodeKind::Viz { config, .. } = &p.view.kind else {
        panic!("a viz")
    };
    let s = config.sections.clone().unwrap();
    assert_eq!(s.group.as_deref(), Some("ext"));
    assert_eq!(s.group_link.as_deref(), Some("ext_uri"));
}

/// `hide_empty` and `empty:` contradict each other: one says to leave the
/// section out, the other what to say in it.
#[test]
fn hide_empty_with_an_empty_text_is_refused() {
    let err = page(
        "title: P\nloaders:\n  l: { sql: \"SELECT 1 AS n\" }\nview: { id: v, viz: list, from: l, hide_empty: true, empty: None. }\n",
    )
    .unwrap_err();
    assert!(err.contains("`hide_empty` leaves it out"), "{err}");
}

/// `fold: true` starts a viz's rows closed under their count.
#[test]
fn fold_is_read_into_the_viz_config() {
    let p = page("title: P\nloaders:\n  l: { sql: \"SELECT 1 AS n\" }\nview: { id: v, viz: list, from: l, fold: true }\n")
        .expect("loads");
    let crate::pages::schema::NodeKind::Viz { config, .. } = &p.view.kind else {
        panic!("a viz");
    };
    assert!(config.fold);
}

/// `see_also:` is a list of `{ page, params? }` and `{ from }` entries; an
/// entry that is neither, both, or names a loader or a param that is not
/// there is refused saying so.
#[test]
fn see_also_entries_parse_and_are_checked() {
    let ok = page(
        "title: T\nparams: [{ name: n }]\nloaders:\n  q: { rows: [{ title: A, uri: 'oxplow:oxplow/page?page=x' }] }\nview: { viz: table, from: q }\nsee_also:\n  - { page: other, params: { n: { param: n } } }\n  - { from: q }\n",
    )
    .expect("loads");
    assert_eq!(ok.see_also.len(), 2);
    assert!(matches!(&ok.see_also[0], SeeAlsoDecl::Page { page, .. } if page == "other"));
    assert!(matches!(&ok.see_also[1], SeeAlsoDecl::Rows { from } if from == "q"));
    let base = "title: T\nloaders:\n  q: { rows: [{ a: 1 }] }\nview: { viz: table, from: q }\n";
    for (entry, want) in [
        ("{ page: a, from: q }", "an entry is"),
        ("{}", "an entry is"),
        ("{ from: nope }", "no loader `nope`"),
        ("{ from: q, params: { n: 1 } }", "`params` go with `page:`"),
        (
            "{ page: a, params: { n: { param: ghost } } }",
            "isn't one of the page's params",
        ),
        (
            "{ page: a, params: { n: { from: ghost.col } } }",
            "no loader `ghost`",
        ),
    ] {
        let why = refused(&format!("{base}see_also:\n  - {entry}\n"));
        assert!(why.contains(want), "{entry}: {why}");
    }
}

/// `lead: <column>` on a viz node is kept; its column must be in the loader.
#[test]
fn a_viz_keeps_its_lead_column() {
    let p = page(
        "title: T\nloaders:\n  q: { rows: [{ d: x }] }\nview: { viz: detail, from: q, lead: d }\n",
    )
    .expect("loads");
    let crate::pages::schema::NodeKind::Viz { config, .. } = &p.view.kind else {
        panic!("a viz");
    };
    assert_eq!(config.lead.as_deref(), Some("d"));
    assert_eq!(
        config
            .role_columns()
            .iter()
            .filter(|(b, k)| *b == "lead" && *k == "d")
            .count(),
        1
    );
}

// --- the parse loader ---------------------------------------------------

/// `parse` turns one text into rows: a CSV or TSV as a table, a JSON or
/// YAML document as a tree. It reads the text as an arg does, so it reads
/// the loader it names.
#[test]
fn a_parse_loader_takes_a_format_and_a_text_source() {
    let p = parse_page(
        "acme",
        "acme",
        "sheet",
        "extensions/acme/pages/sheet.yaml",
        "title: T\nfor: oxplow/file\ncontent: { text: true }\nparams: [{ name: uri }]\nloaders:\n  rows: { parse: csv, text: { from: content.text } }\nview: { viz: table, from: rows }\n",
        PageKind::Page,
    )
    .unwrap();
    let decl = p.loader("rows").unwrap();
    assert_eq!(
        decl.loader,
        Loader::Parse {
            format: ParseFormat::Csv,
            text: ArgSource::Column {
                from: "content.text".into()
            },
        }
    );
    assert_eq!(decl.reads_loaders, ["content"]);
    assert_eq!(p.loader_chain("rows"), ["content", "rows"]);
    for format in ["tsv", "json", "yaml"] {
        page(&format!(
            "title: T\nloaders:\n  rows: {{ parse: {format}, text: \"a\" }}\nview: {{ viz: table, from: rows }}\n"
        ))
        .unwrap_or_else(|e| panic!("{format}: {e}"));
    }
}

#[test]
fn a_parse_loader_needs_a_known_format_and_a_text() {
    let e = refused(
        "title: T\nloaders:\n  rows: { parse: xml, text: \"a\" }\nview: { viz: table, from: rows }\n",
    );
    assert!(
        e.contains("loader `rows`")
            && e.contains("`parse: xml`")
            && e.contains("csv, tsv, json or yaml"),
        "{e}"
    );
    let e =
        refused("title: T\nloaders:\n  rows: { parse: csv }\nview: { viz: table, from: rows }\n");
    assert!(e.contains("loader `rows`") && e.contains("`text`"), "{e}");
    let e = refused(
        "title: T\nloaders:\n  rows: { parse: csv, text: \"a\", sql: SELECT 1 }\nview: { viz: table, from: rows }\n",
    );
    assert!(e.contains("exactly one of") && e.contains("`parse`"), "{e}");
}

/// `markdown: { text, base }`: `base` is a column holding the URI the text
/// came from (a file's), so the desktop resolves the text's relative links
/// and images against that file's folder. Both are role columns the dry
/// run checks.
#[test]
fn a_markdown_viz_may_name_the_file_its_text_came_from() {
    let p = page(
        "title: T\nloaders:\n  d: { sql: \"SELECT 'hi' AS body, 'oxplow:oxplow/file?path=README.md' AS u\" }\nview: { viz: markdown, from: d, markdown: { text: body, base: u } }\n",
    )
    .expect("loads");
    let crate::pages::schema::NodeKind::Viz { config, .. } = &p.view.kind else {
        panic!("a viz")
    };
    let m = config.markdown.clone().unwrap();
    assert_eq!(m.text.as_deref(), Some("body"));
    assert_eq!(m.base.as_deref(), Some("u"));
    assert_eq!(
        config
            .role_columns()
            .iter()
            .map(|(b, k)| format!("{b}:{k}"))
            .collect::<Vec<_>>(),
        ["markdown:body", "markdown:u"]
    );
}
