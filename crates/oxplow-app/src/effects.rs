//! An extension's effects (`effects:` in its manifest, experimental — a
//! private extension only; P8.D9–D11, `.context/extensions.md` →
//! "Effects"): a Starlark script that reacts to a logged event by
//! composing commands, run as `Actor::Effect` with an agent's rights.
//!
//! ```yaml
//! effects:
//!   - id: announce-done          # [a-z0-9-]+, unique in the extension
//!     summary: Note a finished item on its thread.
//!     on: [work_item.transitioned]
//!     where: { to: done }        # optional: payload fields equal to these
//!     input: "SELECT title FROM v_work_item WHERE ref = :work_item"   # optional; payload fields bound
//!     entry: effects/announce.star   # transform({event, rows}) → {commands, events?} | {skip}
//!     after: [page_ref.work_item]    # optional: consumers it waits for
//! ```
//!
//! Nothing runs until a person approves the effect on this machine
//! (`exec_consent::ProgramKind::Effect`, hashed over the whole extension
//! folder, so any edit needs approving again), and an approval starts it
//! at the log's head (`effect_state`): an effect never reacts to what
//! happened before its approval.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::exec_consent::{ApprovalStore, ProgramKind, ProjectProgram};
use crate::extensions::manifest_v2::{at, entry_line, key_line};
use crate::extensions::Extension;

