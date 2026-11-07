//! Every example oxplow ships, one test each: they take seconds apiece, so they
//! run side by side and a failure names its example. Generated from
//! `examples::all()`, so a new example is tested with no list to update.

#![allow(clippy::unwrap_used)]

use std::io::Write as _;
use std::sync::mpsc;
use std::time::Duration;

use libtest_mimic::{Arguments, Trial};
use oxplow_app::extension::extension_catalog::ExtensionCatalog;
use oxplow_sdk::examples::Example;
use oxplow_sdk::extension_test::test_extension;
use oxplow_sdk::{check, render_findings, Format};

/// One example has hung these tests on CI, now and then, with nothing in the
/// log to say which. While this lives, an example that takes this long says
/// its name and what is running, and ends the test binary: CI's step limit
/// would end it anyway, saying nothing. Written straight to stderr, which a
/// test's output capture would otherwise hold back.
const HANG_LIMIT: Duration = Duration::from_secs(300);

struct HangWatch {
    _done: mpsc::Sender<()>,
}

impl HangWatch {
    fn on(example: &'static str) -> Self {
        let (done, wait) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            if wait.recv_timeout(HANG_LIMIT) != Err(mpsc::RecvTimeoutError::Timeout) {
                return;
            }
            let processes = std::process::Command::new("ps")
                .args(["-eo", "pid,ppid,stat,etime,args"])
                .output()
                .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
                .unwrap_or_default();
            let mut err = std::io::stderr();
            let _ = writeln!(
                err,
                "example `{example}` is still going after {HANG_LIMIT:?}; what is running:\n{processes}"
            );
            std::process::abort();
        });
        Self { _done: done }
    }
}

async fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    oxplow_app::vcs::GitProvider
        .init_repository(dir.path())
        .await
        .unwrap();
    dir
}

/// `extension new --from <example>` copies each example oxplow ships, and
/// the copy checks and tests clean as it stands (an example that builds on
/// another comes with it).
async fn scaffolds_checks_and_tests_clean(example: &'static Example) {
    let _watch = HangWatch::on(example.name);
    let dir = project().await;
    let needs: &[&str] = match example.name {
        "review-badges" => &["effort-review"],
        _ => &[],
    };
    for name in needs.iter().copied().chain([example.name]) {
        oxplow_sdk::scaffold_example(dir.path(), name, None).unwrap();
    }
    let report = check(
        dir.path(),
        example.name,
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        report.ok,
        "{}: {}",
        example.name,
        render_findings(&report, Format::Text)
    );
    let report = test_extension(dir.path(), example.name, false)
        .await
        .unwrap();
    assert_eq!(
        report.error_lines(),
        Vec::<String>::new(),
        "{}",
        example.name
    );
    // A component's `ui` steps need an app and a display that is awake.
    let warning_lines = report.warning_lines();
    let unexpected: Vec<_> = warning_lines
        .iter()
        .filter(|w| !w.contains("aren't run"))
        .collect();
    assert!(unexpected.is_empty(), "{}: {unexpected:?}", example.name);
    // Every example the manifest names ran.
    let manifest = example
        .files
        .iter()
        .find(|(rel, _)| *rel == "extension.yaml")
        .map(|(_, bytes)| serde_yaml::from_slice::<serde_yaml::Value>(bytes).unwrap())
        .unwrap();
    let named = manifest["intent"]["examples"].as_sequence().unwrap();
    assert!(!named.is_empty(), "{} names examples", example.name);
    for item in named {
        let ran = format!("example {}", item["name"].as_str().unwrap());
        assert!(
            report.ran.contains(&ran),
            "{ran} of {} in {:?}",
            example.name,
            report.ran
        );
    }
}

/// `--from <example> <name>` renames: every example copied as
/// `renamed-<example>` checks without errors and passes its own fixtures, so
/// no own reference was left naming the old extension.
async fn renames_and_tests_clean(example: &'static Example) {
    let mut failures = Vec::new();
    let _watch = HangWatch::on(example.name);
    let dir = project().await;
    // An example that builds on another comes with it, under its name.
    if example.name == "review-badges" {
        oxplow_sdk::scaffold_example(dir.path(), "effort-review", None).unwrap();
    }
    let name = format!("renamed-{}", example.name);
    let made = oxplow_sdk::scaffold_example(dir.path(), example.name, Some(&name)).unwrap();
    assert_eq!(made.name, name);
    let report = check(
        dir.path(),
        &name,
        &ExtensionCatalog::new(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    if !report.ok {
        failures.push(format!(
            "{name}: check: {}",
            render_findings(&report, Format::Text)
        ));
    }
    let report = test_extension(dir.path(), &name, false).await.unwrap();
    failures.extend(
        report
            .error_lines()
            .iter()
            .map(|e| format!("{name}: test: {e}")),
    );
    failures.extend(
        report
            .warning_lines()
            .iter()
            .filter(|w| !w.contains("aren't run"))
            .map(|w| format!("{name}: warning: {w}")),
    );
    if report.ran.is_empty() {
        failures.push(format!("{name}: ran no fixtures"));
    }
    assert_eq!(failures, Vec::<String>::new());
}

/// One test on a runtime of its own.
fn on_runtime(run: impl std::future::Future<Output = ()>) {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(run);
}

fn main() {
    let all = oxplow_sdk::examples::all();
    let enough = all.len() >= 9;
    let mut trials = vec![Trial::test("the_examples_are_compiled_in", move || {
        if enough {
            Ok(())
        } else {
            Err("the examples aren't compiled in".into())
        }
    })];
    for example in all {
        let name = example.name;
        trials.push(Trial::test(
            format!("{name}::scaffolds_checks_and_tests_clean"),
            move || {
                on_runtime(async { scaffolds_checks_and_tests_clean(example).await });
                Ok(())
            },
        ));
        trials.push(Trial::test(
            format!("{name}::renames_and_tests_clean"),
            move || {
                on_runtime(async { renames_and_tests_clean(example).await });
                Ok(())
            },
        ));
    }
    libtest_mimic::run(&Arguments::from_args(), trials).exit();
}
