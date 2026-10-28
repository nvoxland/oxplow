//! The datasets that restate what the catalog and the page-ref graph hold, so
//! an agent reads them with `oxplow_query_sql` instead of a tool: `v_page`,
//! `v_extension`, `v_config_key` and `v_page_ref`.

use serde_json::{json, Value};

use crate::test_fixtures::{services_with_effort, EffortFixture};

const MANIFEST: &str =
    "manifest: 2\nname: acme\nversion: 2.1.0\nintent:\n  purpose: Count things.\n";

async fn rows(fx: &EffortFixture, sql: &str) -> Vec<Vec<Value>> {
    let out = fx.svc.sql.query_sql(sql, vec![], None).await.unwrap();
    serde_json::to_value(&out.rows)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_array().unwrap().clone())
        .collect()
}

fn write(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(".oxplow/extensions/acme").join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A project extension is a row once it activates, and the row follows
/// it away when it's deleted (its pages: `pages_are_datasets_that_follow_the_catalog`).
#[tokio::test]
async fn extensions_are_datasets_that_follow_the_catalog() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(&root, "extension.yaml", MANIFEST);
    crate::test_fixtures::activate(&fx.svc).await;

    assert_eq!(
        rows(
            &fx,
            "SELECT uri, name, version, origin, enabled, purpose, errors
             FROM v_extension WHERE name = 'acme'"
        )
        .await,
        vec![vec![
            json!("oxplow:oxplow/extension?name=acme"),
            json!("acme"),
            json!("2.1.0"),
            json!("project"),
            json!(1),
            json!("Count things."),
            json!("[]")
        ]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT origin FROM v_extension WHERE name = 'oxplow-bundled'"
        )
        .await,
        vec![vec![json!("bundled")]]
    );

    std::fs::remove_dir_all(root.join(".oxplow/extensions/acme")).unwrap();
    crate::extension::extension_deploy::remove(&fx.svc, "acme")
        .await
        .unwrap();
    assert!(rows(&fx, "SELECT 1 FROM v_extension WHERE name = 'acme'")
        .await
        .is_empty());
}

const PAGE: &str = "title: Acme PR\ndescription: One acme PR.\nparams:\n  - { name: number, label: PR }\n  - { name: view, options: [{ value: all, label: All }, { value: open, label: Open }], default: all }\nloaders:\n  a: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: a }\nanswers:\n  - { question: \"Which checks failed on this PR?\", params: { number: 12 } }\n";

/// A page file is a `v_page` row with its params and answers as rows, and
/// a disabled extension lists none.
#[tokio::test]
async fn pages_are_datasets_that_follow_the_catalog() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(&root, "extension.yaml", MANIFEST);
    write(&root, "pages/pr.yaml", PAGE);
    crate::test_fixtures::activate(&fx.svc).await;

    assert_eq!(
        rows(
            &fx,
            "SELECT uri, extension, id, title, description, hidden FROM v_page WHERE extension = 'acme'"
        )
        .await,
        vec![vec![
            json!("oxplow:oxplow/page?page=acme/pr"),
            json!("acme"),
            json!("acme/pr"),
            json!("Acme PR"),
            json!("One acme PR."),
            json!(0),
        ]]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT position, name, type, role, label, \"default\" FROM v_page_param
             WHERE page_uri = 'oxplow:oxplow/page?page=acme/pr' ORDER BY position"
        )
        .await,
        vec![
            vec![
                json!(1),
                json!("number"),
                json!("text"),
                json!("resource"),
                json!("PR"),
                Value::Null
            ],
            vec![
                json!(2),
                json!("view"),
                json!("choice"),
                json!("resource"),
                Value::Null,
                json!("\"all\"")
            ],
        ]
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT position, question, about, params FROM v_page_answer
             WHERE page_uri = 'oxplow:oxplow/page?page=acme/pr'"
        )
        .await,
        vec![vec![
            json!(1),
            json!("Which checks failed on this PR?"),
            Value::Null,
            json!(r#"{"number":12}"#)
        ]]
    );

    crate::extension::extension_deploy::set_enabled(&fx.svc, "acme", false, true)
        .await
        .unwrap();
    for dataset in ["v_page", "v_page_answer", "v_page_param"] {
        assert!(
            rows(
                &fx,
                &format!(
                    "SELECT 1 FROM {dataset} WHERE {} LIKE '%acme/pr'",
                    if dataset == "v_page" {
                        "uri"
                    } else {
                        "page_uri"
                    }
                )
            )
            .await
            .is_empty(),
            "{dataset}"
        );
    }
}

