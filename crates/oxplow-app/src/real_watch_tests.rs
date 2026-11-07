//! What a real watch over a real project has to deliver: a change made outside
//! oxplow reaches what follows it, with no restart. The logic behind it is
//! tested with no OS (`oxplow-fs-watch`, the batch tests of each follower); this
//! is the one place all of it runs together, on the services a daemon boots.

use std::time::Duration;

use serde_json::json;

use crate::test_fixtures::{until, until_some};

const WAIT: Duration = Duration::from_secs(60);

async fn count(svc: &crate::Services, sql: &str) -> i64 {
    let rows = svc.sql.query_sql(sql, vec![], None).await.unwrap().rows;
    serde_json::to_value(rows).unwrap()[0][0]
        .as_i64()
        .unwrap_or(0)
}

/// `path` is in the stream's working-files list.
async fn listed(svc: &crate::Services, stream: i64, path: &str) {
    let sql = format!(
        "SELECT count(*) FROM v_change_file f JOIN v_change c ON c.id = f.change_id \
         WHERE c.kind = 'working' AND c.stream_id = {stream} AND f.path = '{path}'"
    );
    assert!(
        until(WAIT, || async { count(svc, &sql).await == 1 }).await,
        "{path} never reached stream {stream}'s working files"
    );
}

/// A snapshot holds `path`. A take comes once the tree has been quiet.
async fn snapshotted(svc: &crate::Services, path: &str) {
    let sql = format!("SELECT count(*) FROM v_snapshot_file WHERE path = '{path}'");
    assert!(
        until(WAIT, || async { count(svc, &sql).await > 0 }).await,
        "{path} never reached a snapshot"
    );
}

async fn streams(svc: &crate::Services) -> Vec<oxplow_domain::Stream> {
    svc.streams.list_streams().await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn what_changes_on_disk_reaches_what_follows_it_with_no_restart() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let project = dir.path().to_path_buf();
    let svc = crate::Services::in_memory(&project).await.unwrap();
    crate::boot::run_boot_orchestration(&svc).await.unwrap();
    // The watches and the first sweep are up: a change from here is heard.
    assert!(
        until(WAIT, || async {
            svc.background_tasks.list_running().is_empty()
        })
        .await,
        "boot never finished"
    );
    let primary = streams(&svc).await[0].clone();
    let primary_id: i64 = primary.id.value();

    // An edit reaches the working-files list.
    std::fs::write(project.join("edited.txt"), "hop\n").unwrap();
    listed(&svc, primary_id, "edited.txt").await;

    // A folder made after the watch started: `mkdir d && write d/f` races any
    // new watch, and the file is in a snapshot all the same.
    std::fs::create_dir_all(project.join("made-later/deep")).unwrap();
    std::fs::write(project.join("made-later/deep/f.txt"), "x\n").unwrap();
    snapshotted(&svc, "made-later/deep/f.txt").await;

    // A burst larger than anything could queue path by path: all of it lands.
    std::fs::create_dir(project.join("burst")).unwrap();
    for i in 0..3000 {
        std::fs::write(project.join(format!("burst/f{i}.txt")), format!("{i}\n")).unwrap();
    }
    let burst = format!(
        "SELECT count(*) FROM v_change_file f JOIN v_change c ON c.id = f.change_id \
         WHERE c.kind = 'working' AND c.stream_id = {primary_id} AND f.path LIKE 'burst/%'"
    );
    assert!(
        until(Duration::from_secs(120), || async {
            count(&svc, &burst).await == 3000
        })
        .await,
        "the whole burst never reached the working files: {}",
        count(&svc, &burst).await
    );

    // The wiki's row follows its file.
    std::fs::create_dir_all(project.join(".oxplow/wiki")).unwrap();
    std::fs::write(
        project.join(".oxplow/wiki/by-hand.md"),
        "# By hand\n\nbody\n",
    )
    .unwrap();
    let page = "SELECT count(*) FROM v_knowledge_page WHERE slug = 'by-hand'";
    assert!(
        until(WAIT, || async { count(&svc, page).await == 1 }).await,
        "the wiki page's row never appeared"
    );

    // A hand edit of the config goes live, and a `generated` change applies:
    // what it excludes leaves the file list, and goes out of the next snapshot.
    let left_out = "SELECT count(*) FROM v_file WHERE path = 'left-out/before.txt'";
    std::fs::create_dir(project.join("left-out")).unwrap();
    std::fs::write(project.join("left-out/before.txt"), "x\n").unwrap();
    assert!(
        until(WAIT, || async { count(&svc, left_out).await == 1 }).await,
        "the folder never reached the file list"
    );
    let config = project.join(".oxplow/project.yaml");
    let mut text = std::fs::read_to_string(&config).unwrap_or_default();
    text.push_str("\ngenerated:\n  exclude:\n    - left-out\n");
    std::fs::write(&config, text).unwrap();
    assert!(
        until(WAIT, || async { count(&svc, left_out).await == 0 }).await,
        "the hand edit of the config never applied"
    );
    std::fs::write(project.join("left-out/no.txt"), "no\n").unwrap();
    std::fs::write(project.join("after-generated.txt"), "yes\n").unwrap();
    snapshotted(&svc, "after-generated.txt").await;
    assert_eq!(
        count(
            &svc,
            "SELECT count(*) FROM v_snapshot_file WHERE path = 'left-out/no.txt'"
        )
        .await,
        0,
        "what `generated` excludes is not snapshotted"
    );

    // A stream made now is followed like one that was there at boot: its
    // files, by its own ignore rules, and its commits.
    svc.commands
        .run(
            &oxplow_domain::Actor::Human,
            "oxplow.stream.create",
            json!({
                "title": "Made Later",
                "source": "new",
                "new_branch": "made-later",
                "start_point": "HEAD",
            }),
            false,
        )
        .await
        .unwrap();
    let later = until_some(WAIT, || async {
        streams(&svc)
            .await
            .into_iter()
            .find(|s| s.title == "Made Later")
    })
    .await
    .expect("the new stream exists");
    let tree = std::path::PathBuf::from(&later.worktree_path);
    let ignore = tree.join(".gitignore");
    let mut rules = std::fs::read_to_string(&ignore).unwrap_or_default();
    rules.push_str("only-here.txt\n");
    std::fs::write(&ignore, rules).unwrap();
    std::fs::write(tree.join("only-here.txt"), "ignored here\n").unwrap();
    std::fs::write(tree.join("in-the-worktree.txt"), "x\n").unwrap();
    listed(&svc, later.id.value(), "in-the-worktree.txt").await;
    snapshotted(&svc, "in-the-worktree.txt").await;
    assert_eq!(
        count(
            &svc,
            "SELECT count(*) FROM v_snapshot_file WHERE path = 'only-here.txt'"
        )
        .await,
        0,
        "its own .gitignore rules it"
    );
    crate::test_fixtures::commit_all(&tree, "Commit in the later stream");
    assert!(
        until(WAIT, || async {
            count(
                &svc,
                "SELECT count(*) FROM v_commit WHERE subject LIKE 'Commit in the later stream%'",
            )
            .await
                > 0
        })
        .await,
        "the later stream's commit never appeared"
    );
}
