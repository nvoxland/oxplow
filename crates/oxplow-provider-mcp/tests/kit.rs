//! P7.A6: the `notes` fixture extension — an MCP server behind oxplow's
//! adapter — passes `oxplow plugin test`: the handshake, the tool pin at
//! check, its `create` example, the read-back, and the work-items
//! conformance suite through a throwaway host over the adapter.
//! `OXPLOW_BLESS=1` rewrites its golden transcript.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use oxplow_sdk::plugin_test::test_extension;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_notes_extension_passes_plugin_test_over_the_adapter() {
    let dir = tempfile::tempdir().unwrap();
    oxplow_app::vcs::GitProvider
        .init_repository(dir.path())
        .await
        .unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes");
    let ext = dir.path().join("oxplow/extensions/notes");
    copy_dir(&fixture, &ext);
    let server = ext.join("bin/notes-server");
    std::fs::write(
        &server,
        format!(
            "#!/bin/sh\nexec '{}' \"$@\"\n",
            env!("CARGO_BIN_EXE_oxplow-provider-mcp-notes")
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&server, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The host runs oxplow's adapter from beside this test's build.
    assert!(
        oxplow_app::providers::host::adapter_bin().is_file(),
        "{}",
        oxplow_app::providers::host::adapter_bin().display()
    );

    let bless = std::env::var_os("OXPLOW_BLESS").is_some();
    let report = test_extension(dir.path(), "notes", bless).await.unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
    if bless {
        std::fs::create_dir_all(fixture.join("fixtures/transcripts")).unwrap();
        std::fs::copy(
            ext.join("fixtures/transcripts/notes.jsonl"),
            fixture.join("fixtures/transcripts/notes.jsonl"),
        )
        .unwrap();
    }
    for ran in ["work_items suite", "read items", "discover"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
}

/// P9.B4: the same extension with its server reached by `url` — the notes
/// served over streamable HTTP — passes `oxplow plugin test` too: the host
/// starts the adapter with `--url`, through the egress proxy where the OS
/// enforces `network`. The server outlives each start of the adapter (as a
/// real one would), so its notes' ids aren't the golden transcript's: the
/// run blesses its own, in the throwaway copy.
#[tokio::test(flavor = "multi_thread")]
async fn the_notes_extension_passes_plugin_test_with_its_server_by_url() {
    let dir = tempfile::tempdir().unwrap();
    oxplow_app::vcs::GitProvider
        .init_repository(dir.path())
        .await
        .unwrap();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/notes");
    let ext = dir.path().join("oxplow/extensions/notes");
    copy_dir(&fixture, &ext);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let _ = oxplow_provider_mcp::notes::serve_http(
            listener,
            None,
            oxplow_provider_mcp::notes::Refusal::Challenge,
        )
        .await;
    });
    let manifest = std::fs::read_to_string(ext.join("extension.yaml")).unwrap();
    let by_command = "      mcp: { command: [bin/notes-server] }\n";
    assert!(manifest.contains(by_command));
    std::fs::write(
        ext.join("extension.yaml"),
        manifest.replace(by_command, &format!("      mcp: {{ url: \"{url}\" }}\n"))
            + "    network: [127.0.0.1]\n",
    )
    .unwrap();

    let report = test_extension(dir.path(), "notes", true).await.unwrap();
    assert_eq!(report.errors, Vec::<String>::new());
    for ran in ["work_items suite", "read items", "discover"] {
        assert!(
            report.ran.iter().any(|r| r == ran),
            "{ran}: {:?}",
            report.ran
        );
    }
    server.abort();
}
