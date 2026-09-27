//! Running extension-declared sources: consent, exec, coercion, storage.
//!
//! A source is code an extension ships (`runtime: exec`). It runs only
//! after a human approved that exact entry script (by content hash);
//! approvals live in `.oxplow/source-approvals.json`, which is local
//! state (gitignored), so each person consents on their own machine and
//! again whenever the script changes. The entry runs with a scrubbed
//! environment (PATH, HOME, the declared `env` names, OXPLOW_* context)
//! and must print `{"entities": {"<name>": [ {col: value, …}, … ]}}`.
//! See `.context/semantic-layer.md` → "User and extension sources".

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use oxplow_db::{EntityTable, SourceState, SqlCell, SqliteExtSourceStore, StoredType};
use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::extension_sources::{ColumnType, SourceEntity, SourceSpec};

/// How long a source may run.
pub const SOURCE_TIMEOUT: Duration = Duration::from_secs(120);
/// Largest stdout a source may produce.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// Local (gitignored) consent file under `.oxplow/`.
pub const APPROVALS_FILE: &str = "source-approvals.json";

/// Outcome of one run, as reported to the UI / agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct SourceRunReport {
    pub extension: String,
    pub source_id: String,
    pub row_counts: BTreeMap<String, i64>,
}

/// SHA-256 of the entry script: what an approval is bound to.
pub fn entry_hash(ext_dir: &Path, entry: &str) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(ext_dir.join(entry))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ApprovalFile {
    /// `"<extension>/<source>"` → approved entry hash.
    #[serde(default)]
    approved: BTreeMap<String, String>,
}

fn read_approvals(state_dir: &Path) -> ApprovalFile {
    std::fs::read_to_string(state_dir.join(APPROVALS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Whether `extension/source` is approved for this exact entry hash.
pub fn is_approved(state_dir: &Path, extension: &str, source: &str, hash: &str) -> bool {
    read_approvals(state_dir)
        .approved
        .get(&format!("{extension}/{source}"))
        .is_some_and(|h| h == hash)
}

/// Record a human's approval of `extension/source` at `hash`.
pub fn approve(state_dir: &Path, extension: &str, source: &str, hash: &str) -> std::io::Result<()> {
    let mut file = read_approvals(state_dir);
    file.approved
        .insert(format!("{extension}/{source}"), hash.to_string());
    std::fs::create_dir_all(state_dir)?;
    let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
    std::fs::write(state_dir.join(APPROVALS_FILE), text)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceOutput {
    entities: BTreeMap<String, Vec<serde_json::Value>>,
}

/// Run the entry and return its raw rows per entity.
pub fn exec_source(
    ext_dir: &Path,
    spec: &SourceSpec,
    host_env: &dyn Fn(&str) -> Option<String>,
    timeout: Duration,
) -> Result<BTreeMap<String, Vec<serde_json::Value>>, String> {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let entry = ext_dir.join(&spec.entry);
    if !entry.is_file() {
        return Err(format!(
            "entry `{}` doesn't exist in the extension folder",
            spec.entry
        ));
    }
    let mut cmd = Command::new(&entry);
    cmd.current_dir(ext_dir)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("OXPLOW_EXTENSION_DIR", ext_dir)
        .env("OXPLOW_SOURCE_ID", &spec.id);
    for name in ["PATH", "HOME"] {
        if let Some(v) = host_env(name) {
            cmd.env(name, v);
        }
    }
    for name in &spec.env {
        if let Some(v) = host_env(name) {
            cmd.env(name, v);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start `{}`: {e}", spec.entry))?;

    // Drain both pipes on threads so a chatty source can't deadlock.
    let mut out_pipe = child.stdout.take().ok_or("no stdout")?;
    let mut err_pipe = child.stderr.take().ok_or("no stderr")?;
    let out_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = out_pipe
            .by_ref()
            .take(MAX_OUTPUT_BYTES as u64 + 1)
            .read_to_end(&mut buf);
        buf
    });
    let err_thread = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = err_pipe.by_ref().take(64 * 1024).read_to_end(&mut buf);
        buf
    });

    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("source `{}` timed out after {timeout:?}", spec.id));
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };
    let stdout = out_thread.join().unwrap_or_default();
    let stderr = String::from_utf8_lossy(&err_thread.join().unwrap_or_default())
        .trim()
        .to_string();
    if !status.success() {
        return Err(format!(
            "source `{}` failed ({status}, exit code {:?}): {stderr}",
            spec.id,
            status.code()
        ));
    }
    if stdout.len() > MAX_OUTPUT_BYTES {
        return Err(format!(
            "source `{}` printed more than {MAX_OUTPUT_BYTES} bytes",
            spec.id
        ));
    }
    let parsed: SourceOutput = serde_json::from_slice(&stdout).map_err(|e| {
        format!(
            "source `{}` must print JSON like {{\"entities\": {{\"<name>\": [...]}}}}: {e}",
            spec.id
        )
    })?;
    if let Some(unknown) = parsed
        .entities
        .keys()
        .find(|k| !spec.entities.iter().any(|e| &e.name == *k))
    {
        return Err(format!(
            "source `{}` returned undeclared entity `{unknown}`",
            spec.id
        ));
    }
    Ok(parsed.entities)
}

