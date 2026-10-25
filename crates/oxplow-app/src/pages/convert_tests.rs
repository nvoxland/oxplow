//! The lens-to-page converter: each mapping rule on a small extension,
//! idempotency, and oxplow's own two extensions converted in memory.

use std::path::Path;

use super::convert::*;
use super::schema::{parse_page, ArgSource, Loader, NodeKind, Page};

fn src(manifest: &str, lenses: &[(&str, &str)]) -> Source {
    Source {
        manifest: manifest.to_string(),
        lenses: lenses
            .iter()
            .map(|(s, t)| (s.to_string(), t.to_string()))
            .collect(),
    }
}

const ACME: &str = "name: acme\nnamespace: acme\n";

fn run(manifest: &str, lenses: &[(&str, &str)]) -> Conversion {
    convert(&src(manifest, lenses), &[]).unwrap()
}

fn text<'a>(c: &'a Conversion, slug: &str) -> &'a str {
    let rel = Path::new("pages").join(format!("{slug}.yaml"));
    c.files
        .iter()
        .find(|(p, _)| *p == rel)
        .map(|(_, t)| t.as_str())
        .unwrap_or_else(|| {
            panic!(
                "no pages/{slug}.yaml among {:?}",
                c.files.iter().map(|(p, _)| p).collect::<Vec<_>>()
            )
        })
}

fn has(c: &Conversion, slug: &str) -> bool {
    let rel = Path::new("pages").join(format!("{slug}.yaml"));
    c.files.iter().any(|(p, _)| *p == rel)
}

fn page(c: &Conversion, slug: &str) -> Page {
    let t = text(c, slug);
    assert!(!t.contains("TODO(convert)"), "unexpected TODO:\n{t}");
    parse_page("acme", "acme", slug, &format!("pages/{slug}.yaml"), t)
        .unwrap_or_else(|e| panic!("pages/{slug}.yaml: {e}\n{t}"))
}

fn ids(nodes: &[super::schema::Node]) -> Vec<&str> {
    nodes.iter().map(|n| n.id.as_str()).collect()
}

#[test]
fn an_entry_page_takes_its_metadata_from_the_entry_and_its_body_from_the_lens() {
    let manifest = format!(
        "{ACME}pages:\n  - {{ id: open, title: Open, icon: inbox, category: Work, keywords: k, \
         params: [id], lens: open-items, count: open-count }}\n"
    );
    let c = run(
        &manifest,
        &[
            (
                "open-items",
                "title: Open items\ndescription: What is open.\nparams:\n  - { name: id }\n  - { name: q, role: filter, default: \"\" }\nquery: SELECT id FROM v_work_item WHERE id = :id\ntool: open_items\nactions:\n  - { id: add, label: Add, command: acme.item.add, input: {} }\n  - { id: del, label: Delete, command: acme.item.delete, row: true, input: { id: \"{{row.id}}\" } }\n",
            ),
            (
                "open-count",
                "title: Count\nquery: SELECT count(*) AS n FROM v_work_item\nviz: number\n",
            ),
        ],
    );
    let p = page(&c, "open");
    assert_eq!(p.title, "Open");
    assert_eq!(p.description, "What is open.");
    assert_eq!(p.icon.as_deref(), Some("inbox"));
    assert_eq!(p.keywords.as_deref(), Some("k"));
    assert!(p.category.is_some());
    assert_eq!(p.tool.as_deref(), Some("acme_open_items"));
    let names: Vec<&str> = p.params.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["id", "q"]);
    let loaders: Vec<&str> = p.loaders.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(loaders, ["open_items", "count"]);
    assert_eq!(p.badge.as_ref().unwrap().loader, "count");
    assert_eq!(
        p.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["add"]
    );
    match &p.view.kind {
        NodeKind::Viz { from, lens } => {
            assert_eq!(from.as_deref(), Some("open_items"));
            assert_eq!(lens.actions.len(), 1);
            assert_eq!(lens.actions[0].id, "del");
        }
        other => panic!("{other:?}"),
    }
    // The page's lens and its count-only lens become no pages of their own.
    assert!(!has(&c, "open-items"));
    assert!(!has(&c, "open-count"));
    assert_eq!(c.files.len(), 1);
}

