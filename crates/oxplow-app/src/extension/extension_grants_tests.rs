//! Scoped grants are enforced, through real extensions on disk and the
//! services that run them (`.context/extensions-permissions.md` "Services" and
//! "Grants"): qualified reads, undeclared services, approved code,
//! revocation and the decisions a run records.

use std::path::Path;

use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

use crate::test_fixtures::{services_with_task_effort, TaskEffortFixture};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// A command `read.<name>` whose script is `<name>.star`.
fn reader(name: &str, needs: &str) -> String {
    format!(
        "  - name: read.{name}\n    summary: Reads.\n    input_schema: {{ type: object }}\n    entry: {name}.star\n    needs: [{needs}]\n"
    )
}

fn script(sql: &str) -> String {
    format!(
        "def transform(x):\n    rows = scope(\"sql.read\", {{\"sql\": \"{sql}\"}})\n    return {{\"commands\": [], \"result\": len(rows)}}\n"
    )
}

async fn run(fx: &TaskEffortFixture, name: &str) -> Result<Value, CommandError> {
    fx.svc
        .commands
        .run(&Actor::Human, name, json!({}), false)
        .await
        .map(|outcome| outcome.result)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_qualified_read_reaches_only_its_dataset_and_what_derives_from_it() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    let commands = [
        (
            "items",
            "sql.read:v_work_item",
            "SELECT uri FROM v_work_item",
        ),
        ("efforts", "sql.read:v_work_item", "SELECT id FROM v_effort"),
        ("any", "sql.read", "SELECT id FROM v_effort"),
        (
            "derived",
            "sql.read:v_work_item",
            "SELECT id FROM v_acme_efforts",
        ),
        (
            "joined",
            "sql.read:v_work_item",
            "SELECT w.uri FROM v_work_item w, v_effort e",
        ),
        (
            "ownlineage",
            "sql.read:v_acme_mine",
            "SELECT uri FROM v_acme_mine",
        ),
        (
            "both",
            "sql.read:v_acme_mine, sql.read:v_work_item",
            "SELECT uri FROM v_acme_mine",
        ),
        ("anyderived", "sql.read", "SELECT id FROM v_acme_efforts"),
    ];
    let block: String = commands
        .iter()
        .map(|(name, needs, _)| reader(name, needs))
        .collect();
    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        &format!(
            "manifest: 2\nname: acme\nintent:\n  purpose: p\npermissions: [sql.read]\ndatasets:\n  - name: efforts\n    version: 1\n    description: Efforts.\n    fields:\n      - {{ name: id, type: INTEGER, doc: Id. }}\n  - name: mine\n    version: 1\n    description: Items.\n    fields:\n      - {{ name: uri, type: TEXT, doc: Ref. }}\ncommands:\n{block}"
        ),
    );
    write(
        &root,
        ".oxplow/extensions/acme/datasets/efforts.sql",
        "SELECT id FROM ref('effort')",
    );
    write(
        &root,
        ".oxplow/extensions/acme/datasets/mine.sql",
        "SELECT uri FROM ref('work_item')",
    );
    for (name, _, sql) in commands {
        write(
            &root,
            &format!(".oxplow/extensions/acme/{name}.star"),
            &script(sql),
        );
    }
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    crate::test_fixtures::activate(&fx.svc).await;
    let errors = fx
        .svc
        .extension_catalog
        .unpinned(&root, "a test reads declarations")
        .iter()
        .find(|e| e.name == "acme")
        .map(|e| {
            e.errors
                .iter()
                .map(|d| e.shown(d))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap();
    assert!(errors.is_empty(), "{errors}");

    assert_eq!(
        run(&fx, "acme.read.items").await.unwrap()["result"],
        json!(1)
    );
    // Refused, naming the dataset that isn't granted: a dataset it was not given,
    // a join with one, an extension dataset derived from one, and an extension
    // dataset it holds a grant for whose own inputs it does not.
    for (name, dataset) in [
        ("efforts", "v_effort"),
        ("joined", "v_effort"),
        ("derived", "v_effort"),
        ("ownlineage", "v_work_item"),
    ] {
        let error = run(&fx, &format!("acme.read.{name}"))
            .await
            .expect_err(name);
        let message = format!("{error:?}");
        assert!(
            message.contains(&format!("dataset `{dataset}`: not declared")),
            "{name}: {message}"
        );
    }
    // A bare `sql.read` reads anything published, derived datasets included,
    // and a grant for both ends of a lineage reads the derived one.
    for name in ["any", "both", "anyderived"] {
        assert!(
            run(&fx, &format!("acme.read.{name}")).await.is_ok(),
            "{name}"
        );
    }

    // The refusal is a recorded decision on the run, readable in SQL.
    let rows = fx
        .svc
        .sql
        .query_sql(
            "SELECT decisions FROM v_extension_run WHERE contribution = 'oxplow:oxplow/command?command=acme.read.efforts'",
            vec![],
            None,
        )
        .await
        .unwrap();
    let oxplow_db::SqlCell::Text(denied) = &rows.rows[0][0] else {
        panic!("{rows:?}");
    };
    let denied: Value = serde_json::from_str(denied).unwrap();
    assert!(
        denied
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["need"] == "dataset"
                && d["name"] == "v_effort"
                && d["decision"] == "denied"),
        "{denied}"
    );
}

