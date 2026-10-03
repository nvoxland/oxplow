//! P9.B5: the example extension against **a real Linear workspace** —
//! `oxplow plugin test` (handshake, check, its `create` example, the
//! read-back, the work-items conformance suite) over Linear's API, then
//! the issues the run created go to Linear's trash.
//!
//! It runs only when asked:
//!
//! ```text
//! OXPLOW_LIVE_LINEAR=1 LINEAR_API_KEY=lin_api_… LINEAR_TEAM=ENG \
//!     cargo test -p oxplow-provider-linear --test live -- --nocapture
//! ```
//!
//! Use a scratch team: the run files issues in it. Afterwards it trashes
//! **exactly the issues the run reports it left** (`TestReport.left`: what
//! a `create` example returned, and what the conformance suite filed and
//! didn't delete) — never one filed by hand, nor another run's. Without
//! the three variables the live test says it was skipped and passes; CI
//! never sets them.
//!
//! The same suite runs against the simulator on every build, so the code
//! a live run takes — cleanup included — is exercised. What only a live
//! run shows is whether Linear accepts what the simulator does: nobody
//! has run it against a workspace yet (`.context/providers.md` → "Live").

#![allow(clippy::unwrap_used)]

mod common;

use oxplow_provider_linear::graphql::{Client, DEFAULT_URL};
use oxplow_provider_linear::issue::{ISSUE_CREATE, ISSUE_DELETE};
use oxplow_provider_linear::sim::{LinearSim, TEAM_ID};
use oxplow_sdk::plugin_test::{test_extension_in, TestReport};
use serde_json::json;

/// Where a run goes: a team of a workspace, by an API key.
struct Target {
    key: String,
    team: String,
    /// The GraphQL endpoint; Linear's own unless it is the simulator's.
    url: String,
}

/// Trash the issues `left` names (`work_item:<id>:ENG-5` refs): their
/// identifiers.
async fn trash(target: &Target, left: &[String]) -> Vec<String> {
    let client = Client::new(&target.url, &target.key);
    let mut trashed = Vec::new();
    for item in left {
        let identifier = item.rsplit(':').next().unwrap_or_default();
        client
            .run(ISSUE_DELETE, json!({ "id": identifier }))
            .await
            .unwrap_or_else(|e| panic!("trashing {identifier}: {e}"));
        trashed.push(identifier.to_string());
    }
    trashed
}

/// Run `oxplow plugin test` on the example against `target`, then trash
/// what it created: the report, and the trashed issues' identifiers.
///
/// The transcript is blessed into the throwaway copy rather than compared:
/// a workspace's issue numbers, ids and times aren't the golden's.
async fn suite(target: &Target, network: &str) -> (TestReport, Vec<String>) {
    // The run's key and endpoint are its own, never this process's: the
    // simulator's run and a live one share the binary.
    let env = common::linear_env(
        &target.key,
        (target.url != DEFAULT_URL).then_some(target.url.as_str()),
    );
    let dir = tempfile::tempdir().unwrap();
    let ext = common::install_example(dir.path()).await;
    common::rewrite(&ext, "extension.yaml", "network: [api.linear.app]", network);
    common::rewrite(
        &ext,
        "fixtures/provider-linear.yaml",
        "config: { team: ENG }",
        &format!("config: {{ team: {} }}", target.team),
    );
    let report = test_extension_in(dir.path(), "linear", true, env)
        .await
        .expect("plugin test ran (had it not, it filed nothing)");
    // Whatever the run came to, what it left goes.
    let trashed = trash(target, &report.left).await;
    (report, trashed)
}

/// The suite against the simulator: the path a live run takes, cleanup
/// included.
#[tokio::test(flavor = "multi_thread")]
async fn the_live_suite_runs_and_cleans_up_against_the_simulator() {
    let sim = LinearSim::start("lin_api_live").await.unwrap();
    let target = Target {
        key: "lin_api_live".into(),
        team: "ENG".into(),
        url: sim.url.clone(),
    };
    // Someone files an issue in the team by hand, as the run starts.
    Client::new(&target.url, &target.key)
        .run(
            ISSUE_CREATE,
            json!({ "input": { "teamId": TEAM_ID, "title": "Filed by hand" } }),
        )
        .await
        .unwrap();
    let (report, trashed) = suite(&target, "network: [api.linear.app, localhost]").await;
    assert_eq!(report.errors, Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "work_items suite"),
        "{:?}",
        report.ran
    );
    // Everything the run filed is in the trash, and nothing else: an
    // issue filed beside the run (by hand, by another run) stays.
    assert!(!trashed.is_empty());
    assert_eq!(sim.live_issues(), vec!["ENG-1".to_string()]);
}

/// The suite against Linear itself, when asked for.
#[tokio::test(flavor = "multi_thread")]
async fn the_example_extension_passes_plugin_test_against_linear() {
    let asked = std::env::var("OXPLOW_LIVE_LINEAR").is_ok_and(|v| v == "1");
    let (Some(key), Some(team), true) = (
        std::env::var("LINEAR_API_KEY")
            .ok()
            .filter(|k| !k.is_empty()),
        std::env::var("LINEAR_TEAM").ok().filter(|t| !t.is_empty()),
        asked,
    ) else {
        eprintln!(
            "SKIPPED: the live Linear suite runs only with OXPLOW_LIVE_LINEAR=1, LINEAR_API_KEY \
             and LINEAR_TEAM (a scratch team: it files issues there and trashes them)."
        );
        return;
    };
    let target = Target {
        key,
        team,
        url: DEFAULT_URL.into(),
    };
    let (report, trashed) = suite(&target, "network: [api.linear.app]").await;
    eprintln!(
        "live Linear: ran {:?}; trashed {} issue(s): {}",
        report.ran,
        trashed.len(),
        trashed.join(", ")
    );
    assert_eq!(report.errors, Vec::<String>::new());
    for ran in ["work_items suite", "read issues", "discover"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
}