fn item_ext(extra_manifest: &str, files_lens: &str) -> Conversion {
    let manifest = format!(
        "{ACME}pages:\n  - {{ id: item, title: Item, label: \"{{{{row.title}}}}\", for: acme/item, default: true, lens: item }}\n{extra_manifest}"
    );
    run(
        &manifest,
        &[
            (
                "item",
                "title: Item\nparams:\n  - { name: id }\nquery: SELECT id AS change_id, 'x' AS title FROM v_change WHERE id = :id\ncolumns: [{ key: change_id }, { key: title }]\nviz: document\nsections:\n  - { id: detail, lens: item-detail }\n  - { id: notes, markdown: \"Some notes\" }\n  - { id: files, title: Changed, lens: item-files }\n",
            ),
            (
                "item-detail",
                "title: Detail\nparams:\n  - { name: id }\n  - { name: change_id }\nquery: SELECT * FROM v_change WHERE id = :id AND change_id = :change_id\nviz: detail\ncolumns: [{ key: id }]\n",
            ),
            ("item-files", files_lens),
        ],
    )
}

const FILES: &str = "title: Files\nparams:\n  - { name: id }\nquery: SELECT path FROM v_change_file WHERE change = :id\n";

#[test]
fn a_document_inlines_a_part_used_once_and_embeds_a_shared_one() {
    let panel = "panels:\n  - { id: f, title: F, scope: thread, body: item-files }\n";
    let c = item_ext(panel, FILES);
    let p = page(&c, "item");
    assert_eq!(p.label.as_deref(), Some("{{row.title}}"));
    let NodeKind::Stack { children } = &p.view.kind else {
        panic!("{:?}", p.view)
    };
    assert_eq!(ids(children), ["detail", "notes", "files"]);
    assert!(matches!(&children[0].kind, NodeKind::Viz { from: Some(f), .. } if f == "detail"));
    assert!(matches!(&children[1].kind, NodeKind::Markdown { text } if text == "Some notes"));
    assert!(matches!(&children[2].kind, NodeKind::Embed { page, .. } if page == "acme/item-files"));
    assert_eq!(children[2].title.as_deref(), Some("Changed"));
    let loaders: Vec<&str> = p.loaders.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(loaders, ["row", "detail"]);
    assert_eq!(p.loaders[0].columns.len(), 2);
    let files = page(&c, "item-files");
    assert!(files.hidden);
    assert!(c.report.inlined.iter().any(|l| l.contains("item-detail")));
    assert!(c.report.embedded.iter().any(|l| l.contains("item-files")));
}

#[test]
fn a_part_param_the_document_row_holds_binds_from_the_row_loader() {
    let c = item_ext("", FILES);
    let p = page(&c, "item");
    let detail = p.loader("detail").unwrap();
    let Loader::Sql { params, .. } = &detail.loader else {
        panic!("{detail:?}")
    };
    assert_eq!(
        params.get("change_id"),
        Some(&ArgSource::Column {
            from: "row.change_id".into()
        })
    );
    assert!(!p.params.iter().any(|x| x.name == "change_id"));
    // Used once and inlined too.
    assert!(!has(&c, "item-files"));
}

#[test]
fn a_part_that_names_a_row_param_outside_its_query_is_embedded() {
    let files = "title: Files\nparams:\n  - { name: id }\n  - { name: change_id }\nquery: SELECT path FROM v_change_file WHERE change = :id\nactions:\n  - { id: open, label: Open, command: acme.change.open, input: { change: \"{{param.change_id}}\" } }\n";
    let c = item_ext("", files);
    let p = page(&c, "item");
    let NodeKind::Stack { children } = &p.view.kind else {
        panic!()
    };
    assert!(matches!(&children[2].kind, NodeKind::Embed { .. }));
    assert!(page(&c, "item-files").hidden);
}

#[test]
fn a_part_offered_as_a_tool_is_embedded() {
    let files = format!("{FILES}tool: item_files\n");
    let c = item_ext("", &files);
    let f = page(&c, "item-files");
    assert_eq!(f.tool.as_deref(), Some("acme_item_files"));
    assert!(f.hidden);
}