async fn errors_of(fx: &TaskEffortFixture, extension: &str) -> String {
    crate::test_fixtures::load_errors(&fx.svc, extension).await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_the_script_did_not_declare_is_refused() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        &format!(
            "manifest: 2\nname: acme\nintent:\n  purpose: p\npermissions: [sql.read]\ncommands:\n{}{}",
            reader("peek", "sql.read"),
            reader("sneak", "sql.read"),
        ),
    );
    // Declares `sql.read` and calls `state.read`: the run refuses it.
    write(
        &root,
        ".oxplow/extensions/acme/peek.star",
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"state.read\", {\"scope\": \"project\", \"area\": \"state\"})}\n",
    );
    write(
        &root,
        ".oxplow/extensions/acme/sneak.star",
        &script("SELECT 1 AS n"),
    );
    crate::test_fixtures::activate(&fx.svc).await;

    // `state.read` is not among what the script declared: the run refuses it
    // and says which service and why.
    let refused = run(&fx, "acme.read.peek").await.expect_err("undeclared");
    let message = format!("{refused:?}");
    assert!(
        message.contains("`state.read` isn't in the command's `needs`"),
        "{message}"
    );
    // The declared service works for the same extension.
    assert!(run(&fx, "acme.read.sneak").await.is_ok());

    // Fact collectors reaching for a dataset or the tree without declaring it.
    write(
        &root,
        ".oxplow/extensions/facts/extension.yaml",
        "manifest: 2\nname: facts\nintent:\n  purpose: p\npermissions: [sql.read, ai.compute]\ncollectors:
  - id: asks
    runtime: starlark
    entry: asks.star
    needs: [sql.read]
    facts: [facts.tally]
",
    );
    write(
        &root,
        ".oxplow/extensions/facts/asks.star",
        "def transform(x):\n    return {\"facts\": [{\"measure\": \"facts.tally\", \"value\": len(ai_summarize(\"t\"))}]}\n",
    );
    let errors = errors_of(&fx, "facts").await;
    assert!(
        errors.contains("uses `ai_summarize()` without `ai.compute`"),
        "{errors}"
    );
}

mod program_collectors {
    use std::os::unix::fs::PermissionsExt;

    use oxplow_domain::run::{Mode, RunContext};

    use super::*;
    use crate::collector_runner::{run_collector, Collectors, RunCollectorError, RunTrigger};

    const MANIFEST: &str =
        "manifest: 2\nname: acme\nintent:\n  purpose: p\npermissions: [process.run]\ncollectors:
  - id: sync
    runtime: exec
    entry: sync.sh
    entities:
      - { name: seen, key: k, columns: { k: text, v: text } }
";

