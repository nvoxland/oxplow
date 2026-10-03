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
//! Use a scratch team: the run files issues in it, and the cleanup
//! trashes **every issue the key's user created in that team since the
//! run began** — one filed by hand meanwhile goes with them. Without the
//! three variables the live test says it was skipped and passes; CI never
//! sets them.
//!
//! The same suite runs against the simulator on every build, so the code
//! a live run takes — cleanup included — is exercised. What only a live
//! run shows is whether Linear accepts what the simulator does: nobody
//! has run it against a workspace yet (`.context/providers.md` → "Live").

#![allow(clippy::unwrap_used)]

mod common;

use oxplow_provider_linear::graphql::{Client, Operation, DEFAULT_URL};
use oxplow_provider_linear::issue::ISSUE_DELETE;
use oxplow_provider_linear::sim::LinearSim;
use oxplow_sdk::plugin_test::{test_extension, TestReport};
use serde_json::{json, Value};

/// The issues the key's user created in a team since a time: what a run
/// left behind. (Trashed issues aren't listed, so a second cleanup finds
/// nothing.)
const ISSUES_CREATED: Operation = Operation {
    name: "IssuesCreated",
    document: "query IssuesCreated($filter: IssueFilter!, $first: Int!, $after: String) { \
               issues(filter: $filter, first: $first, after: $after) { nodes { id identifier } \
               pageInfo { hasNextPage endCursor } } }",
};

/// Where a run goes: a team of a workspace, by an API key.
struct Target {
    key: String,
    team: String,
    /// The GraphQL endpoint; Linear's own unless it is the simulator's.
    url: String,
}

/// Trash every issue the key's user created in the team since `since`
/// (RFC 3339): their identifiers.
async fn trash_created_since(target: &Target, since: &str) -> Vec<String> {
    let client = Client::new(&target.url, &target.key);
    let filter = json!({
        "team": { "key": { "eq": target.team } },
        "createdAt": { "gte": since },
        "creator": { "isMe": { "eq": true } },
    });
    let mut found: Vec<(String, String)> = Vec::new();
    let mut after = Value::Null;
    loop {
        let page = client
            .run(
                ISSUES_CREATED,
                json!({ "filter": filter, "first": 50, "after": after }),
            )
            .await
            .expect("listing the issues the run created");
        let issues = &page["issues"];
        for node in issues["nodes"].as_array().into_iter().flatten() {
            found.push((
                node["id"].as_str().unwrap_or_default().to_string(),
                node["identifier"].as_str().unwrap_or_default().to_string(),
            ));
        }
        if issues["pageInfo"]["hasNextPage"] != true {
            break;
        }
        after = issues["pageInfo"]["endCursor"].clone();
    }
    let mut trashed = Vec::new();
    for (id, identifier) in found {
        client
            .run(ISSUE_DELETE, json!({ "id": id }))
            .await
            .unwrap_or_else(|e| panic!("trashing {identifier}: {e}"));
        trashed.push(identifier);
    }
    trashed
}

/// Run `oxplow plugin test` on the example against `target`, then trash
/// what it created: the report, and the trashed issues' identifiers.
///
/// The transcript is blessed into the throwaway copy rather than compared:
/// a workspace's issue numbers, ids and times aren't the golden's.
async fn suite(target: &Target, network: &str) -> (TestReport, Vec<String>) {
    // `plugin test` takes credentials and declared env from its own
    // environment, as a person running it would.
    std::env::set_var("LINEAR_API_KEY", &target.key);
    if target.url == DEFAULT_URL {
        std::env::remove_var("LINEAR_API_URL");
    } else {
        std::env::set_var("LINEAR_API_URL", &target.url);
    }
    let dir = tempfile::tempdir().unwrap();
    let ext = common::install_example(dir.path()).await;
    common::rewrite(&ext, "extension.yaml", "network: [api.linear.app]", network);
    common::rewrite(
        &ext,
        "fixtures/provider-linear.yaml",
        "config: { team: ENG }",
        &format!("config: {{ team: {} }}", target.team),
    );
    // A minute's margin for a clock that isn't Linear's.
    let since =
        oxplow_domain::Timestamp::from_unix_ms(oxplow_domain::Timestamp::now().unix_ms() - 60_000)
            .to_string();
    let report = test_extension(dir.path(), "linear", true).await;
    // Whatever the run came to, what it filed goes.
    let trashed = trash_created_since(target, &since).await;
    (report.unwrap(), trashed)
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
    let (report, trashed) = suite(&target, "network: [api.linear.app, localhost]").await;
    assert_eq!(report.errors, Vec::<String>::new());
    assert!(
        report.ran.iter().any(|r| r == "work_items suite"),
        "{:?}",
        report.ran
    );
    // Everything the run filed is in the trash; nothing is left to find.
    assert!(!trashed.is_empty());
    assert_eq!(sim.live_issues(), Vec::<String>::new());
    assert_eq!(
        trash_created_since(&target, "2000-01-01T00:00:00.000Z").await,
        Vec::<String>::new()
    );
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