#[test]
fn a_view_document_becomes_a_switch_and_a_dashboard_a_grid() {
    let manifest = format!(
        "{ACME}pages:\n  - {{ id: settings, title: Settings, lens: settings }}\n  - {{ id: usage, title: Usage, lens: usage }}\n"
    );
    let c = run(
        &manifest,
        &[
            (
                "settings",
                "title: Settings\nparams:\n  - { name: area, default: one }\nviz: document\nview: area\nsections:\n  - { id: one, lens: s-one }\n  - { id: two, lens: s-two }\n",
            ),
            ("s-one", "title: One\nquery: SELECT 1 AS a\n"),
            ("s-two", "title: Two\nquery: SELECT 2 AS a\nviz: list\n"),
            (
                "usage",
                "title: Usage\nviz: dashboard\ntiles:\n  - { lens: u-total, size: small }\n  - { lens: u-days, size: wide }\n",
            ),
            ("u-total", "title: Total\nquery: SELECT 3 AS n\nviz: number\n"),
            ("u-days", "title: Days\nquery: SELECT 4 AS n\n"),
        ],
    );
    let s = page(&c, "settings");
    let NodeKind::Switch { param, children } = &s.view.kind else {
        panic!("{:?}", s.view)
    };
    assert_eq!(param, "area");
    assert_eq!(ids(children), ["one", "two"]);
    let u = page(&c, "usage");
    let NodeKind::Grid { children } = &u.view.kind else {
        panic!("{:?}", u.view)
    };
    assert_eq!(ids(children), ["u-total", "u-days"]);
    assert!(children.iter().all(|c| c.size.is_some()));
    assert_eq!(c.files.len(), 2);
}

#[test]
fn a_document_other_extensions_mount_into_ends_with_its_slot() {
    let other = src(
        "name: beta\nnamespace: beta\nui:\n  slots:\n    - { slot: acme/item.sections, lens: extra, at: { before: files } }\n",
        &[("extra", "title: Extra\nquery: SELECT 1 AS a\n")],
    );
    let manifest = format!(
        "{ACME}pages:\n  - {{ id: item, title: Item, lens: item }}\nui:\n  slots:\n    - {{ slot: acme/item.sections, lens: own }}\n"
    );
    let c = convert(
        &src(
            &manifest,
            &[
                (
                    "item",
                    "title: Item\nviz: document\nsections:\n  - { id: files, lens: item-files }\n",
                ),
                ("item-files", "title: Files\nquery: SELECT 1 AS a\n"),
                ("own", "title: Own\nquery: SELECT 1 AS a\n"),
            ],
        ),
        &[other],
    )
    .unwrap();
    let p = page(&c, "item");
    let NodeKind::Stack { children } = &p.view.kind else {
        panic!()
    };
    assert_eq!(ids(children), ["files", "sections"]);
    assert!(matches!(&children[1].kind, NodeKind::Slot { name } if name == "sections"));
    assert!(page(&c, "own").hidden);
    assert!(
        c.report
            .manifest_suggestions
            .iter()
            .any(|s| s.contains("ui.slots") && s.contains("\"page\":\"own\"")),
        "{:?}",
        c.report.manifest_suggestions
    );
}

#[test]
fn expand_and_options_from_targets_get_hidden_pages_and_name_them() {
    let c = run(
        ACME,
        &[
            (
                "a",
                "title: A\nlauncher: { category: Work }\nparams:\n  - { name: pick, options_from: a-choices }\nquery: SELECT 1 AS id\ncolumns:\n  - { key: id, expand: a-detail }\n",
            ),
            (
                "a-choices",
                "title: Choices\nquery: SELECT 1 AS value, 'x' AS label\n",
            ),
            (
                "a-detail",
                "title: Detail\nparams:\n  - { name: id }\nquery: SELECT :id AS id\n",
            ),
        ],
    );
    let a = page(&c, "a");
    assert!(!a.hidden);
    assert!(a.category.is_some());
    assert_eq!(a.params[0].options_from.as_deref(), Some("acme/a-choices"));
    let NodeKind::Viz { lens, .. } = &a.view.kind else {
        panic!()
    };
    assert_eq!(lens.columns[0].expand.as_deref(), Some("acme/a-detail"));
    assert!(page(&c, "a-choices").hidden);
    assert!(page(&c, "a-detail").hidden);
}