/// An effect as loaded (valid ones; invalid ones are in the extension's
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EffectDecl {
    pub id: String,
    pub extension: String,
    pub summary: String,
    /// The event types it reacts to.
    pub on: Vec<String>,
    /// Payload fields that must equal these values (the collectors'
    /// `where`).
    pub filter: BTreeMap<String, String>,
    /// SQL whose rows the script gets, the event's payload fields bound.
    pub input: Option<String>,
    /// The script's path in the folder.
    pub entry: String,
    /// Its source.
    pub script: String,
    /// Consumers it waits for on each event.
    pub after: Vec<String>,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

impl EffectDecl {
    /// `<extension>/<id>`: its actor (`Actor::Effect`), approval and state.
    pub fn name(&self) -> String {
        format!("{}/{}", self.extension, self.id)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EffectFile {
    id: String,
    summary: String,
    on: serde_yaml::Value,
    #[serde(default, rename = "where")]
    filter: Option<serde_yaml::Value>,
    #[serde(default)]
    input: Option<String>,
    entry: String,
    #[serde(default)]
    after: Vec<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Parse an `effects:` block: each effect's trigger (`on`, `where`, by the
/// collectors' rules, against `knows_event`), its script (in the folder,
/// defining `transform`); what's wrong is `file:line`.
pub fn parse_effects(
    extension: &str,
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
    knows_event: &dyn Fn(&str) -> bool,
) -> (Vec<EffectDecl>, Vec<String>) {
    let block = key_line(manifest, "effects");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`effects` must be a list")],
        );
    };
    let mut out: Vec<EffectDecl> = Vec::new();
    let mut errors = Vec::new();
    for item in items {
        let f: EffectFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, block, format!("effect: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "effects", "id", &f.id).or(block);
        let declared_at = at(file, line, "").trim_end_matches(": ").to_string();
        match decl_of(extension, f, read, knows_event, declared_at) {
            Ok(d) if out.iter().any(|o| o.id == d.id) => errors.push(at(
                file,
                line,
                format!("effect `{}` is declared twice", d.id),
            )),
            Ok(d) => out.push(d),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

fn decl_of(
    extension: &str,
    f: EffectFile,
    read: &dyn Fn(&str) -> Option<String>,
    knows_event: &dyn Fn(&str) -> bool,
    declared_at: String,
) -> Result<EffectDecl, String> {
    if !valid_id(&f.id) {
        return Err(format!(
            "effect id `{}` must be lowercase letters, digits and dashes",
            f.id
        ));
    }
    let named = |m: String| format!("effect `{}`: {m}", f.id);
    let mut trigger = serde_yaml::Mapping::new();
    trigger.insert("on".into(), f.on.clone());
    if let Some(w) = &f.filter {
        trigger.insert("where".into(), w.clone());
    }
    let (on, filter) = match oxplow_config::collectors::parse_trigger(
        Some(&serde_yaml::Value::Mapping(trigger)),
        knows_event,
    )
    .map_err(|e| named(e.replacen("trigger: ", "", 1)))?
    {
        oxplow_config::collectors::Trigger::On { events, filter } => (events, filter),
        _ => return Err(named("`on` lists the event types it reacts to".into())),
    };
    let script = read(&f.entry)
        .ok_or_else(|| named(format!("entry `{}` isn't in the extension", f.entry)))?;
    oxplow_collect_plugin::runtime::check_starlark(&f.entry, &script)
        .map_err(|e| named(format!("`{}` {e}", f.entry)))?;
    Ok(EffectDecl {
        id: f.id,
        extension: extension.to_string(),
        summary: f.summary,
        on,
        filter,
        input: f.input,
        entry: f.entry,
        script,
        after: f.after,
        declared_at,
    })
}

/// An effect as a program to approve: its script, over every file of its
/// extension's folder (the manifest, whose `on`/`where`/`input` decide
/// when and with what it runs, included).
pub fn effect_program(ext: &Extension, decl: &EffectDecl) -> ProjectProgram {
    let dir = ext.path.trim_end_matches('/');
    ProjectProgram {
        kind: ProgramKind::Effect,
        name: decl.name(),
        program: format!("{dir}/{}", decl.entry),
        args: Vec::new(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        tree: Some(dir.to_string()),
        approved: false,
        version: None,
    }
}

/// Whether an effect may react to the event at `seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Not approved as it is now (never approved, or edited since).
    Unapproved,
    /// Approved, but the event was logged before (or at) its approval.
    BeforeApproval,
    Runs,
}

/// Decide [`Gate`] for `decl` at event `seq`, given where it starts
/// (`effect_state`).
pub fn gate(
    store: &ApprovalStore,
    project_dir: &Path,
    ext: &Extension,
    decl: &EffectDecl,
    start_after: Option<i64>,
    seq: i64,
) -> Gate {
    let program = effect_program(ext, decl);
    let approved = program
        .hash(project_dir)
        .is_ok_and(|h| store.is_approved(&program.key(), &h));
    match start_after {
        _ if !approved => Gate::Unapproved,
        None => Gate::Unapproved,
        Some(start) if seq <= start => Gate::BeforeApproval,
        Some(_) => Gate::Runs,
    }
}

/// A person approved effect `name`: it starts after the log's head.
pub async fn approved(
    db: &oxplow_db::Database,
    name: &str,
) -> Result<i64, oxplow_domain::DomainError> {
    let (name, now) = (
        name.to_string(),
        oxplow_domain::Timestamp::now().to_string(),
    );
    db.transaction(move |tx| oxplow_db::effect_state_store::start_at_head_tx(tx, &name, &now))
        .await
}

/// Where effect `name` starts reading the log; `None` before approval.
pub async fn start_after(
    db: &oxplow_db::Database,
    name: &str,
) -> Result<Option<i64>, oxplow_domain::DomainError> {
    let name = name.to_string();
    db.read(move |tx| oxplow_db::effect_state_store::start_after_tx(tx, &name))
        .await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::extensions::load_extensions;

    pub(crate) const MANIFEST: &str = "manifest: 2
name: acme
sharing: private
intent: { purpose: Effects., origin: null, examples: [] }
effects:
  - id: announce-done
    summary: Note a finished item.
    on: [work_item.transitioned]
    where: { to: done }
    entry: effects/announce.star
";

    pub(crate) const SCRIPT: &str = "def transform(x):\n    return {\"skip\": \"nothing to do\"}\n";

    pub(crate) fn write_acme(root: &Path, manifest: &str, script: &str) {
        let dir = root.join("oxplow/extensions/acme/effects");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(root.join("oxplow/extensions/acme/extension.yaml"), manifest).unwrap();
        std::fs::write(dir.join("announce.star"), script).unwrap();
    }

    fn acme(root: &Path) -> Extension {
        load_extensions(root)
            .into_iter()
            .find(|e| e.name == "acme")
            .unwrap()
    }

    #[test]
    fn an_effect_loads_with_its_trigger_and_script() {
        let dir = tempfile::tempdir().unwrap();
        write_acme(dir.path(), MANIFEST, SCRIPT);
        let ext = acme(dir.path());
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let e = &ext.effects[0];
        assert_eq!(e.name(), "acme/announce-done");
        assert_eq!(e.on, vec!["work_item.transitioned".to_string()]);
        assert_eq!(e.filter.get("to").map(String::as_str), Some("done"));
        assert_eq!(e.declared_at, "oxplow/extensions/acme/extension.yaml:6");
    }

    #[test]
    fn a_broken_effect_is_an_error_at_its_line() {
        for (from, to, says) in [
            (
                "on: [work_item.transitioned]",
                "on: [nope.never]",
                "isn't a registered event type",
            ),
            (
                "entry: effects/announce.star",
                "entry: effects/gone.star",
                "isn't in the extension",
            ),
            ("id: announce-done", "id: Announce", "lowercase"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            write_acme(dir.path(), &MANIFEST.replace(from, to), SCRIPT);
            let ext = acme(dir.path());
            assert!(ext.effects.is_empty(), "{to}");
            let errors = ext.errors.join("\n");
            assert!(
                errors.contains("extension.yaml:6:") && errors.contains(says),
                "{to}: {errors}"
            );
        }
    }

    async fn gate_at(svc: &crate::Services, root: &Path, seq: i64) -> Gate {
        let ext = acme(root);
        let decl = &ext.effects[0];
        let start = start_after(&svc.db, &decl.name()).await.unwrap();
        gate(&svc.approvals, root, &ext, decl, start, seq)
    }

    /// P8.D9: an unapproved effect never runs; a person's approval starts
    /// it at the log's head; an edit stops it until it's approved again,
    /// which starts it at the head again.
    #[tokio::test]
    async fn an_effect_runs_only_approved_and_only_on_what_follows_its_approval() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        write_acme(&root, MANIFEST, SCRIPT);
        let log = || async {
            svc.event_log_store
                .append(
                    oxplow_domain::Envelope::new(
                        "config.changed",
                        1,
                        "test",
                        serde_json::json!({ "key": "zones", "before": null, "after": [] }),
                    )
                    .unwrap(),
                )
                .await
                .unwrap()
        };
        let before = log().await;
        assert_eq!(gate_at(svc, &root, before).await, Gate::Unapproved);

        let config = svc.config.read().unwrap().clone();
        let approve = || {
            let ext = acme(&root);
            let program = effect_program(&ext, &ext.effects[0]);
            crate::exec_consent::approve_program(
                &svc.approvals,
                &root,
                &config,
                std::slice::from_ref(&ext),
                ProgramKind::Effect,
                &program.name,
                &program.hash(&root).unwrap(),
            )
            .unwrap();
        };
        approve();
        let head = approved(&svc.db, "acme/announce-done").await.unwrap();
        assert_eq!(head, before);
        assert_eq!(gate_at(svc, &root, before).await, Gate::BeforeApproval);
        let after = log().await;
        assert_eq!(gate_at(svc, &root, after).await, Gate::Runs);

        write_acme(&root, MANIFEST, &format!("{SCRIPT}# edited\n"));
        assert_eq!(gate_at(svc, &root, after).await, Gate::Unapproved);
        approve();
        let head = approved(&svc.db, "acme/announce-done").await.unwrap();
        assert_eq!(gate_at(svc, &root, head).await, Gate::BeforeApproval);
        assert_eq!(gate_at(svc, &root, log().await).await, Gate::Runs);
    }
}