/// Coerce raw JSON rows to the entity's declared columns (in order).
pub fn coerce_rows(
    entity: &SourceEntity,
    rows: Vec<serde_json::Value>,
) -> Result<Vec<Vec<SqlCell>>, String> {
    use serde_json::Value;
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let Value::Object(obj) = row else {
            return Err(format!(
                "entity `{}` row {i} must be an object",
                entity.name
            ));
        };
        let mut cells = Vec::with_capacity(entity.columns.len());
        for col in &entity.columns {
            let v = obj.get(&col.name).cloned().unwrap_or(Value::Null);
            let bad = || {
                format!(
                    "entity `{}` row {i}: column `{}` expects {:?}, got {v}",
                    entity.name, col.name, col.col_type
                )
            };
            let cell = match (&v, col.col_type) {
                (Value::Null, _) => SqlCell::Null(()),
                (Value::Number(n), ColumnType::Int) => SqlCell::Int(n.as_i64().ok_or_else(bad)?),
                (Value::Number(n), ColumnType::Real) => SqlCell::Real(n.as_f64().ok_or_else(bad)?),
                (Value::Bool(b), ColumnType::Bool) => SqlCell::Int(i64::from(*b)),
                (Value::String(s), ColumnType::Text | ColumnType::Time) => SqlCell::Text(s.clone()),
                _ => return Err(bad()),
            };
            if col.name == entity.key && cell == SqlCell::Null(()) {
                return Err(format!(
                    "entity `{}` row {i}: key `{}` is missing",
                    entity.name, entity.key
                ));
            }
            cells.push(cell);
        }
        out.push(cells);
    }
    Ok(out)
}

fn stored(t: ColumnType) -> StoredType {
    match t {
        ColumnType::Int | ColumnType::Bool => StoredType::Integer,
        ColumnType::Real => StoredType::Real,
        ColumnType::Text | ColumnType::Time => StoredType::Text,
    }
}

/// Run one source end to end: consent check (recording approval when a
/// human passed `approve`), exec, coercion, atomic store, run state.
/// Failures after the consent check are also recorded as the source's
/// state so the UI can show them.
pub async fn run_source(
    root: &Path,
    state_dir: &Path,
    store: &SqliteExtSourceStore,
    extension: &str,
    source_id: &str,
    approve_now: bool,
) -> Result<SourceRunReport, DomainError> {
    let ext = crate::extensions::load_extensions(root)
        .into_iter()
        .find(|e| e.name == extension)
        .ok_or(DomainError::NotFound)?;
    let spec = ext
        .sources
        .iter()
        .find(|s| s.id == source_id)
        .cloned()
        .ok_or(DomainError::NotFound)?;
    let ext_dir = root.join(&ext.path);
    let hash = entry_hash(&ext_dir, &spec.entry).map_err(|e| {
        DomainError::Invalid(format!("source `{source_id}`: entry `{}`: {e}", spec.entry))
    })?;
    if approve_now {
        approve(state_dir, extension, source_id, &hash)
            .map_err(|e| DomainError::Storage(format!("record approval: {e}")))?;
    } else if !is_approved(state_dir, extension, source_id, &hash) {
        return Err(DomainError::Invalid(format!(
            "source `{extension}/{source_id}` runs `{}` and needs a person's approval first \
             (Settings → Extensions → Approve & Run). Approval is per machine and per script version.",
            spec.entry
        )));
    }

    let result = run_approved(&ext_dir, extension, &spec, store).await;
    // Timestamp serializes as an RFC 3339 string.
    let now = serde_json::to_value(oxplow_domain::Timestamp::now())
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let state = match &result {
        Ok(report) => SourceState {
            extension: extension.to_string(),
            source_id: source_id.to_string(),
            status: "ok".into(),
            last_run_at: now,
            error: None,
            row_counts: report.row_counts.clone(),
        },
        Err(e) => SourceState {
            extension: extension.to_string(),
            source_id: source_id.to_string(),
            status: "error".into(),
            last_run_at: now,
            error: Some(e.clone()),
            row_counts: BTreeMap::new(),
        },
    };
    store.record_run(state).await?;
    result.map_err(DomainError::Invalid)
}

