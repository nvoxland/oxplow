//! Scoped grants are enforced, through real extensions on disk and the
//! services that run them (`.context/extensions.md` "Services" and
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

const HUMAN: &str = "invokers: { human: true, agent: false, lens: false }";

/// A command `read.<name>` whose script is `<name>.star`.
fn reader(name: &str, needs: &str) -> String {
    format!(
        "  - name: read.{name}\n    summary: Reads.\n    input_schema: {{ type: object }}\n    entry: {name}.star\n    needs: [{needs}]\n    {HUMAN}\n"
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
async fn a_qualified_read_reaches_only_its_model_and_what_derives_from_it() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    let commands = [
        (
            "items",
            "sql.read:v_work_item",
            "SELECT ref FROM v_work_item",
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
            "SELECT w.ref FROM v_work_item w, v_effort e",
        ),
        (
            "ownlineage",
            "sql.read:v_acme_mine",
            "SELECT ref FROM v_acme_mine",
        ),
        (
            "both",
            "sql.read:v_acme_mine, sql.read:v_work_item",
            "SELECT ref FROM v_acme_mine",
        ),
        ("anyderived", "sql.read", "SELECT id FROM v_acme_efforts"),
    ];
    let block: String = commands
        .iter()
        .map(|(name, needs, _)| reader(name, needs))
        .collect();
    write(
        &root,
        "oxplow/extensions/acme/extension.yaml",
        &format!(
            "manifest: 2\nname: acme\nintent:\n  purpose: p\nmodels:\n  - name: efforts\n    version: 1\n    description: Efforts.\n    columns:\n      - {{ name: id, type: INTEGER, doc: Id. }}\n  - name: mine\n    version: 1\n    description: Items.\n    columns:\n      - {{ name: ref, type: TEXT, doc: Ref. }}\ncommands:\n{block}"
        ),
    );
    write(
        &root,
        "oxplow/extensions/acme/models/efforts.sql",
        "SELECT id FROM ref('effort')",
    );
    write(
        &root,
        "oxplow/extensions/acme/models/mine.sql",
        "SELECT ref FROM ref('work_item')",
    );
    for (name, _, sql) in commands {
        write(
            &root,
            &format!("oxplow/extensions/acme/{name}.star"),
            &script(sql),
        );
    }
    fx.svc
        .extension_activation
        .activate(fx.svc.clone())
        .await
        .expect("the project activates");
    crate::test_fixtures::activate(&fx.svc).await;
    let errors = fx
        .svc
        .extension_catalog
        .unpinned(&root, "a test reads declarations")
        .iter()
        .find(|e| e.name == "acme")
        .map(|e| e.errors.join("\n"))
        .unwrap();
    assert!(errors.is_empty(), "{errors}");

    assert_eq!(
        run(&fx, "acme.read.items").await.unwrap()["result"],
        json!(1)
    );
    // Refused, naming the model that isn't granted: a model it was not given,
    // a join with one, an extension model derived from one, and an extension
    // model it holds a grant for whose own inputs it does not.
    for (name, model) in [
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
            message.contains(&format!("model `{model}`: not declared")),
            "{name}: {message}"
        );
    }
    // A bare `sql.read` reads anything published, derived models included,
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
            "SELECT decisions FROM v_extension_run WHERE contribution = 'command:acme.read.efforts'",
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
            .any(|d| d["need"] == "model" && d["name"] == "v_effort" && d["decision"] == "denied"),
        "{denied}"
    );
}