/// What a loaded extension warns about is on its row; one that fails to
/// load blocks the generation, so it is not a row (`oxplow_validate_extension`
/// says why).
#[tokio::test]
async fn an_extensions_warnings_are_on_its_row() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(&root, "extension.yaml", MANIFEST);
    crate::test_fixtures::activate(&fx.svc).await;
    assert_eq!(
        rows(
            &fx,
            "SELECT json_array_length(warnings) > 0 FROM v_extension WHERE name = 'acme'"
        )
        .await,
        vec![vec![json!(1)]]
    );

    write(
        &root,
        "pages/bad.yaml",
        "title: Bad\nloaders:\n  a: { sql: SELECT 1 }\nview: { viz: nonsense, from: a }\n",
    );
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(
        rows(&fx, "SELECT 1 FROM v_page WHERE uri LIKE '%acme/bad%'")
            .await
            .is_empty(),
        "a page that doesn't load is no row"
    );
}

/// `v_page_ref` is the edge list the backlinks tools returned, both ways.
#[tokio::test]
async fn page_ref_edges_are_a_dataset_in_both_directions() {
    let fx = services_with_effort().await;
    fx.svc
        .db
        .transaction(|tx| {
            tx.execute(
                "INSERT INTO page_ref (source_uri, target_uri, source_resource, target_resource, ref_type)
                 VALUES (uri('oxplow/wiki', 'slug', 'intro'), uri('oxplow/file', 'path', 'src/app.rs'),
                         'oxplow/wiki', 'oxplow/file', 'mention'),
                        (uri('oxplow/wiki', 'slug', 'intro'), uri('oxplow/wiki', 'slug', 'other'),
                         'oxplow/wiki', 'oxplow/wiki', 'link'),
                        (uri('oxplow/commit', 'sha', 'abc1234'), uri('oxplow/file', 'path', 'src/app.rs'),
                         'oxplow/commit', 'oxplow/file', 'touches')",
                [],
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
    // What points at a file.
    assert_eq!(
        rows(
            &fx,
            "SELECT source_uri, ref_type FROM v_page_ref WHERE target_uri = 'oxplow:oxplow/file?path=src/app.rs' ORDER BY source_uri"
        )
        .await,
        vec![
            vec![json!("oxplow:oxplow/commit?sha=abc1234"), json!("touches")],
            vec![json!("oxplow:oxplow/wiki?slug=intro"), json!("mention")],
        ]
    );
    // What a page points at.
    assert_eq!(
        rows(
            &fx,
            "SELECT target_uri FROM v_page_ref WHERE source_uri = 'oxplow:oxplow/wiki?slug=intro' ORDER BY target_uri"
        )
        .await,
        vec![vec![json!("oxplow:oxplow/file?path=src/app.rs")], vec![json!("oxplow:oxplow/wiki?slug=other")]]
    );
}

/// What a person can ask and what they can configure are datasets too:
/// what every enabled extension's pages answer in `v_page_answer`,
/// project.yaml's keys in `v_config_key`.
#[tokio::test]
async fn questions_and_config_keys_are_datasets() {
    let fx = services_with_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        "extension.yaml",
        "manifest: 2\nname: acme\nintent:\n  purpose: Count things.\n",
    );
    write(
        &root,
        "pages/changed.yaml",
        "title: Changed\nanswers:\n  - { question: Which acme things changed?, about: oxplow/file }\nloaders:\n  a: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: a }\n",
    );
    crate::test_fixtures::activate(&fx.svc).await;

    assert_eq!(
        rows(
            &fx,
            "SELECT p.extension, a.question, a.about FROM v_page_answer a JOIN v_page p ON p.uri = a.page_uri WHERE p.extension = 'acme'"
        )
        .await,
        vec![vec![
            json!("acme"),
            json!("Which acme things changed?"),
            json!("oxplow/file")
        ]]
    );
    // What oxplow's own pages say they answer is offered as foundation's.
    let ours = rows(
        &fx,
        "SELECT DISTINCT p.extension FROM v_page_answer a JOIN v_page p ON p.uri = a.page_uri WHERE p.extension = 'oxplow-foundation'",
    )
    .await;
    assert_eq!(ours, vec![vec![json!("oxplow-foundation")]]);

    let keys = rows(&fx, "SELECT key, doc <> '' FROM v_config_key ORDER BY key").await;
    let mut want: Vec<String> = oxplow_config::keys::config_keys()
        .into_iter()
        .map(|k| k.key)
        .collect();
    want.sort();
    assert_eq!(
        keys,
        want.into_iter()
            .map(|k| vec![json!(k), json!(1)])
            .collect::<Vec<_>>()
    );

    std::fs::remove_dir_all(root.join(".oxplow/extensions/acme")).unwrap();
    crate::extension::extension_deploy::remove(&fx.svc, "acme")
        .await
        .unwrap();
    assert!(rows(
        &fx,
        "SELECT 1 FROM v_page_answer a JOIN v_page p ON p.uri = a.page_uri WHERE p.extension = 'acme'"
    )
    .await
    .is_empty());
}