#[test]
fn the_lenses_the_migration_deletes_are_skipped_and_listed() {
    let manifest = format!(
        "{ACME}pages:\n  - {{ id: lens, title: Lens, for: oxplow/lens, component: \"oxplow:lens\" }}\n  - {{ id: resource, title: Resource, component: \"oxplow:resource\" }}\n  - {{ id: answer, title: Answer, lens: answer }}\npanels:\n  - {{ id: answers, title: Answers, scope: thread, body: thread-answers }}\n"
    );
    let c = run(
        &manifest,
        &[
            (
                "answer",
                "title: Answer\nviz: document\nsections:\n  - { lens: answer-body }\n",
            ),
            (
                "answer-body",
                "title: Body\nviz: answer\nquery: SELECT 1 AS a\n",
            ),
            ("thread-answers", "title: Answers\nquery: SELECT 1 AS a\n"),
            ("thread-notes", "title: Notes\nquery: SELECT 1 AS a\n"),
            ("dashboard", "title: D\nquery: SELECT 1 AS a\n"),
            ("only-for-dashboard", "title: O\nquery: SELECT 1 AS a\n"),
            (
                "dashboards",
                "title: Ds\nviz: document\nsections:\n  - { lens: only-for-dashboard }\n",
            ),
        ],
    );
    assert!(c.files.is_empty(), "{:?}", c.files);
    for s in [
        "lens",
        "resource",
        "answer",
        "answer-body",
        "thread-answers",
        "thread-notes",
        "dashboard",
        "dashboards",
        "only-for-dashboard",
    ] {
        assert!(
            c.report
                .skipped
                .iter()
                .any(|l| l.starts_with(&format!("{s} ")) || l.starts_with(&format!("{s}:"))),
            "{s} not in {:?}",
            c.report.skipped
        );
    }
}

#[test]
fn parts_whose_params_collide_are_written_with_a_todo() {
    let manifest = format!("{ACME}pages:\n  - {{ id: d, title: D, lens: d }}\n");
    let c = run(
        &manifest,
        &[
            (
                "d",
                "title: D\nviz: document\nsections:\n  - { lens: p1 }\n  - { lens: p2 }\n",
            ),
            (
                "p1",
                "title: P1\nparams:\n  - { name: limit, default: 10 }\nquery: SELECT 1 AS a LIMIT :limit\n",
            ),
            (
                "p2",
                "title: P2\nparams:\n  - { name: limit, label: Rows, default: 20 }\nquery: SELECT 2 AS a LIMIT :limit\n",
            ),
        ],
    );
    let t = text(&c, "d");
    assert!(t.starts_with("# TODO(convert): "), "{t}");
    assert!(t.contains("limit"));
    assert!(c.report.todos.iter().any(|l| l.contains("pages/d.yaml")));
}

#[test]
fn a_form_with_no_query_shows_no_loader() {
    let manifest = format!("{ACME}pages:\n  - {{ id: new, title: New, lens: new-thing }}\n");
    let c = run(
        &manifest,
        &[(
            "new-thing",
            "title: New\nviz: form\nform: { command: acme.thing.create }\n",
        )],
    );
    let p = page(&c, "new");
    assert!(p.loaders.is_empty());
    assert!(matches!(&p.view.kind, NodeKind::Viz { from: None, .. }));
}

#[test]
fn a_whole_action_reading_its_one_row_stays_on_the_detail_node() {
    let manifest = format!("{ACME}pages:\n  - {{ id: one, title: One, lens: one }}\n");
    let c = run(
        &manifest,
        &[(
            "one",
            "title: One\nquery: SELECT 1 AS id\nviz: detail\ncolumns: [{ key: id }]\nactions:\n  - { id: go, label: Go, command: acme.thing.go, input: { id: \"{{row.id}}\" } }\n",
        )],
    );
    let p = page(&c, "one");
    assert!(p.actions.is_empty());
    let NodeKind::Viz { lens, .. } = &p.view.kind else {
        panic!()
    };
    assert_eq!(lens.actions[0].id, "go");
}