async fn run_approved(
    ext_dir: &Path,
    extension: &str,
    spec: &SourceSpec,
    store: &SqliteExtSourceStore,
) -> Result<SourceRunReport, String> {
    let dir = ext_dir.to_path_buf();
    let spec_owned = spec.clone();
    let raw = tokio::task::spawn_blocking(move || {
        exec_source(
            &dir,
            &spec_owned,
            &|k| std::env::var(k).ok(),
            SOURCE_TIMEOUT,
        )
    })
    .await
    .map_err(|e| format!("source task panicked: {e}"))??;

    let mut writes = Vec::new();
    let mut row_counts = BTreeMap::new();
    for entity in &spec.entities {
        // An entity the source didn't mention this run is left empty.
        let rows = coerce_rows(entity, raw.get(&entity.name).cloned().unwrap_or_default())?;
        row_counts.insert(entity.name.clone(), rows.len() as i64);
        writes.push((
            EntityTable {
                extension: extension.to_string(),
                entity: entity.name.clone(),
                view: entity.view.clone(),
                key: entity.key.clone(),
                columns: entity
                    .columns
                    .iter()
                    .map(|c| (c.name.clone(), stored(c.col_type)))
                    .collect(),
            },
            rows,
        ));
    }
    store
        .replace_rows(writes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(SourceRunReport {
        extension: extension.to_string(),
        source_id: spec.id.clone(),
        row_counts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension_sources::{parse_sources, SourceSchedule};
    use serde_json::json;

    fn spec(entry: &str, env: &[&str]) -> SourceSpec {
        let yaml = format!(
            "- id: gh\n  runtime: exec\n  entry: {entry}\n  env: [{}]\n  entities:\n    - name: pr\n      key: number\n      columns: {{ number: int, title: text, score: real, draft: bool, opened_at: time }}\n",
            env.join(", ")
        );
        let (s, e) = parse_sources("my-gh", &serde_yaml::from_str(&yaml).unwrap());
        assert!(e.is_empty(), "{e:?}");
        s.into_iter().next().unwrap()
    }

    fn script(dir: &Path, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn approvals_are_bound_to_the_entry_hash() {
        let ext = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        script(ext.path(), "bin/sync.sh", "echo one");
        let h1 = entry_hash(ext.path(), "bin/sync.sh").unwrap();
        assert!(!is_approved(state.path(), "my-gh", "gh", &h1));
        approve(state.path(), "my-gh", "gh", &h1).unwrap();
        assert!(is_approved(state.path(), "my-gh", "gh", &h1));
        assert!(!is_approved(state.path(), "my-gh", "other", &h1));
        script(ext.path(), "bin/sync.sh", "echo two");
        let h2 = entry_hash(ext.path(), "bin/sync.sh").unwrap();
        assert_ne!(h1, h2);
        assert!(
            !is_approved(state.path(), "my-gh", "gh", &h2),
            "a changed script needs re-approval"
        );
    }

    #[test]
    fn exec_passes_only_declared_env_and_parses_entities() {
        let ext = tempfile::tempdir().unwrap();
        script(
            ext.path(),
            "bin/sync.sh",
            r#"printf '{"entities":{"pr":[{"number":1,"title":"%s|%s"}]}}' "$GH_TOKEN" "$SECRET""#,
        );
        let env = |k: &str| match k {
            "GH_TOKEN" => Some("tok".to_string()),
            "SECRET" => Some("leak".to_string()),
            _ => None,
        };
        let out = exec_source(
            ext.path(),
            &spec("bin/sync.sh", &["GH_TOKEN"]),
            &env,
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(out["pr"][0]["title"], json!("tok|"));
    }

    #[test]
    fn exec_reports_failures_timeouts_and_bad_output() {
        let ext = tempfile::tempdir().unwrap();
        let none = |_: &str| None;
        script(ext.path(), "fail.sh", "echo boom >&2; exit 3");
        let e = exec_source(
            ext.path(),
            &spec("fail.sh", &[]),
            &none,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(e.contains("exit") && e.contains("boom"), "{e}");

        script(ext.path(), "slow.sh", "sleep 5");
        let e = exec_source(
            ext.path(),
            &spec("slow.sh", &[]),
            &none,
            Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(e.contains("timed out"), "{e}");

        script(ext.path(), "junk.sh", "echo not json");
        let e = exec_source(
            ext.path(),
            &spec("junk.sh", &[]),
            &none,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(e.contains("JSON"), "{e}");

        script(
            ext.path(),
            "undeclared.sh",
            r#"echo '{"entities":{"issue":[]}}'"#,
        );
        let e = exec_source(
            ext.path(),
            &spec("undeclared.sh", &[]),
            &none,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(e.contains("issue"), "{e}");

        let e = exec_source(
            ext.path(),
            &spec("missing.sh", &[]),
            &none,
            Duration::from_secs(10),
        )
        .unwrap_err();
        assert!(e.contains("missing.sh"), "{e}");
    }

    #[test]
    fn coerces_rows_to_declared_types() {
        let s = spec("x", &[]);
        let e = &s.entities[0];
        let rows = coerce_rows(
            e,
            vec![json!({"number": 7, "title": "T", "score": 2, "draft": true, "opened_at": "2026-09-27T00:00:00Z", "extra": 1})],
        )
        .unwrap();
        assert_eq!(
            rows,
            vec![vec![
                SqlCell::Int(7),
                SqlCell::Text("T".into()),
                SqlCell::Real(2.0),
                SqlCell::Int(1),
                SqlCell::Text("2026-09-27T00:00:00Z".into())
            ]]
        );
        // Missing optional columns are NULL.
        let rows = coerce_rows(e, vec![json!({"number": 8})]).unwrap();
        assert_eq!(rows[0][1], SqlCell::Null(()));
        // Wrong types and a missing key are errors naming the row + column.
        let err = coerce_rows(e, vec![json!({"number": "seven"})]).unwrap_err();
        assert!(err.contains("row 0") && err.contains("number"), "{err}");
        let err = coerce_rows(e, vec![json!({"title": "no key"})]).unwrap_err();
        assert!(err.contains("key"), "{err}");
        let err = coerce_rows(e, vec![json!([1, 2])]).unwrap_err();
        assert!(err.contains("object"), "{err}");
        let _ = SourceSchedule::Manual;
    }

    #[tokio::test]
    async fn run_source_requires_consent_then_stores_queryable_rows() {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join(".oxplow");
        let ext = root.path().join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: my-gh\nsources:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n",
        )
        .unwrap();
        script(
            &ext,
            "sync.sh",
            r#"echo '{"entities":{"pr":[{"number":1,"title":"First"},{"number":2,"title":"Second"}]}}'"#,
        );
        let db = oxplow_db::Database::in_memory();
        let store = SqliteExtSourceStore::new(db.clone());

        let err = run_source(root.path(), &state, &store, "my-gh", "gh", false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("approval")),
            "{err:?}"
        );
        assert!(
            store.list_states().await.unwrap().is_empty(),
            "refused runs record nothing"
        );

        let report = run_source(root.path(), &state, &store, "my-gh", "gh", true)
            .await
            .unwrap();
        assert_eq!(report.row_counts["pr"], 2);
        let out = oxplow_db::SemanticLayer::new(db)
            .query_sql("SELECT title FROM v_my_gh_pr ORDER BY number", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&out.rows).unwrap(),
            json!([["First"], ["Second"]])
        );

        // Approved now, so a later run needs no approve flag…
        run_source(root.path(), &state, &store, "my-gh", "gh", false)
            .await
            .unwrap();
        // …and a failing run is recorded, keeping the last good rows.
        script(&ext, "sync.sh", "echo nope >&2; exit 1");
        let err = run_source(root.path(), &state, &store, "my-gh", "gh", false)
            .await
            .unwrap_err();
        assert!(
            matches!(err, DomainError::Invalid(ref m) if m.contains("approval")),
            "script changed: {err:?}"
        );
        run_source(root.path(), &state, &store, "my-gh", "gh", true)
            .await
            .unwrap_err();
        let st = &store.list_states().await.unwrap()[0];
        assert_eq!(st.status, "error");
        assert!(st.error.as_deref().unwrap().contains("nope"));
        assert_eq!(st.row_counts["pr"], 2, "last good counts kept");
    }
}