    fn script(root: &Path, rel: &str, body: &str) {
        let path = root.join(".oxplow/extensions/acme").join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn as_person() -> RunContext {
        RunContext::core(Actor::Human, Mode::External)
    }

    async fn go(
        fx: &TaskEffortFixture,
    ) -> Result<crate::collector_runner::CollectorRunReport, RunCollectorError> {
        let root = fx.svc.layout.project_dir.clone();
        // What was written (or edited) since is published first.
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let collectors = Collectors::of(&fx.svc, &root);
        run_collector(
            &collectors,
            &as_person(),
            "acme",
            "sync",
            RunTrigger::Manual,
        )
        .await
    }

    /// Disabling the extension revokes both its command and its program on
    /// their next call, with no restart.
    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_the_extension_stops_its_next_command_and_collector_run() {
        let fx = services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        write(
            &root,
            ".oxplow/extensions/acme/extension.yaml",
            &format!(
                "{}commands:\n{}",
                MANIFEST.replace("[process.run]", "[process.run, sql.read]"),
                super::reader("all", "sql.read")
            ),
        );
        write(
            &root,
            ".oxplow/extensions/acme/all.star",
            &super::script("SELECT 1 AS n"),
        );
        script(
            &root,
            "sync.sh",
            "#!/bin/sh\necho '{\"entities\":{\"seen\":[]}}'\n",
        );
        crate::test_fixtures::activate(&fx.svc).await;
        assert!(super::run(&fx, "acme.read.all").await.is_ok());
        go(&fx).await.unwrap();

        write(
            &root,
            ".oxplow/personal.yaml",
            "disableExtensions: [acme]\n",
        );
        crate::test_fixtures::activate(&fx.svc).await;
        assert!(
            super::run(&fx, "acme.read.all").await.is_err(),
            "its command is withdrawn"
        );
        let refused = go(&fx).await.unwrap_err();
        assert!(
            matches!(
                refused,
                RunCollectorError::NotFound | RunCollectorError::Disabled(_)
            ),
            "{refused:?}"
        );

        // Enabling it again restores both.
        std::fs::remove_file(root.join(".oxplow/personal.yaml")).unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        assert!(super::run(&fx, "acme.read.all").await.is_ok());
        go(&fx).await.unwrap();
    }

    async fn spec_of(
        fx: &TaskEffortFixture,
    ) -> (std::path::PathBuf, oxplow_config::collectors::CollectorSpec) {
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        let root = fx.svc.layout.project_dir.clone();
        let ext = fx
            .svc
            .extension_catalog
            .snapshot(&root)
            .extensions()
            .iter()
            .find(|e| e.name == "acme")
            .cloned()
            .unwrap();
        (root.join(&ext.path), ext.collectors[0].clone())
    }

    fn exec(
        (dir, spec): (std::path::PathBuf, oxplow_config::collectors::CollectorSpec),
        timeout: std::time::Duration,
    ) -> Result<crate::collector_runner::ScriptOutput, String> {
        let host_env = |name: &str| match name {
            "PATH" | "HOME" => std::env::var(name).ok(),
            "KEPT" => Some("declared".to_string()),
            "SECRET_TOKEN" => Some("hunter2".to_string()),
            _ => None,
        };
        crate::collector_runner::exec_collector(
            &dir,
            &spec,
            &host_env,
            &Default::default(),
            &as_person().within(oxplow_domain::run::Budget::with_timeout(timeout)),
            &crate::collector_runner::Egress::Open,
            &json!({}),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_program_gets_a_scrubbed_environment() {
        let fx = services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        write(
            &root,
            ".oxplow/extensions/acme/extension.yaml",
            &MANIFEST.replace(
                "    entry: sync.sh\n",
                "    entry: sync.sh\n    env: [KEPT]\n",
            ),
        );
        let dump = root.join("env.txt");
        script(
            &root,
            "sync.sh",
            &format!(
                "#!/bin/sh\nenv > {}\necho '{{\"entities\":{{\"seen\":[]}}}}'\n",
                dump.display()
            ),
        );
        let spec = spec_of(&fx).await;
        tokio::task::block_in_place(|| exec(spec, std::time::Duration::from_secs(10))).unwrap();
        let seen = std::fs::read_to_string(&dump).unwrap();
        let names: std::collections::BTreeSet<_> = seen
            .lines()
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_string()))
            .collect();
        assert!(
            names.contains("KEPT"),
            "a declared variable is passed: {names:?}"
        );
        assert!(
            !seen.contains("hunter2") && !names.contains("SECRET_TOKEN"),
            "{names:?}"
        );
        for expected in [
            "PATH",
            "HOME",
            "OXPLOW_EXTENSION_DIR",
            "OXPLOW_COLLECTOR_ID",
        ] {
            assert!(names.contains(expected), "{expected} is passed");
        }
        // Nothing of the host's own environment leaks through: only what the
        // run was given, plus what the shell itself adds.
        let allowed = [
            "PATH",
            "HOME",
            "KEPT",
            "OXPLOW_EXTENSION_DIR",
            "OXPLOW_COLLECTOR_ID",
            "PWD",
            "OLDPWD",
            "SHLVL",
            "_",
        ];
        let extra: Vec<_> = names
            .iter()
            .filter(|n| !allowed.contains(&n.as_str()))
            .collect();
        assert!(extra.is_empty(), "unexpected variables: {extra:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_timeout_reaps_the_program_and_what_it_started() {
        let fx = services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        write(&root, ".oxplow/extensions/acme/extension.yaml", MANIFEST);
        let pidfile = root.join("child.pid");
        script(
            &root,
            "sync.sh",
            &format!(
                "#!/bin/sh\nsleep 300 &\necho $! > {}\nwait\n",
                pidfile.display()
            ),
        );
        let spec = spec_of(&fx).await;
        let error =
            tokio::task::block_in_place(|| exec(spec, std::time::Duration::from_millis(600)))
                .expect_err("it never finishes");
        assert!(error.contains("timed out"), "{error}");

        let child: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        // SAFETY: signal 0 only probes whether the process exists.
        while unsafe { libc::kill(child, 0) } == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the program's child outlived the timeout"
            );
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}

/// A script composes commands of its own extension freely; one outside its
/// namespace needs a `commands:<pattern>` permission naming it — what it
/// may do with the person's rights is what was approved at deploy.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_composing_a_command_outside_its_namespace_needs_the_commands_permission() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    let item = oxplow_tasks::work_item_ref(fx.task);
    let manifest = |permissions: &str| {
        format!(
            "manifest: 2\nname: acme\nintent:\n  purpose: p\n{permissions}commands:\n  - name: notes.add\n    summary: Comments.\n    input_schema: {{ type: object }}\n    entry: note.star\n"
        )
    };
    write(
        &root,
        ".oxplow/extensions/acme/note.star",
        &format!(
            "def transform(x):\n    return {{\"commands\": [{{\"name\": \"oxplow.work_item.comment\", \"input\": {{\"uri\": \"{item}\", \"body\": \"noted\"}}}}]}}\n"
        ),
    );
    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        &manifest(""),
    );
    crate::test_fixtures::activate(&fx.svc).await;
    let refused = run(&fx, "acme.notes.add")
        .await
        .expect_err("outside its namespace");
    let message = format!("{refused:?}");
    assert!(
        message.contains("commands:oxplow.work_item.comment"),
        "{message}"
    );

    write(
        &root,
        ".oxplow/extensions/acme/extension.yaml",
        &manifest("permissions: [commands:oxplow.work_item.*]\n"),
    );
    crate::test_fixtures::activate(&fx.svc).await;
    run(&fx, "acme.notes.add")
        .await
        .expect("its pattern names it");
}