#[test]
fn a_component_page_shows_its_component_with_its_rail() {
    let manifest = format!(
        "{ACME}custom_components:\n  - {{ id: explore, title: Explore, bundle: dist/explore }}\npages:\n  - {{ id: explore, title: Explore, params: [model], component: explore, rail: explore-rail }}\n"
    );
    let c = run(
        &manifest,
        &[(
            "explore-rail",
            "title: Rail\nparams:\n  - { name: model, default: \"\" }\nquery: SELECT 1 AS view\nviz: list\n",
        )],
    );
    let p = page(&c, "explore");
    assert!(
        matches!(&p.view.kind, NodeKind::Component { component, .. } if component == "explore")
    );
    assert!(matches!(
        &p.rail.as_ref().unwrap().kind,
        NodeKind::Viz { .. }
    ));
    assert_eq!(c.files.len(), 1);
}

#[test]
fn converting_twice_writes_the_same_files() {
    let panel = "panels:\n  - { id: f, title: F, scope: thread, body: item-files }\n";
    let a = item_ext(panel, FILES);
    let b = item_ext(panel, FILES);
    assert_eq!(a.files, b.files);
}

/// oxplow's own extensions convert: every file without a TODO parses, and
/// few need a hand.
#[test]
fn oxplow_s_own_extensions_convert() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions");
    for name in ["oxplow-foundation", "oxplow-bundled"] {
        let c = convert_dir(&root.join(name)).unwrap();
        let mut todo = 0;
        for (rel, t) in &c.files {
            if t.contains("# TODO(convert)") {
                todo += 1;
                continue;
            }
            let slug = rel.file_stem().unwrap().to_str().unwrap();
            if let Err(e) = parse_page(name, "oxplow", slug, &rel.to_string_lossy(), t) {
                panic!("{name}/{}: {e}\n{t}", rel.display());
            }
        }
        println!(
            "{name}: {} files, {todo} TODO, {} inlined, {} embedded, {} skipped, {} suggestions",
            c.files.len(),
            c.report.inlined.len(),
            c.report.embedded.len(),
            c.report.skipped.len(),
            c.report.manifest_suggestions.len()
        );
        for t in &c.report.todos {
            println!("  TODO {t}");
        }
        assert!(!c.files.is_empty());
        assert!(
            todo * 100 <= c.files.len() * 15,
            "{name}: {todo} of {} files need a hand",
            c.files.len()
        );
    }
}

#[test]
fn a_part_restating_part_of_a_param_is_the_same_param() {
    let manifest = format!("{ACME}pages:\n  - {{ id: d, title: D, lens: d }}\n");
    let c = run(
        &manifest,
        &[
            (
                "d",
                "title: D\nparams:\n  - { name: pick, type: choice, role: filter, default: a, options: [{ value: a, label: A }] }\nviz: document\nsections:\n  - { lens: p1 }\n",
            ),
            (
                "p1",
                "title: P1\nparams:\n  - { name: pick, default: a }\nquery: SELECT :pick AS a\n",
            ),
        ],
    );
    let p = page(&c, "d");
    assert_eq!(p.params.len(), 1);
    assert_eq!(p.params[0].options.len(), 1);
}

#[test]
fn a_document_action_reading_its_row_is_a_page_action_over_the_first_loader() {
    let manifest = format!("{ACME}pages:\n  - {{ id: s, title: S, lens: s }}\n");
    let c = run(
        &manifest,
        &[
            (
                "s",
                "title: S\nparams:\n  - { name: id }\nquery: SELECT :id AS session, 1 AS can_stop\ncolumns: [{ key: session }, { key: can_stop }]\nviz: document\nsections:\n  - { lens: s-part }\nactions:\n  - { id: stop, label: Stop, command: acme.session.stop, input: { session: \"{{row.session}}\" }, when: can_stop }\n",
            ),
            ("s-part", "title: Part\nquery: SELECT 1 AS a\n"),
        ],
    );
    let p = page(&c, "s");
    assert_eq!(p.loaders[0].name, "row");
    assert_eq!(
        p.actions.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(),
        ["stop"]
    );
}