async fn errors_of(fx: &TaskEffortFixture, extension: &str) -> String {
    let root = fx.svc.layout.project_dir.clone();
    fx.svc
        .extension_catalog
        .snapshot(&root)
        .extensions()
        .iter()
        .find(|e| e.name == extension)
        .map(|e| e.errors.join("\n"))
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_service_the_script_did_not_declare_is_refused() {
    let fx = services_with_task_effort().await;
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        "oxplow/extensions/acme/extension.yaml",
        &format!(
            "manifest: 2\nname: acme\nintent:\n  purpose: p\ncommands:\n{}{}",
            reader("peek", "sql.read"),
            reader("sneak", "sql.read"),
        ),
    );
    // Declares `sql.read` and calls `state.read`: the run refuses it.
    write(
        &root,
        "oxplow/extensions/acme/peek.star",
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"state.read\", {\"scope\": \"project\", \"area\": \"state\"})}\n",
    );
    write(
        &root,
        "oxplow/extensions/acme/sneak.star",
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

    // Fact collectors reaching for a model or the tree without declaring it.
    write(
        &root,
        "oxplow/extensions/facts/extension.yaml",
        "manifest: 2\nname: facts\nintent:\n  purpose: p\ncollectors:
  - id: asks
    runtime: starlark
    entry: asks.star
    needs: [sql.read]
    facts: [facts.tally]
",
    );
    write(
        &root,
        "oxplow/extensions/facts/asks.star",
        "def transform(x):\n    return {\"facts\": [{\"measure\": \"facts.tally\", \"value\": len(ai_summarize(\"t\"))}]}\n",
    );
    let errors = errors_of(&fx, "facts").await;
    assert!(
        errors.contains("uses `ai_summarize()` without `ai.compute`"),
        "{errors}"
    );
}

mod approved_code {
    use std::os::unix::fs::PermissionsExt;

    use oxplow_domain::run::{Mode, RunContext};

    use super::*;
    use crate::collector_runner::{
        approval_hash, approve, run_collector, Collectors, RunCollectorError, RunTrigger,
    };

    const MANIFEST: &str = "manifest: 2\nname: acme\nintent:\n  purpose: p\ncollectors:
  - id: sync
    runtime: exec
    entry: sync.sh
    entities:
      - { name: seen, key: k, columns: { k: text, v: text } }
";

    fn script(root: &Path, rel: &str, body: &str) {
        let path = root.join("oxplow/extensions/acme").join(rel);
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

    fn approve_current(fx: &TaskEffortFixture) {
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
        let spec = ext.collectors[0].clone();
        let hash = approval_hash(&root.join(&ext.path), &spec).unwrap();
        approve(&fx.svc.approvals, "acme", "sync", &hash).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_program_runs_only_once_approved_and_asks_again_when_edited() {
        let fx = services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        write(&root, "oxplow/extensions/acme/extension.yaml", MANIFEST);
        let marker = root.join("ran");
        let program = |entities: &str| {
            format!(
                "#!/bin/sh\ntouch {}\necho '{{\"entities\":{{\"seen\":{entities}}}}}'\n",
                marker.display()
            )
        };
        script(&root, "sync.sh", &program("[{\"k\":\"a\",\"v\":\"one\"}]"));

        let refused = go(&fx).await.unwrap_err();
        assert!(
            matches!(refused, RunCollectorError::NeedsApproval(_)),
            "{refused:?}"
        );
        assert!(!marker.exists(), "an unapproved program never starts");

        approve_current(&fx);
        go(&fx).await.unwrap();
        assert!(marker.exists());

        // Any edit to the program asks again, and the edited program doesn't run.
        std::fs::remove_file(&marker).unwrap();
        script(&root, "sync.sh", &program("[]"));
        let refused = go(&fx).await.unwrap_err();
        assert!(
            matches!(refused, RunCollectorError::NeedsApproval(_)),
            "{refused:?}"
        );
        assert!(
            !marker.exists(),
            "an edited program does not run unapproved"
        );

        // So does a helper added beside it, which the approval covers.
        approve_current(&fx);
        go(&fx).await.unwrap();
        script(&root, "helper.txt", "added later");
        let refused = go(&fx).await.unwrap_err();
        assert!(
            matches!(refused, RunCollectorError::NeedsApproval(_)),
            "{refused:?}"
        );
    }

    /// Disabling the extension revokes both its command and its approved
    /// program on their next call, with no restart.
    #[tokio::test(flavor = "multi_thread")]
    async fn disabling_the_extension_stops_its_next_command_and_collector_run() {
        let fx = services_with_task_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        write(
            &root,
            "oxplow/extensions/acme/extension.yaml",
            &format!("{MANIFEST}commands:\n{}", super::reader("all", "sql.read")),
        );
        write(
            &root,
            "oxplow/extensions/acme/all.star",
            &super::script("SELECT 1 AS n"),
        );
        script(
            &root,
            "sync.sh",
            "#!/bin/sh\necho '{\"entities\":{\"seen\":[]}}'\n",
        );
        fx.svc
            .extension_activation
            .activate(fx.svc.clone())
            .await
            .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        approve_current(&fx);
        assert!(super::run(&fx, "acme.read.all").await.is_ok());
        go(&fx).await.unwrap();

        write(
            &root,
            ".oxplow/project.yaml",
            "extensions:\n  disabled: [acme]\n",
        );
        fx.svc
            .extension_activation
            .activate(fx.svc.clone())
            .await
            .unwrap();
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

        // Enabling it again restores both; the approval was kept.
        std::fs::remove_file(root.join(".oxplow/project.yaml")).unwrap();
        fx.svc
            .extension_activation
            .activate(fx.svc.clone())
            .await
            .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        assert!(super::run(&fx, "acme.read.all").await.is_ok());
        go(&fx).await.unwrap();
    }

    /// The rows the collector published, read from its entity table.
    async fn seen(fx: &TaskEffortFixture) -> Vec<(String, String)> {
        fx.svc
            .db
            .read(|c| {
                // The table appears with the first write.
                let exists: bool = c
                    .query_row(
                        "SELECT count(*) > 0 FROM sqlite_master WHERE name = 'ext__acme__seen'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                if !exists {
                    return Ok(Vec::new());
                }
                let mut s = c
                    .prepare("SELECT k, v FROM ext__acme__seen ORDER BY k")
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = s
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await
            .unwrap()
    }

    /// A waiting program: reports where it ran, and what its helper said
    /// once the test released it.
    async fn waiting_program(fx: &TaskEffortFixture) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = fx.svc.layout.project_dir.clone();
        write(&root, "oxplow/extensions/acme/extension.yaml", MANIFEST);
        script(&root, "helper.txt", "approved");
        let started = root.join("started");
        let release = root.join("release");
        script(
            &root,
            "sync.sh",
            &format!(
                "#!/bin/sh\ntouch {started}\nwhile [ ! -f {release} ]; do sleep 0.05; done\nprintf '{{\"entities\":{{\"seen\":[{{\"k\":\"dir\",\"v\":\"%s\"}},{{\"k\":\"helper\",\"v\":\"%s\"}}]}}}}' \"$OXPLOW_EXTENSION_DIR\" \"$(cat helper.txt)\"\n",
                started = started.display(),
                release = release.display()
            ),
        );
        fx.svc
            .extension_activation
            .activate(fx.svc.clone())
            .await
            .expect("the project activates");
        approve_current(fx);
        (started, release)
    }

    async fn start(
        fx: &TaskEffortFixture,
        started: &Path,
    ) -> tokio::task::JoinHandle<
        Result<crate::collector_runner::CollectorRunReport, RunCollectorError>,
    > {
        let svc = fx.svc.clone();
        let root = fx.svc.layout.project_dir.clone();
        let run = tokio::spawn(async move {
            let collectors = Collectors::of(&svc, &root);
            run_collector(
                &collectors,
                &as_person(),
                "acme",
                "sync",
                RunTrigger::Manual,
            )
            .await
        });
        for _ in 0..200 {
            if started.exists() {
                return run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("the program started");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_program_runs_from_a_private_copy_removed_afterwards() {
        let fx = services_with_task_effort().await;
        let original = fx.svc.layout.project_dir.join("oxplow/extensions/acme");
        let (started, release) = waiting_program(&fx).await;
        let run = start(&fx, &started).await;
        std::fs::write(&release, "").unwrap();
        run.await.unwrap().unwrap();

        let rows = seen(&fx).await;
        let find = |key: &str| rows.iter().find(|(k, _)| k == key).unwrap().1.clone();
        assert_eq!(find("helper"), "approved");
        let dir = find("dir");
        assert_ne!(
            Path::new(&dir),
            original,
            "it ran beside a copy, not in the tree"
        );
        assert!(
            !Path::new(&dir).exists(),
            "the copy is removed after the run"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_during_the_run_stops_it_and_publishes_nothing() {
        let fx = services_with_task_effort().await;
        let original = fx.svc.layout.project_dir.join("oxplow/extensions/acme");
        let (started, release) = waiting_program(&fx).await;
        let run = start(&fx, &started).await;
        std::fs::write(original.join("helper.txt"), "EDITED").unwrap();
        std::fs::write(&release, "").unwrap();
        let error = run
            .await
            .unwrap()
            .expect_err("the edited program is not approved");
        let RunCollectorError::Failed(message) = &error else {
            panic!("{error:?}");
        };
        assert!(message.contains("changed during its run"), "{message}");
        assert!(seen(&fx).await.is_empty(), "nothing it produced was kept");
    }

    fn spec_of(
        fx: &TaskEffortFixture,
    ) -> (std::path::PathBuf, oxplow_config::collectors::CollectorSpec) {
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
        fx: &TaskEffortFixture,
        timeout: std::time::Duration,
    ) -> Result<crate::collector_runner::ScriptOutput, String> {
        let (dir, spec) = spec_of(fx);
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
            "oxplow/extensions/acme/extension.yaml",
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
        tokio::task::block_in_place(|| exec(&fx, std::time::Duration::from_secs(10))).unwrap();
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
        write(&root, "oxplow/extensions/acme/extension.yaml", MANIFEST);
        let pidfile = root.join("child.pid");
        script(
            &root,
            "sync.sh",
            &format!(
                "#!/bin/sh\nsleep 300 &\necho $! > {}\nwait\n",
                pidfile.display()
            ),
        );
        let error =
            tokio::task::block_in_place(|| exec(&fx, std::time::Duration::from_millis(600)))
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
