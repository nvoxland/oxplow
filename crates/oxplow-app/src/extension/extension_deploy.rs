//! Deploying extensions (`.context/extensions-deploy.md` "Deployments"): what
//! `oxplow.extension.deploy`, `rollback`, `enable`, `disable`, `remove`,
//! `share` and `unshare` do, and the startup pass that deploys an enabled
//! extension with no deployment here.
//!
//! A deploy checks the source, copies it into the store
//! (`extension_store`), and activates the live set with that deployment in
//! it. The activation stages first, so a deployment that doesn't prepare
//! changes nothing live and its problems are the deploy's error. The call
//! returns once the deployment runs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_config::ExtensionSettings;
use oxplow_db::extension_deployment_store::{self as store_rows, DeploymentRow};
use oxplow_db::TxError;
use oxplow_domain::diagnostic::Diagnostic;
use oxplow_domain::{DomainError, Timestamp};
use serde::Serialize;

use crate::extension::extension_catalog::LiveChanges;
use crate::extension::extension_store;
use crate::extension::extensions::Permissions;
use crate::vocabulary_reactor::VocabularyErrors;
use crate::Services;

/// What a deploy did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct Deployed {
    pub name: String,
    /// The hash of the source it deployed: the deployment's identity.
    pub hash: String,
    /// It was already the active deployment: nothing changed.
    pub already: bool,
    /// What the person should know, the deployment went live regardless.
    pub warnings: Vec<String>,
    /// The primary checkout's HEAD the deployed source was at; `None` for
    /// a source outside the project, or a shipped copy.
    pub commit: Option<String>,
    /// The deployed source had uncommitted changes.
    pub dirty: bool,
}

impl Deployed {
    /// Where the deployment came from, for a message: `commit abc1234`,
    /// with `(uncommitted changes)` when dirty; `None` when it has no commit.
    pub fn provenance(&self) -> Option<String> {
        provenance_text(self.commit.as_deref(), self.dirty)
    }
}

fn provenance_text(commit: Option<&str>, dirty: bool) -> Option<String> {
    let commit = commit?;
    let short = &commit[..commit.len().min(7)];
    Some(if dirty {
        format!("commit {short} (uncommitted changes)")
    } else {
        format!("commit {short}")
    })
}

/// Where `source` came from: the checkout's HEAD and whether `source` had
/// uncommitted changes (untracked files included). Both empty for a
/// source outside `project`, or a project that isn't a repository.
async fn provenance(svc: &Services, project: &Path, source: &Path) -> (Option<String>, bool) {
    let Ok(rel) = source.strip_prefix(project) else {
        return (None, false);
    };
    let Some(commit) = svc.vcs.head(project).await.ok().and_then(|h| h.revision) else {
        return (None, false);
    };
    let rel = rel.to_string_lossy();
    let dirty = svc.vcs.status(project).await.is_ok_and(|status| {
        status
            .entries
            .iter()
            .any(|e| e.path.starts_with(rel.as_ref()))
    });
    (Some(commit), dirty)
}

/// What differs between two versions of an extension, by id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Changes {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Same id, a different definition.
    pub changed: Vec<String>,
}

impl Changes {
    fn between(
        running: &std::collections::BTreeMap<String, String>,
        source: &std::collections::BTreeMap<String, String>,
    ) -> Changes {
        Changes {
            added: source
                .keys()
                .filter(|k| !running.contains_key(*k))
                .cloned()
                .collect(),
            removed: running
                .keys()
                .filter(|k| !source.contains_key(*k))
                .cloned()
                .collect(),
            changed: source
                .iter()
                .filter(|(k, v)| running.get(*k).is_some_and(|old| old != *v))
                .map(|(k, _)| k.clone())
                .collect(),
        }
    }
}

/// How an extension's source differs from the version that runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceDiff {
    pub name: String,
    /// The hash of the deployment that runs; `None` when none does.
    pub deployed_hash: Option<String>,
    pub source_hash: String,
    pub differs: bool,
    /// Page files, by id.
    pub pages: Changes,
    /// Commands, by name (`changed` is filled too).
    pub commands: Changes,
}

/// An item's definition as text, leaving out where it was loaded from.
fn definition(item: &impl Serialize) -> String {
    let mut value = serde_json::to_value(item).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.remove("origin");
    }
    without_provenance(&mut value);
    value.to_string()
}

/// `value` without where its parts were loaded from: a page's viz nodes
/// each carry the page's provenance (`origin: { running, .. }`), which
/// differs between a source and its deployment though nothing changed.
fn without_provenance(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            if object
                .get("origin")
                .is_some_and(|o| o.get("running").is_some())
            {
                object.remove("origin");
            }
            object.values_mut().for_each(without_provenance);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(without_provenance),
        _ => {}
    }
}

type Definitions = std::collections::BTreeMap<String, String>;

/// An extension's pages and commands, each by id
/// with its definition.
fn definitions(ext: &crate::extension::extensions::Extension) -> (Definitions, Definitions) {
    let pages = ext
        .page_files
        .iter()
        .map(|p| (p.id.clone(), definition(p)))
        .collect();
    let commands = ext
        .commands
        .iter()
        .map(|c| (c.name.clone(), definition(c)))
        .collect();
    (pages, commands)
}

/// What `source` adds, removes and changes over `running` (nothing running:
/// everything is added).
fn compare(
    running: Option<&crate::extension::extensions::Extension>,
    source: &crate::extension::extensions::Extension,
) -> (Changes, Changes) {
    let (old_pages, old_commands) = running.map(definitions).unwrap_or_default();
    let (pages, commands) = definitions(source);
    (
        Changes::between(&old_pages, &pages),
        Changes::between(&old_commands, &commands),
    )
}

fn removal_warning(pages: &Changes, commands: &Changes) -> Option<String> {
    let list = |ids: &[String]| {
        ids.iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    if !pages.removed.is_empty() {
        parts.push(format!("page(s) {}", list(&pages.removed)));
    }
    if !commands.removed.is_empty() {
        parts.push(format!("command(s) {}", list(&commands.removed)));
    }
    (!parts.is_empty()).then(|| {
        format!(
            "deploying removes {} that the running version has",
            parts.join(" and ")
        )
    })
}

/// How `name`'s source in `checkout` differs from what runs: the draft in
/// a stream's worktree against the running version.
pub async fn source_diff(
    svc: &Arc<Services>,
    checkout: &Path,
    name: &str,
) -> Result<SourceDiff, DomainError> {
    valid(name)?;
    let dir = oxplow_config::extension_settings(&svc.layout.project_dir).source(checkout, name);
    if !dir.join("extension.yaml").is_file() {
        return Err(invalid(format!(
            "no source for extension `{name}`: {} has no extension.yaml",
            dir.display()
        )));
    }
    let (hash, loaded) = {
        let (dir, owned) = (dir.clone(), name.to_string());
        tokio::task::spawn_blocking(move || {
            Ok::<_, std::io::Error>((
                extension_store::source_hash(&dir)?,
                crate::extension::extensions::load_extension_at(&dir, &owned),
            ))
        })
        .await
        .map_err(storage)?
        .map_err(|e| invalid(format!("extension `{name}`: {e}")))?
    };
    let running = running(svc, name);
    let deployed_hash = running.as_ref().and_then(|e| match &e.deployment {
        crate::extension::extensions::Deployment::Deployed { hash }
        | crate::extension::extensions::Deployment::Shipped { hash } => Some(hash.clone()),
        _ => None,
    });
    let (pages, commands) = compare(running.as_ref(), &loaded);
    Ok(SourceDiff {
        name: name.to_string(),
        differs: deployed_hash.as_deref() != Some(hash.as_str()),
        deployed_hash,
        source_hash: hash,
        pages,
        commands,
    })
}

/// What writing a page did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PageWritten {
    /// `<extension>/<slug>`.
    pub page: String,
    /// The file, relative to the project when inside it.
    pub path: String,
    /// The deploy that followed, when asked for.
    pub deployed: Option<Deployed>,
}

/// A new, empty dashboard: a page in the Dashboards launcher category whose
/// view is an empty grid (tiles are embeds the app adds), named from
/// `title`, in `extension`'s source, deployed. Refused when that page exists.
pub async fn new_dashboard(
    svc: &Arc<Services>,
    extension: &str,
    title: &str,
    confirmed: bool,
) -> Result<PageWritten, DeployError> {
    let mut slug = String::new();
    for c in title.trim().chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = match slug.chars().next() {
        Some(c) if c.is_ascii_lowercase() => slug.to_string(),
        Some(_) => format!("dashboard-{slug}"),
        None => return Err(invalid("a dashboard needs a title".to_string()).into()),
    };
    let project = svc.layout.project_dir.clone();
    let dir = crate::extension::extensions::writable_extension_dir(&project, &project, extension)?;
    if dir.join("pages").join(format!("{slug}.yaml")).exists() {
        return Err(invalid(format!(
            "`{extension}/{slug}` already exists; open it, or give the new one another title"
        ))
        .into());
    }
    let spec = format!(
        "title: {}\nicon: layout-dashboard\ncategory: Dashboards\nview:\n  grid: []\n",
        serde_json::Value::String(title.trim().to_string())
    );
    write_page(svc, extension, &slug, &spec, true, confirmed).await
}

/// Write `spec` (a page's YAML text) as `pages/<page>.yaml` in `extension`'s
/// source in the primary checkout, creating the extension (a scaffolded
/// manifest) when it is missing and replacing the page when it is there.
/// The text must parse as a page and the extension must still load with
/// it; otherwise nothing stays written. With `deploy_now` it then deploys.
pub async fn write_page(
    svc: &Arc<Services>,
    extension: &str,
    page: &str,
    spec: &str,
    deploy_now: bool,
    confirmed: bool,
) -> Result<PageWritten, DeployError> {
    let project = svc.layout.project_dir.clone();
    let dir = crate::extension::extensions::writable_extension_dir(&project, &project, extension)?;
    if !crate::extension::extensions::is_valid_name(page) {
        return Err(invalid(format!(
            "page `{page}` must start with a letter and be lowercase letters, digits and single dashes"
        ))
        .into());
    }
    let file = dir.join("pages").join(format!("{page}.yaml"));
    let shown = crate::extension::extensions::shown(&project, &file);
    crate::pages::schema::parse_page(
        extension,
        &extension.replace('-', "_"),
        page,
        &shown,
        spec,
        crate::pages::schema::PageKind::Page,
    )
    .map_err(|e| invalid(format!("page `{extension}/{page}` doesn't parse: {e}")))?;
    let manifest = dir.join("extension.yaml");
    let made_manifest = !manifest.exists();
    let before = std::fs::read_to_string(&file).ok();
    let (name, slug, text) = (extension.to_string(), page.to_string(), spec.to_string());
    let (dir_, file_, manifest_) = (dir.clone(), file.clone(), manifest.clone());
    let loaded = tokio::task::spawn_blocking(move || -> std::io::Result<_> {
        std::fs::create_dir_all(file_.parent().unwrap_or(&dir_))?;
        if made_manifest {
            std::fs::write(
                &manifest_,
                crate::extension::extensions::scaffold_manifest(
                    &crate::extension::extensions::ManifestScaffold {
                        name: &name,
                        description: "Pages written from the app",
                        purpose: "Pages saved from answers and dashboards",
                        origin: None,
                        example_name: &slug,
                        example_input: &format!("{{ page: {slug} }}"),
                        example_expect: "the page opens",
                    },
                ),
            )?;
        }
        std::fs::write(&file_, &text)?;
        Ok(crate::extension::extensions::load_extension_at(
            &dir_, &name,
        ))
    })
    .await
    .map_err(storage)?
    .map_err(|e| storage(format!("write page: {e}")))?;
    if !loaded.page_files.iter().any(|p| p.slug == page) {
        match before {
            Some(old) => {
                let _ = std::fs::write(&file, old);
            }
            None => {
                let _ = std::fs::remove_file(&file);
                if made_manifest {
                    let _ = std::fs::remove_file(&manifest);
                }
            }
        }
        let why: Vec<String> = loaded
            .errors
            .iter()
            .map(|e| loaded.shown(e))
            .filter(|e| e.contains(&format!("{page}.yaml")))
            .collect();
        return Err(invalid(format!(
            "the page `{extension}/{page}` doesn't load: {}",
            if why.is_empty() {
                "no reason given".to_string()
            } else {
                why.join("; ")
            }
        ))
        .into());
    }
    let deployed = if deploy_now {
        Some(deploy(svc, extension, confirmed).await?)
    } else {
        None
    };
    Ok(PageWritten {
        page: format!("{extension}/{page}"),
        path: shown,
        deployed,
    })
}

/// An extension this big says so when it deploys: each deployment is a
/// whole copy of its folder.
const BIG_DEPLOYMENT: u64 = 25 * 1024 * 1024;

fn size_warnings(name: &str, bytes: u64) -> Vec<String> {
    if bytes <= BIG_DEPLOYMENT {
        return Vec::new();
    }
    vec![format!(
        "extension `{name}` is {:.1} MiB: every deployment copies its whole folder, so keep build inputs and dependencies out of it",
        bytes as f64 / (1024.0 * 1024.0)
    )]
}

/// Why a deploy, rollback or enable didn't go live.
#[derive(Debug)]
pub enum DeployError {
    /// It adds permissions over what runs now: a person confirms first
    /// (`adds`, one line each), and an agent's becomes a proposal.
    NeedsApproval {
        name: String,
        adds: Vec<String>,
        /// It's turned off, and deploying it turns it on: `adds` is then
        /// everything it would run with, since nothing of it runs now.
        turns_on: bool,
    },
    Failed(DomainError),
}

impl From<DomainError> for DeployError {
    fn from(e: DomainError) -> Self {
        DeployError::Failed(e)
    }
}

impl std::fmt::Display for DeployError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployError::NeedsApproval { name, adds, .. } => write!(
                f,
                "deploying `{name}` adds permissions a person approves first: {}",
                adds.join("; ")
            ),
            DeployError::Failed(e) => e.fmt(f),
        }
    }
}

/// The permissions a bundled extension ships with.
fn shipped_permissions(b: &crate::bundled_extensions::BundledExtension) -> Permissions {
    b.files()
        .iter()
        .find(|(rel, _)| *rel == "extension.yaml")
        .and_then(|(_, bytes)| {
            serde_yaml::from_slice::<crate::extension::extensions::manifest_v2::ManifestV2>(bytes)
                .ok()
        })
        .map(|m| m.permissions)
        .unwrap_or_default()
}

/// The copy of `name` built into oxplow, when this oxplow runs it
/// (`bundled_extensions::shipped`).
fn shipped_of(
    svc: &Services,
    name: &str,
) -> Option<&'static crate::bundled_extensions::BundledExtension> {
    crate::bundled_extensions::shipped(
        name,
        &svc.extension_catalog.settings(&svc.layout.project_dir),
    )
}

/// `name`'s deployment as it runs now, if one does.
fn running(svc: &Services, name: &str) -> Option<crate::extension::extensions::Extension> {
    svc.extension_catalog.active().and_then(|snapshot| {
        snapshot
            .extensions()
            .iter()
            .find(|e| {
                e.name == name
                    && matches!(
                        e.deployment,
                        crate::extension::extensions::Deployment::Deployed { .. }
                            | crate::extension::extensions::Deployment::Shipped { .. }
                    )
            })
            .cloned()
    })
}

/// What `name` may reach as it runs now: the approved set a deploy is
/// compared with. Nothing running is the baseline.
fn active_permissions(svc: &Services, name: &str) -> Permissions {
    running(svc, name)
        .map(|e| e.permissions)
        .or_else(|| shipped_of(svc, name).map(shipped_permissions))
        .unwrap_or_default()
}

/// The build command `name`'s running deployment was approved with.
fn active_build(svc: &Services, name: &str) -> Option<String> {
    running(svc, name).and_then(|e| e.build).map(|b| b.run)
}

/// What a deploy that runs a new or changed build command asks a person:
/// the command runs with their authority (`.context/extensions-deploy.md`
/// "Builds"). `shown` is the source folder as the person knows it.
pub(crate) fn build_line(run: &str, shown: &str) -> String {
    format!("build — runs `{run}` in {shown} with your authority, on every deploy")
}

/// What going from what `name` runs now to one declaring `declared` and
/// built with `build` (from source `shown`) adds.
fn approval_lines(
    svc: &Services,
    name: &str,
    declared: &Permissions,
    build: Option<&str>,
    shown: &str,
) -> Vec<String> {
    let mut adds = declared.added_over(&active_permissions(svc, name));
    if let Some(run) = build {
        if active_build(svc, name).as_deref() != Some(run) {
            adds.push(build_line(run, shown));
        }
    }
    adds
}

/// Going from what `name` runs now to `declared` and `build` may add
/// permissions or a build command: a person confirms that first.
fn approved(
    svc: &Services,
    name: &str,
    declared: &Permissions,
    build: Option<&str>,
    shown: &str,
    confirmed: bool,
) -> Result<(), DeployError> {
    let adds = approval_lines(svc, name, declared, build, shown);
    if adds.is_empty() || confirmed {
        Ok(())
    } else {
        Err(DeployError::NeedsApproval {
            name: name.to_string(),
            adds,
            turns_on: false,
        })
    }
}

/// `name` as the active generation knows it, on or off.
fn active_entry(svc: &Services, name: &str) -> Option<crate::extension::extensions::Extension> {
    svc.extension_catalog.active().and_then(|snapshot| {
        snapshot
            .extensions()
            .iter()
            .find(|e| e.name == name)
            .cloned()
    })
}

/// Each extension `name` requires is on (`.context/extensions-deploy.md`
/// "Requires").
fn requirements_met(
    settings: &ExtensionSettings,
    name: &str,
    requires: &[String],
) -> Result<(), DomainError> {
    match requires
        .iter()
        .find(|r| !settings.is_enabled(r) && !crate::bundled_extensions::is_required(r))
    {
        Some(r) => Err(invalid(format!(
            "`{name}` requires `{r}`, which is off (turn it on first)"
        ))),
        None => Ok(()),
    }
}

/// No enabled extension requires `name`: it may go off.
fn not_required(svc: &Services, name: &str) -> Result<(), DomainError> {
    let by: Vec<String> = svc
        .extension_catalog
        .active()
        .map(|snapshot| {
            snapshot
                .extensions()
                .iter()
                .filter(|e| e.enabled && e.name != name && e.requires.iter().any(|r| r == name))
                .map(|e| format!("`{}`", e.name))
                .collect()
        })
        .unwrap_or_default();
    if by.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{name}` is required by {}; turn that off first",
            by.join(", ")
        )))
    }
}

fn invalid(message: impl Into<String>) -> DomainError {
    DomainError::Invalid(message.into())
}

fn storage(error: impl std::fmt::Display) -> DomainError {
    DomainError::Storage(error.to_string())
}

fn valid(name: &str) -> Result<(), DomainError> {
    if crate::extension::extensions::is_valid_name(name) {
        Ok(())
    } else {
        Err(invalid(format!(
            "`{name}` isn't an extension name: lowercase letters, digits and single dashes, \
             starting with a letter"
        )))
    }
}

/// Activate `changes`, staging again when a change made meanwhile (a
/// config reload, a provider's restart) overtook the candidate.
async fn activate(svc: &Arc<Services>, changes: &LiveChanges) -> Result<(), DomainError> {
    let mut attempts = 0;
    loop {
        attempts += 1;
        match svc
            .extension_activation
            .activate_changes(svc.clone(), changes)
            .await
        {
            Ok(_) => {
                after_activation(svc).await;
                return Ok(());
            }
            Err(TxError::Aborted(problems)) if problems.contains_key("<host>") && attempts < 3 => {}
            Err(error) => return Err(problems_of(svc, error)),
        }
    }
}

/// A refusal as text: `<extension>: <line>` per problem, each shown under
/// the folder of the extension it belongs to.
fn problems_of(svc: &Services, error: TxError<VocabularyErrors>) -> DomainError {
    match error {
        TxError::Aborted(problems) => invalid(
            crate::vocabulary_reactor::render_problems(
                &problems,
                svc.extension_catalog
                    .snapshot(&svc.layout.project_dir)
                    .extensions(),
            )
            .join("\n"),
        ),
        TxError::Storage(error) => error,
    }
}

/// Where `name`'s one deployable source is: in the primary checkout (the
/// project's), checked to hold a manifest. Another stream's checkout holds
/// a draft of it, never deployed ([`refuse_draft_deploy`]).
fn source_of(project: &Path, name: &str) -> Result<PathBuf, DomainError> {
    let dir = oxplow_config::extension_settings(project).source(project, name);
    if dir.join("extension.yaml").is_file() {
        Ok(dir)
    } else {
        Err(invalid(format!(
            "no source for extension `{name}`: {} has no extension.yaml",
            dir.display()
        )))
    }
}

/// Refuse a deploy asked from a stream other than the primary when
/// `name`'s source lies inside the project: that stream's checkout holds a
/// draft of it, which is checked, previewed and tested there, and deploys
/// from the primary once the change is on its branch. A source outside
/// the project (`~/` or an absolute `extensionSources` entry) has one copy,
/// so any stream may deploy it. `stream` is the asking stream, `None` for
/// a person or a caller with none.
pub async fn refuse_draft_deploy(
    svc: &Services,
    stream: Option<oxplow_domain::StreamId>,
    name: &str,
) -> Result<(), DomainError> {
    use oxplow_domain::stores::StreamStore as _;
    let Some(stream) = stream else {
        return Ok(());
    };
    let project = &svc.layout.project_dir;
    let source = oxplow_config::extension_settings(project).source(project, name);
    if !source.starts_with(project) {
        return Ok(());
    }
    let Some(asking) = svc.stream_store.get(&stream).await? else {
        return Ok(());
    };
    if asking.kind == oxplow_domain::StreamKind::Primary {
        return Ok(());
    }
    let main_branch = svc
        .stream_store
        .primary()
        .await?
        .map(|p| p.branch)
        .unwrap_or_default();
    Err(invalid(format!(
        "`{name}` deploys only from the main stream ({}, on `{main_branch}`); this thread's \
         stream `{stream}` (on `{}`) holds a draft of it. Check it with \
         `oxplow_validate_extension` and test it with `$OXPLOW_BIN extension test {name}` there. \
         It deploys once the change is on `{main_branch}`: from a main-stream thread, or \
         Settings → Extensions.",
        project.display(),
        asking.branch,
    )))
}

/// The live-set change that runs `hash`: none when it is the hash of the
/// copy that ships (`shipped`, when this oxplow runs one).
fn target_of(
    shipped: Option<&crate::bundled_extensions::BundledExtension>,
    hash: &str,
) -> Option<String> {
    let shipped = shipped.map(extension_store::shipped_hash);
    (shipped.as_deref() != Some(hash)).then(|| hash.to_string())
}

/// Deploy `name` from its one deployable source, in the primary checkout
/// (uncommitted changes included). Turns it on in the person's layer when
/// nothing turns it on yet. A source that declares permissions beyond what
/// runs now goes live only `confirmed` by a person; otherwise nothing
/// changes and it says what it would add.
pub async fn deploy(
    svc: &Arc<Services>,
    name: &str,
    confirmed: bool,
) -> Result<Deployed, DeployError> {
    valid(name)?;
    let project = svc.layout.project_dir.clone();
    let source = source_of(&project, name)?;
    let (commit, dirty) = provenance(svc, &project, &source).await;
    let (hash, size, mut loaded) = {
        let (source, owned) = (source.clone(), name.to_string());
        tokio::task::spawn_blocking(move || {
            let hash = extension_store::source_hash(&source)?;
            let size = extension_store::source_size(&source)?;
            Ok::<_, std::io::Error>((
                hash,
                size,
                crate::extension::extensions::load_extension_at(&source, &owned),
            ))
        })
        .await
        .map_err(storage)?
        .map_err(|e| invalid(format!("extension `{name}`: {e}")))?
    };
    // What it reports is shown under the source folder as the project
    // names it, not the absolute path it was read from.
    loaded.source = Some(crate::extension::extensions::shown(&project, &source));
    let target = target_of(shipped_of(svc, name), &hash);
    // Already the one that runs: chosen, and not held (a held deployment
    // stays chosen; deploying it again activates it again).
    let live_now = svc.extension_catalog.live().get(name) == target.as_ref()
        && running(svc, name).is_some_and(|e| match (&e.deployment, &target) {
            (crate::extension::extensions::Deployment::Deployed { hash }, Some(t)) => hash == t,
            (crate::extension::extensions::Deployment::Shipped { .. }, None) => true,
            _ => false,
        });
    requirements_met(
        &oxplow_config::extension_settings(&project),
        name,
        &loaded.requires,
    )?;
    if !confirmed && turned_off(svc, name) {
        let shown = crate::extension::extensions::shown(&project, &source);
        let mut adds = loaded.permissions.added_over(&Permissions::default());
        if let Some(build) = &loaded.build {
            adds.push(build_line(&build.run, &shown));
        }
        return Err(DeployError::NeedsApproval {
            name: name.to_string(),
            adds,
            turns_on: true,
        });
    }
    if !live_now {
        if !loaded.errors.is_empty() {
            return Err(invalid(format!(
                "extension `{name}` doesn't load:\n{}",
                loaded
                    .errors
                    .iter()
                    .map(|d| loaded.shown(d))
                    .collect::<Vec<_>>()
                    .join("\n")
            ))
            .into());
        }
        approved(
            svc,
            name,
            &loaded.permissions,
            loaded.build.as_ref().map(|b| b.run.as_str()),
            &crate::extension::extensions::shown(&project, &source),
            confirmed,
        )?;
    }
    let removal = running(svc, name).and_then(|now| {
        let (pages, commands) = compare(Some(&now), &loaded);
        removal_warning(&pages, &commands)
    });
    let enabled_now = turn_on(svc, name)?;
    if live_now {
        return Ok(Deployed {
            name: name.to_string(),
            hash,
            already: true,
            warnings: loaded.warnings.iter().map(|w| loaded.shown(w)).collect(),
            commit,
            dirty,
        });
    }
    if target.is_some() {
        let built_before = extension_store::deployment_dir(&project, name, &hash).is_dir();
        if let (Some(build), false) = (&loaded.build, built_before) {
            let (dir, run, env) = (source.clone(), build.run.clone(), svc.providers.host_env());
            let ran = tokio::task::spawn_blocking(move || {
                crate::extension::extension_build::run(
                    &dir,
                    &run,
                    &env,
                    crate::extension::extension_build::TIMEOUT,
                )
            })
            .await
            .map_err(storage)?;
            let ran = ran.and_then(|()| {
                if source.join(&build.output).is_dir() {
                    Ok(())
                } else {
                    Err(format!("`{}` didn't write `{}`", build.run, build.output))
                }
            });
            if let Err(e) = ran {
                undo_turn_on(svc, name, enabled_now);
                return Err(invalid(format!("building `{name}`: {e}")).into());
            }
        }
        let copied = {
            let (project, source, name, hash) = (
                project.clone(),
                source.clone(),
                name.to_string(),
                hash.clone(),
            );
            tokio::task::spawn_blocking(move || {
                extension_store::copy_into_store(&project, &name, &source, &hash)
            })
            .await
            .map_err(storage)?
        };
        let deployed_dir = match copied {
            Ok(dir) => dir,
            Err(e) => {
                undo_turn_on(svc, name, enabled_now);
                return Err(storage(format!("deploy `{name}`: {e}")).into());
            }
        };
        if loaded.build.is_some() && !built_before {
            // What the build wrote is checked where it runs from.
            let (dir, owned) = (deployed_dir.clone(), name.to_string());
            let built = tokio::task::spawn_blocking(move || {
                crate::extension::extensions::load_extension_at(&dir, &owned)
            })
            .await
            .map_err(storage)?;
            if !built.errors.is_empty() {
                extension_store::remove(&project, name, std::slice::from_ref(&hash));
                undo_turn_on(svc, name, enabled_now);
                return Err(invalid(format!(
                    "extension `{name}` doesn't load once built:\n{}",
                    // Under the source folder, the one to edit, not the
                    // deployed copy.
                    built
                        .errors
                        .iter()
                        .map(|d| loaded.shown(d))
                        .collect::<Vec<_>>()
                        .join("\n")
                ))
                .into());
            }
        }
        let row = DeploymentRow {
            name: name.to_string(),
            hash: hash.clone(),
            deployed_at: Timestamp::now().to_string(),
            commit: commit.clone(),
            dirty,
            source: source.to_string_lossy().into_owned(),
            active: false,
            permissions: loaded.permissions.clone(),
            build_run: loaded.build.as_ref().map(|b| b.run.clone()),
        };
        svc.db
            .transaction(move |tx| store_rows::record_tx(tx, &row))
            .await?;
    }
    let changes = LiveChanges::from([(name.to_string(), target)]);
    if let Err(error) = activate(svc, &changes).await {
        undo_turn_on(svc, name, enabled_now);
        return Err(error.into());
    }
    prune(svc, name).await?;
    Ok(Deployed {
        name: name.to_string(),
        hash,
        already: false,
        warnings: [
            loaded.warnings.iter().map(|w| loaded.shown(w)).collect(),
            size_warnings(name, size),
            removal.into_iter().collect(),
        ]
        .concat(),
        commit,
        dirty,
    })
}

/// Make an earlier deployment of `name` active again: `to` (a hash or the
/// start of one), else the one deployed before the active one. A bundled
/// extension with nothing older returns to the deployment that ships.
pub async fn rollback(
    svc: &Arc<Services>,
    name: &str,
    to: Option<&str>,
    confirmed: bool,
) -> Result<Deployed, DeployError> {
    valid(name)?;
    let rows = {
        let name = name.to_string();
        svc.db
            .read(move |c| store_rows::list_tx(c, Some(&name)))
            .await?
    };
    let live = svc.extension_catalog.live().get(name).cloned();
    let hash = match to {
        Some(prefix) => {
            let matching: Vec<&DeploymentRow> =
                rows.iter().filter(|r| r.hash.starts_with(prefix)).collect();
            match matching.as_slice() {
                [one] => Some(one.hash.clone()),
                [] => match shipped_of(svc, name) {
                    Some(b) if extension_store::shipped_hash(b).starts_with(prefix) => None,
                    _ => {
                        return Err(invalid(format!("`{name}` has no deployment `{prefix}`")).into())
                    }
                },
                _ => {
                    return Err(invalid(format!(
                        "`{prefix}` names more than one deployment of `{name}`"
                    ))
                    .into())
                }
            }
        }
        None => {
            let previous = rows
                .iter()
                .skip_while(|r| Some(&r.hash) != live.as_ref())
                .nth(1)
                .or_else(|| rows.iter().find(|r| Some(&r.hash) != live.as_ref()));
            match (previous, shipped_of(svc, name)) {
                (Some(row), _) => Some(row.hash.clone()),
                (None, Some(_)) if live.is_some() => None,
                _ => {
                    return Err(invalid(format!(
                        "`{name}` has no earlier deployment to roll back to"
                    ))
                    .into())
                }
            }
        }
    };
    if let Some(hash) = &hash {
        if !extension_store::deployment_dir(&svc.layout.project_dir, name, hash).is_dir() {
            return Err(invalid(format!(
                "deployment `{hash}` of `{name}` is no longer in the store"
            ))
            .into());
        }
    }
    let shown = hash.clone().unwrap_or_else(|| {
        shipped_of(svc, name)
            .map(extension_store::shipped_hash)
            .unwrap_or_default()
    });
    let target_row = hash
        .as_ref()
        .and_then(|hash| rows.iter().find(|r| &r.hash == hash));
    let (commit, dirty) = target_row
        .map(|r| (r.commit.clone(), r.dirty))
        .unwrap_or_default();
    if live == hash {
        return Ok(Deployed {
            name: name.to_string(),
            hash: shown,
            already: true,
            warnings: Vec::new(),
            commit,
            dirty,
        });
    }
    let target_permissions = match &hash {
        Some(_) => target_row
            .map(|r| r.permissions.clone())
            .unwrap_or_default(),
        None => shipped_of(svc, name)
            .map(shipped_permissions)
            .unwrap_or_default(),
    };
    let shown_source = target_row
        .map(|r| crate::extension::extensions::shown(&svc.layout.project_dir, Path::new(&r.source)))
        .unwrap_or_default();
    approved(
        svc,
        name,
        &target_permissions,
        target_row.and_then(|r| r.build_run.as_deref()),
        &shown_source,
        confirmed,
    )?;
    activate(svc, &LiveChanges::from([(name.to_string(), hash)])).await?;
    Ok(Deployed {
        name: name.to_string(),
        hash: shown,
        already: false,
        warnings: Vec::new(),
        commit,
        dirty,
    })
}

/// What follows a publication the activation owner didn't make itself.
async fn after_activation(svc: &Arc<Services>) {
    let _ = svc
        .sql
        .read_generation(svc.metrics.capture_entity_states(true))
        .await;
    crate::capabilities::refresh_agent_text(svc).await;
}

/// Forget the deployments of `name` past the active one and the newest
/// [`extension_store::KEEP`].
async fn prune(svc: &Arc<Services>, name: &str) -> Result<(), DomainError> {
    let rows = {
        let name = name.to_string();
        svc.db
            .read(move |c| store_rows::list_tx(c, Some(&name)))
            .await?
    };
    let beyond = extension_store::beyond_kept(
        &rows
            .iter()
            .map(|r| (r.hash.clone(), r.active))
            .collect::<Vec<_>>(),
    );
    if beyond.is_empty() {
        return Ok(());
    }
    extension_store::remove(&svc.layout.project_dir, name, &beyond);
    let name = name.to_string();
    svc.db
        .transaction(move |tx| store_rows::forget_tx(tx, &name, Some(&beyond)))
        .await
}

// --- the enabled list ---------------------------------------------------

fn personal_list(names: &[String]) -> Option<serde_json::Value> {
    (!names.is_empty()).then(|| serde_json::json!(names))
}

/// Write the person's lists, then reload the config.
fn write_personal(svc: &Services, settings: &ExtensionSettings) -> Result<(), DomainError> {
    let project = &svc.layout.project_dir;
    for (key, names) in [
        ("enableExtensions", &settings.personal.enable),
        ("disableExtensions", &settings.personal.disable),
    ] {
        oxplow_config::write_personal_key(project, key, personal_list(names).as_ref())
            .map_err(storage)?;
    }
    svc.reload_config_from_disk().map_err(storage)
}

/// `name` is off because someone turned it off, as opposed to a new one no
/// layer has heard of yet: on the person's disable list, or off with a
/// deployment still chosen for it.
fn turned_off(svc: &Services, name: &str) -> bool {
    let settings = oxplow_config::extension_settings(&svc.layout.project_dir);
    !settings.is_enabled(name)
        && !crate::bundled_extensions::is_required(name)
        && (settings.personal.disable.iter().any(|n| n == name)
            || svc.extension_catalog.live().contains_key(name))
}

/// Turn `name` on in the person's layer unless it is on already. True when
/// it wrote something (a failed deploy takes it back).
fn turn_on(svc: &Services, name: &str) -> Result<bool, DomainError> {
    let mut settings = oxplow_config::extension_settings(&svc.layout.project_dir);
    if settings.is_enabled(name) || crate::bundled_extensions::is_required(name) {
        return Ok(false);
    }
    settings.personal.disable.retain(|n| n != name);
    if !settings.is_enabled(name) {
        settings.personal.enable.push(name.to_string());
    }
    write_personal(svc, &settings)?;
    Ok(true)
}

fn undo_turn_on(svc: &Services, name: &str, wrote: bool) {
    if wrote {
        let mut settings = oxplow_config::extension_settings(&svc.layout.project_dir);
        settings.personal.enable.retain(|n| n != name);
        let _ = write_personal(svc, &settings);
    }
}

/// Turn `name` on (`true`) or off for this person, then activate. Turning
/// on one with no deployment here deploys it from the main checkout.
pub async fn set_enabled(
    svc: &Arc<Services>,
    name: &str,
    enabled: bool,
    confirmed: bool,
) -> Result<(), DeployError> {
    valid(name)?;
    if crate::bundled_extensions::is_required(name) {
        return Err(invalid(format!("`{name}` is part of oxplow; it is always on")).into());
    }
    let mut settings = oxplow_config::extension_settings(&svc.layout.project_dir);
    if enabled {
        if let Some(running) = active_entry(svc, name) {
            requirements_met(&settings, name, &running.requires)?;
        }
    } else {
        not_required(svc, name)?;
    }
    if enabled {
        settings.personal.disable.retain(|n| n != name);
        if !settings.is_enabled(name) {
            settings.personal.enable.push(name.to_string());
        }
    } else {
        settings.personal.enable.retain(|n| n != name);
        if settings.is_enabled(name) {
            settings.personal.disable.push(name.to_string());
        }
    }
    write_personal(svc, &settings)?;
    let undeployed = enabled
        && shipped_of(svc, name).is_none()
        && !svc.extension_catalog.live().contains_key(name);
    if undeployed {
        deploy(svc, name, confirmed).await?;
        return Ok(());
    }
    Ok(activate(svc, &LiveChanges::new()).await?)
}

/// Take `name` out of both lists, stop running it, and forget its
/// deployments and its stored state and settings. Its source stays.
pub async fn remove(svc: &Arc<Services>, name: &str) -> Result<(), DomainError> {
    valid(name)?;
    if crate::bundled_extensions::is_required(name) {
        return Err(invalid(format!(
            "`{name}` is part of oxplow; it can't be removed"
        )));
    }
    if shipped_of(svc, name).is_some() {
        return Err(invalid(format!(
            "`{name}` ships with oxplow; turn it off instead"
        )));
    }
    not_required(svc, name)?;
    let project = svc.layout.project_dir.clone();
    let mut settings = oxplow_config::extension_settings(&project);
    settings.personal.enable.retain(|n| n != name);
    settings.personal.disable.retain(|n| n != name);
    write_personal(svc, &settings)?;
    if settings.team.iter().any(|n| n == name) {
        let team: Vec<String> = settings
            .team
            .iter()
            .filter(|n| *n != name)
            .cloned()
            .collect();
        write_team(svc, &team)?;
    }
    activate(svc, &LiveChanges::from([(name.to_string(), None)])).await?;
    extension_store::remove_all(&project, name);
    let name = name.to_string();
    svc.db
        .transaction(move |tx| {
            store_rows::forget_tx(tx, &name, None)?;
            oxplow_db::extension_state_store::forget_extension_tx(tx, &name)?;
            oxplow_db::extension_config_store::forget_extension_tx(tx, &name)
        })
        .await
}

fn write_team(svc: &Services, team: &[String]) -> Result<(), DomainError> {
    oxplow_config::write_project_key(
        &svc.layout.project_dir,
        "extensions",
        Some(&serde_json::json!(team)),
    )
    .map_err(storage)?;
    svc.reload_config_from_disk().map_err(storage)
}

// --- sharing ------------------------------------------------------------

/// The lines of `.oxplow/.gitignore` that let git see `name`'s source:
/// the folder above it un-ignored but not its other entries, then its own
/// folder and, since `*` matches a file at any depth, everything in it —
/// but a build's output and excluded folders (`inputs`), which aren't
/// source.
fn share_lines(name: &str, inputs: &extension_store::Inputs) -> Vec<String> {
    let mut lines = vec![
        "!extensions/".to_string(),
        "extensions/*".to_string(),
        format!("!extensions/{name}/"),
        format!("!extensions/{name}/**"),
    ];
    for dir in inputs.output.iter().chain(&inputs.exclude) {
        lines.push(format!("extensions/{name}/{}/", dir.trim_end_matches('/')));
    }
    lines
}

/// Commit `name` for the team: let git see its source (`.oxplow/.gitignore`)
/// and move it from the person's list to the team's (`project.yaml`). Only
/// a source in `.oxplow/extensions/` is shared this way; one elsewhere is
/// the person's to commit.
pub fn share(svc: &Services, name: &str) -> Result<(), DomainError> {
    valid(name)?;
    let project = &svc.layout.project_dir;
    let mut settings = oxplow_config::extension_settings(project);
    let source = settings.source(project, name);
    if source
        != project
            .join(crate::extension::extensions::EXTENSIONS_DIR)
            .join(name)
    {
        return Err(invalid(format!(
            "`{name}`'s source is {}, outside .oxplow/extensions: commit it where it is",
            source.display()
        )));
    }
    let ignore = project.join(".oxplow/.gitignore");
    let text = std::fs::read_to_string(&ignore).unwrap_or_default();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for line in share_lines(name, &extension_store::Inputs::of(&source)) {
        if !lines.contains(&line) {
            lines.push(line);
        }
    }
    std::fs::write(&ignore, lines.join("\n") + "\n").map_err(storage)?;
    if !settings.team.iter().any(|n| n == name) {
        let mut team = settings.team.clone();
        team.push(name.to_string());
        write_team(svc, &team)?;
    }
    settings.personal.enable.retain(|n| n != name);
    write_personal(svc, &settings)
}

/// Undo [`share`]: git stops seeing `name`'s source and it moves back to
/// the person's list.
pub fn unshare(svc: &Services, name: &str) -> Result<(), DomainError> {
    valid(name)?;
    let project = &svc.layout.project_dir;
    let ignore = project.join(".oxplow/.gitignore");
    // Every line naming its folder, whatever its manifest says now.
    let (folder, files, inside) = (
        format!("!extensions/{name}/"),
        format!("!extensions/{name}/**"),
        format!("extensions/{name}/"),
    );
    let text = std::fs::read_to_string(&ignore).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| *l != folder && *l != files && !l.starts_with(&inside))
        .collect();
    std::fs::write(&ignore, lines.join("\n") + "\n").map_err(storage)?;
    let mut settings = oxplow_config::extension_settings(project);
    if settings.team.iter().any(|n| n == name) {
        let team: Vec<String> = settings
            .team
            .iter()
            .filter(|n| *n != name)
            .cloned()
            .collect();
        write_team(svc, &team)?;
    }
    if !settings.personal.enable.iter().any(|n| n == name) {
        settings.personal.enable.push(name.to_string());
    }
    write_personal(svc, &settings)
}

// --- startup ------------------------------------------------------------

/// Copy `name`'s source at `source` into the store (unless it is there, or
/// it is the deployment that ships) and record it: the live-set change
/// that runs it, for tests that stage sources as a deploy would. No load
/// check: activation holds one that doesn't load.
#[cfg(test)]
async fn stage(
    svc: &Arc<Services>,
    name: &str,
    source: &Path,
    permissions: Permissions,
    build_run: Option<String>,
) -> Result<Option<String>, DomainError> {
    let shipped = shipped_of(svc, name);
    let hash = {
        let (project, owned, source) = (
            svc.layout.project_dir.clone(),
            name.to_string(),
            source.to_path_buf(),
        );
        tokio::task::spawn_blocking(move || {
            let hash = extension_store::source_hash(&source)?;
            if target_of(shipped, &hash).is_some() {
                extension_store::copy_into_store(&project, &owned, &source, &hash)?;
            }
            Ok::<_, std::io::Error>(hash)
        })
        .await
        .map_err(storage)?
        .map_err(|e| storage(format!("stage `{name}`: {e}")))?
    };
    let Some(target) = target_of(shipped, &hash) else {
        return Ok(None);
    };
    let row = DeploymentRow {
        name: name.to_string(),
        hash: target.clone(),
        deployed_at: Timestamp::now().to_string(),
        commit: None,
        dirty: false,
        source: source.to_string_lossy().into_owned(),
        active: false,
        permissions,
        build_run,
    };
    svc.db
        .transaction(move |tx| store_rows::record_tx(tx, &row))
        .await?;
    Ok(Some(target))
}

/// The command that deploys a checkout's extensions from outside oxplow,
/// with oxplow quit: what a problem says when oxplow's own extension has
/// nothing to run (`oxplow-dev`, never shipped).
pub const DEPLOY_FROM_OUTSIDE: &str = "cargo run -q -p oxplow-dev -- extension deploy";

/// Before the first activation: start the live set from the deployments
/// recorded as active whose folder is still in the store. Boot deploys
/// nothing: an enabled extension with no deployment here stays undeployed
/// until a person deploys it, so what runs is only ever what someone
/// deployed. A required extension with nothing to run (no deployment, and
/// no copy ships: oxplow developing itself) is a problem, returned for the
/// first activation to list; it isn't held, it simply isn't there.
pub(crate) async fn start(svc: &Arc<Services>) -> Result<VocabularyErrors, DomainError> {
    let project = svc.layout.project_dir.clone();
    let rows = svc.db.read(|c| store_rows::list_tx(c, None)).await?;
    let live = rows
        .into_iter()
        .filter(|r| {
            r.active && extension_store::deployment_dir(&project, &r.name, &r.hash).is_dir()
        })
        .map(|r| (r.name, r.hash))
        .collect();
    svc.extension_catalog.set_live(live);
    let mut missing = VocabularyErrors::new();
    for b in crate::bundled_extensions::BUNDLED
        .iter()
        .filter(|b| b.required)
    {
        if svc.extension_catalog.live().contains_key(b.name) || shipped_of(svc, b.name).is_some() {
            continue;
        }
        missing.insert(
            b.name.to_string(),
            vec![Diagnostic::error(
                "activation.required_not_deployed",
                format!(
                    "oxplow's own `{name}` has no deployment here and no copy of it ships with \
                     this build, so what it provides (the agent harnesses, Alerts, core \
                     commands and pages) is missing",
                    name = b.name
                ),
            )
            .fix(format!(
                "quit oxplow, run `{DEPLOY_FROM_OUTSIDE}` in the project, then start oxplow again"
            ))],
        );
    }
    Ok(missing)
}

/// What the next boot of a project would run, found without starting it.
pub struct NextBoot {
    /// Each extension that would be on, and what of it would run (a
    /// deployment, the copy built in, or nothing), saying when its source
    /// has changed since that deployment.
    pub runs: Vec<(String, String)>,
    /// What its activation would hold back, and why.
    pub problems: VocabularyErrors,
}

/// What the next boot of `project` would run and hold back, without
/// touching it: its settings and active deployments (`rows`, as recorded)
/// are activated in a throwaway oxplow in memory, as boot activates them
/// (`start`, then the first generation), over copies of those
/// deployments and the sources where they are. So a deployment an older
/// oxplow recorded that no longer activates against this one is found
/// before anyone starts it (`oxplow-dev extension status`).
pub async fn next_boot(project: &Path, rows: &[DeploymentRow]) -> Result<NextBoot, String> {
    let settings = oxplow_config::extension_settings(project);
    let mut names: Vec<String> = settings.enabled().into_iter().map(|(n, _)| n).collect();
    for b in crate::bundled_extensions::BUNDLED
        .iter()
        .filter(|b| b.required)
    {
        if !names.iter().any(|n| n == b.name) {
            names.push(b.name.to_string());
        }
    }
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let root = dir.path().to_path_buf();
    crate::vcs::GitProvider
        .init_repository(&root)
        .await
        .map_err(|e| e.to_string())?;
    // The settings as they are, each source where it is.
    let sources: std::collections::BTreeMap<String, String> = names
        .iter()
        .map(|n| (n.clone(), settings.source(project, n).display().to_string()))
        .collect();
    let mut config =
        serde_json::json!({ "extensions": settings.team, "extensionSources": sources });
    if let Some(requested) = settings.bundled_requested {
        config["enableBundledExtensions"] = requested.into();
    }
    let personal = serde_json::json!({
        "enableExtensions": settings.personal.enable,
        "disableExtensions": settings.personal.disable,
    });
    std::fs::create_dir_all(root.join(".oxplow")).map_err(|e| e.to_string())?;
    for (file, value) in [("project.yaml", config), ("personal.yaml", personal)] {
        let yaml = serde_yaml::to_string(&value).map_err(|e| e.to_string())?;
        std::fs::write(root.join(".oxplow").join(file), yaml).map_err(|e| e.to_string())?;
    }
    let svc = Services::on_machine_inactive(
        &root,
        root.join(".global"),
        Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
        Arc::new(|_: &str| None),
        oxplow_db::Database::in_memory(),
    )
    .map_err(|e| e.to_string())?;
    // The store as boot finds it: each active deployment still there.
    let active: Vec<DeploymentRow> = rows
        .iter()
        .filter(|r| r.active && extension_store::deployment_dir(project, &r.name, &r.hash).is_dir())
        .cloned()
        .collect();
    for row in &active {
        let from = extension_store::deployment_dir(project, &row.name, &row.hash);
        extension_store::copy_into_store(&root, &row.name, &from, &row.hash)
            .map_err(|e| format!("copy `{}`'s deployment {}: {e}", row.name, row.hash))?;
    }
    let live: Vec<(String, String)> = active
        .iter()
        .map(|r| (r.name.clone(), r.hash.clone()))
        .collect();
    svc.db
        .transaction(move |tx| {
            for row in &active {
                store_rows::record_tx(tx, row)?;
            }
            store_rows::set_active_tx(tx, &live)
        })
        .await
        .map_err(|e| e.to_string())?;
    svc.extension_activation
        .initialize(svc.clone())
        .await
        .map_err(|e| e.to_string())?;
    let running = svc.extension_catalog.active();
    let runs = names
        .into_iter()
        .map(|name| {
            let entry = running.as_ref().and_then(|snapshot| {
                snapshot
                    .extensions()
                    .iter()
                    .find(|e| e.name == name && e.enabled)
                    .cloned()
            });
            let deployed = rows.iter().find(|r| r.active && r.name == name);
            let what = match entry.map(|e| e.deployment) {
                Some(crate::extension::extensions::Deployment::Deployed { hash }) => {
                    let at = rows
                        .iter()
                        .find(|r| r.name == name && r.hash == hash)
                        .map(|r| {
                            let from = provenance_text(r.commit.as_deref(), r.dirty)
                                .map(|p| format!(", {p}"))
                                .unwrap_or_default();
                            format!(" of {}{from}", r.deployed_at)
                        })
                        .unwrap_or_default();
                    format!("its deployment {hash}{at}")
                }
                Some(crate::extension::extensions::Deployment::Shipped { .. }) => {
                    "the copy built into oxplow".to_string()
                }
                Some(_) => "its source".to_string(),
                None => match deployed {
                    Some(row) => format!("nothing (its deployment {} doesn't activate)", row.hash),
                    None => "nothing".to_string(),
                },
            };
            let source = settings.source(project, &name);
            let moved_on = deployed.is_some_and(|row| {
                extension_store::source_hash(&source).is_ok_and(|now| now != row.hash)
            });
            let what = if moved_on {
                format!("{what}; its source has changed since it was deployed")
            } else {
                what
            };
            (name, what)
        })
        .collect();
    Ok(NextBoot {
        runs,
        problems: svc.extension_activation.problems(),
    })
}

/// For tests that write an extension's files and activate, as an agent
/// that wrote them would deploy them: every source in the project whose
/// deployment isn't the live one is staged, turned on for the person
/// unless something turned it off, and activated with the rest — one that
/// doesn't load is held, as at boot.
#[cfg(test)]
pub(crate) async fn stage_sources(svc: &Arc<Services>) -> LiveChanges {
    let project = svc.layout.project_dir.clone();
    let mut settings = oxplow_config::extension_settings(&project);
    let mut changes = LiveChanges::new();
    let mut turned_on = false;
    for name in crate::extension::extensions::known_names(&project, &settings) {
        let source = settings.source(&project, &name);
        if !source.join("extension.yaml").is_file() || settings.personal.disable.contains(&name) {
            continue;
        }
        let declared = crate::extension::extensions::load_extension_at(&source, &name).permissions;
        let target = stage(svc, &name, &source, declared, None)
            .await
            .expect("a source stages");
        if svc.extension_catalog.live().get(&name) != target.as_ref() {
            changes.insert(name.clone(), target);
        }
        if !settings.is_enabled(&name) && !crate::bundled_extensions::is_reserved(&name) {
            settings.personal.enable.push(name);
            turned_on = true;
        }
    }
    if turned_on {
        write_personal(svc, &settings).expect("the person's layer is written");
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::extensions::Deployment;
    use oxplow_domain::Actor;
    use serde_json::json;

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// `acme`'s source in the project, with one page titled `title`.
    fn source(svc: &Services, title: &str) -> PathBuf {
        let dir = svc.layout.project_dir.join(".oxplow/extensions/acme");
        write(
            &dir,
            "extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        );
        write(
            &dir,
            "pages/one.yaml",
            &format!(
                "title: {title}\nloaders:\n  n: {{ sql: \"SELECT 1 AS n\" }}\nview: {{ viz: number, from: n }}\n"
            ),
        );
        dir
    }

    fn page_title(svc: &Services) -> Option<String> {
        svc.extension_catalog
            .find_page(&svc.layout.project_dir, "acme/one")
            .ok()
            .map(|l| l.title)
    }

    fn acme(svc: &Services) -> Option<crate::extension::extensions::Extension> {
        svc.extension_catalog
            .unpinned(&svc.layout.project_dir, "a test reads the catalog")
            .iter()
            .find(|e| e.name == "acme")
            .cloned()
    }

    async fn rows(svc: &Services) -> Vec<(String, bool)> {
        svc.db
            .read(|c| store_rows::list_tx(c, Some("acme")))
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.hash, r.active))
            .collect()
    }

    /// The hashes of `name`'s deployments recorded active.
    async fn active_hashes(svc: &Services, name: &str) -> Vec<String> {
        let name = name.to_string();
        svc.db
            .read(move |c| store_rows::list_tx(c, Some(&name)))
            .await
            .unwrap()
            .into_iter()
            .filter(|r| r.active)
            .map(|r| r.hash)
            .collect()
    }

    /// `acme`'s source declaring `permissions`, its page titled `title`.
    fn source_with(svc: &Services, title: &str, permissions: &str) -> PathBuf {
        let dir = source(svc, title);
        write(
            &dir,
            "extension.yaml",
            &format!("manifest: 2\nname: acme\n{permissions}intent:\n  purpose: test\n"),
        );
        dir
    }

    const AI: &str = "permissions: [ai.compute]\n";

    fn asked(result: Result<Deployed, DeployError>) -> Vec<String> {
        match result {
            Err(DeployError::NeedsApproval { adds, .. }) => adds,
            other => panic!("expected it to ask, got {other:?}"),
        }
    }

    /// A deploy that reaches no further than what runs goes live at once.
    #[tokio::test]
    async fn a_deploy_within_the_active_permissions_goes_live_at_once() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source_with(svc, "One", AI);
        deploy(svc, "acme", true).await.unwrap();
        source_with(svc, "Two", AI);
        deploy(svc, "acme", false).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Two"));
        source_with(svc, "Three", "");
        deploy(svc, "acme", false).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Three"));
    }

    /// Commits `files` (relative to `root`) in a repository at `root`.
    fn commit_files(root: &Path, files: &[&str]) -> String {
        let repo = git2::Repository::init(root).unwrap();
        let mut idx = repo.index().unwrap();
        for file in files {
            idx.add_path(Path::new(file)).unwrap();
        }
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("test", "t@example.com").unwrap();
        let parents: Vec<git2::Commit> = repo
            .head()
            .ok()
            .and_then(|h| h.peel_to_commit().ok())
            .into_iter()
            .collect();
        let parents: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, "x", &tree, &parents)
            .unwrap()
            .to_string()
    }

    /// A deployment records the primary checkout's HEAD and whether its
    /// source had uncommitted changes; rollback names the target's.
    #[tokio::test]
    async fn a_deployment_records_the_commit_and_whether_it_was_dirty() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let files = [
            ".oxplow/extensions/acme/extension.yaml",
            ".oxplow/extensions/acme/pages/one.yaml",
        ];
        source(svc, "One");
        let head = commit_files(&svc.layout.project_dir, &files);
        let first = deploy(svc, "acme", true).await.unwrap();
        assert_eq!(
            (first.commit.as_deref(), first.dirty),
            (Some(&*head), false)
        );
        assert_eq!(first.provenance(), Some(format!("commit {}", &head[..7])));

        source(svc, "Two");
        write(
            &svc.layout.project_dir.join(".oxplow/extensions/acme"),
            "pages/new.yaml",
            "title: N\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
        let second = deploy(svc, "acme", true).await.unwrap();
        assert_eq!(
            (second.commit.as_deref(), second.dirty),
            (Some(&*head), true)
        );
        assert_eq!(
            second.provenance(),
            Some(format!("commit {} (uncommitted changes)", &head[..7]))
        );
        let rows = svc
            .db
            .read(|c| store_rows::list_tx(c, Some("acme")))
            .await
            .unwrap();
        let by_hash = |hash: &str| rows.iter().find(|r| r.hash == hash).unwrap();
        assert!(by_hash(&second.hash).dirty);
        assert!(!by_hash(&first.hash).dirty);

        let back = rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(back.hash, first.hash);
        assert_eq!((back.commit.as_deref(), back.dirty), (Some(&*head), false));
    }

    /// A copy of the acme source in another checkout, with `pages`
    /// (slug, query).
    fn draft(root: &Path, pages: &[(&str, &str)]) {
        let dir = root.join(".oxplow/extensions/acme");
        write(
            &dir,
            "extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        );
        for (slug, query) in pages {
            write(
                &dir,
                &format!("pages/{slug}.yaml"),
                &format!(
                    "title: {slug}\nloaders:\n  n: {{ sql: \"{query}\" }}\nview: {{ viz: number, from: n }}\n"
                ),
            );
        }
    }

    #[tokio::test]
    async fn source_diff_of_an_undeployed_source_adds_everything() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        let diff = source_diff(svc, &root, "acme").await.unwrap();
        assert_eq!(diff.deployed_hash, None);
        assert!(diff.differs);
        assert_eq!(diff.pages.added, vec!["acme/one".to_string()]);
        assert!(diff.pages.removed.is_empty() && diff.pages.changed.is_empty());
    }

    #[tokio::test]
    async fn source_diff_of_what_runs_does_not_differ() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        let deployed = deploy(svc, "acme", true).await.unwrap();
        let diff = source_diff(svc, &root, "acme").await.unwrap();
        assert_eq!(diff.deployed_hash.as_deref(), Some(deployed.hash.as_str()));
        assert!(!diff.differs);
        assert_eq!(diff.pages, Changes::default());
    }

    #[tokio::test]
    async fn source_diff_of_a_worktree_copy_names_added_removed_and_changed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        write(
            &svc.layout.project_dir.join(".oxplow/extensions/acme"),
            "pages/keep.yaml",
            "title: keep\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
        deploy(svc, "acme", true).await.unwrap();
        let other = tempfile::tempdir().unwrap();
        draft(
            other.path(),
            &[("keep", "SELECT 2 AS n"), ("two", "SELECT 1 AS n")],
        );
        let diff = source_diff(svc, other.path(), "acme").await.unwrap();
        assert!(diff.differs);
        assert_eq!(diff.pages.added, vec!["acme/two".to_string()]);
        assert_eq!(diff.pages.removed, vec!["acme/one".to_string()]);
        assert_eq!(diff.pages.changed, vec!["acme/keep".to_string()]);
    }

    #[tokio::test]
    async fn a_deploy_that_removes_a_page_warns() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        let first = deploy(svc, "acme", true).await.unwrap();
        assert!(!first.warnings.iter().any(|w| w.contains("removes")));
        let dir = svc.layout.project_dir.join(".oxplow/extensions/acme");
        std::fs::remove_file(dir.join("pages/one.yaml")).unwrap();
        write(
            &dir,
            "pages/two.yaml",
            "title: two\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
        let second = deploy(svc, "acme", true).await.unwrap();
        assert!(
            second.warnings.contains(
                &"deploying removes page(s) `acme/one` that the running version has".to_string()
            ),
            "{:?}",
            second.warnings
        );
    }

    const PAGE_SPEC: &str =
        "title: Overview\nloaders:\n  a: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: a }\n";

    /// A page written into a new extension scaffolds its manifest, loads,
    /// and is deployed when asked.
    #[tokio::test]
    async fn write_page_creates_the_extension_and_deploys_when_asked() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let written = write_page(svc, "my-pages", "overview", PAGE_SPEC, true, true)
            .await
            .unwrap();
        assert_eq!(written.page, "my-pages/overview");
        assert!(written.deployed.is_some());
        let dir = svc.layout.project_dir.join(".oxplow/extensions/my-pages");
        assert!(dir.join("extension.yaml").is_file());
        assert_eq!(
            std::fs::read_to_string(dir.join("pages/overview.yaml")).unwrap(),
            PAGE_SPEC
        );
        let ext = running(svc, "my-pages").expect("it runs");
        assert_eq!(ext.page_files.len(), 1);
        // Replacing it keeps one file, and without `deploy` nothing deploys.
        let again = write_page(svc, "my-pages", "overview", PAGE_SPEC, false, true)
            .await
            .unwrap();
        assert!(again.deployed.is_none());
    }

    /// A spec that doesn't parse is refused and nothing is written.
    #[tokio::test]
    async fn write_page_refuses_a_spec_that_does_not_parse() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let err = write_page(
            svc,
            "my-pages",
            "bad",
            "title: T\nnot_a_key: 1\n",
            false,
            true,
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("doesn't parse"), "{err}");
        assert!(!svc
            .layout
            .project_dir
            .join(".oxplow/extensions/my-pages")
            .exists());
    }

    /// A bundled extension is never written to.
    #[tokio::test]
    async fn write_page_refuses_a_bundled_extension() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let err = write_page(&fx.svc, "oxplow-bundled", "x", PAGE_SPEC, false, true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("bundled"), "{err}");
    }

    /// A new dashboard is a page in the Dashboards category with an empty
    /// grid, named after its title, written and deployed.
    #[tokio::test]
    async fn a_new_dashboard_is_an_empty_grid_page_in_dashboards() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let out = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.extension.new_dashboard",
                json!({ "title": "Release health!" }),
                true,
            )
            .await
            .unwrap();
        assert_eq!(
            out.result["page"],
            json!("my-pages/release-health"),
            "{:?}",
            out.result
        );
        let text = std::fs::read_to_string(
            svc.layout
                .project_dir
                .join(".oxplow/extensions/my-pages/pages/release-health.yaml"),
        )
        .unwrap();
        let page = crate::pages::schema::parse_page(
            "my-pages",
            "my_pages",
            "release-health",
            "x",
            &text,
            crate::pages::schema::PageKind::Page,
        )
        .unwrap();
        assert_eq!(page.title, "Release health!");
        assert_eq!(
            page.category,
            Some(crate::pages::spec::LauncherCategory::Dashboards)
        );
        assert!(
            matches!(&page.view.kind, crate::pages::schema::NodeKind::Grid { children } if children.is_empty()),
            "{text}"
        );
        let listed = svc
            .sql
            .query_sql(
                "SELECT title FROM v_page WHERE category = 'Dashboards'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(listed.rows.len(), 1, "deployed: {:?}", listed.rows);
        // The same title again is refused rather than replacing it.
        let err = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.extension.new_dashboard",
                json!({ "title": "Release health" }),
                true,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("already"), "{err}");
    }

    /// An agent in a worktree stream may not write into an in-project
    /// source; the primary's caller may.
    #[tokio::test]
    async fn write_page_is_refused_from_a_worktree_stream() {
        use oxplow_domain::stores::StreamStore as _;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let mut other = svc.stream_store.list().await.unwrap().pop().unwrap();
        other.id = oxplow_domain::StreamId::placeholder();
        other.title = "other".into();
        other.branch = "other".into();
        other.kind = oxplow_domain::StreamKind::Worktree;
        other.worktree_path = tempfile::tempdir()
            .unwrap()
            .keep()
            .to_string_lossy()
            .into_owned();
        let other_id = svc.stream_store.upsert(&other).await.unwrap();
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(other_id),
        };
        let input = json!({ "extension": "my-pages", "page": "overview", "spec": PAGE_SPEC });
        let err = svc
            .commands
            .run(&agent, "oxplow.extension.write_page", input.clone(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("main stream"), "{err}");
        assert!(!svc
            .layout
            .project_dir
            .join(".oxplow/extensions/my-pages")
            .exists());
        svc.commands
            .run(&Actor::Human, "oxplow.extension.write_page", input, false)
            .await
            .unwrap();
    }

    /// A source outside the project has no commit to record.
    #[tokio::test]
    async fn a_source_outside_the_project_has_no_provenance() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let project = svc.layout.project_dir.clone();
        let elsewhere = tempfile::tempdir().unwrap();
        write(&project, "a.txt", "a");
        let head = commit_files(&project, &["a.txt"]);
        assert_eq!(
            provenance(svc, &project, elsewhere.path()).await,
            (None, false)
        );
        assert_eq!(
            provenance(svc, &project, &project.join("a.txt")).await,
            (Some(head), false)
        );
    }

    /// A new version asks only about what it adds to what runs.
    #[tokio::test]
    async fn a_new_version_asks_only_about_the_permissions_it_adds() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source_with(svc, "One", AI);
        deploy(svc, "acme", true).await.unwrap();
        source_with(svc, "Two", "permissions: [ai.compute, files.read]\n");
        let lines = asked(deploy(svc, "acme", false).await);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("files"), "{lines:?}");
    }

    /// An extension turned off stays off until someone says so: deploying it
    /// asks, says it turns it on, and lists everything it would run with,
    /// though nothing changed in its source.
    #[tokio::test]
    async fn deploying_an_extension_that_is_off_asks_to_turn_it_on() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source_with(svc, "One", AI);
        deploy(svc, "acme", true).await.unwrap();
        set_enabled(svc, "acme", false, false).await.unwrap();
        let is_on =
            || oxplow_config::extension_settings(&svc.layout.project_dir).is_enabled("acme");
        assert!(!is_on());
        match deploy(svc, "acme", false).await {
            Err(DeployError::NeedsApproval { adds, turns_on, .. }) => {
                assert!(turns_on);
                assert_eq!(adds, vec!["ai — may call your AI provider with your key"]);
            }
            other => panic!("expected it to ask, got {other:?}"),
        }
        assert!(!is_on(), "asking turned it on");
        deploy(svc, "acme", true).await.unwrap();
        assert!(is_on());
    }

    /// A deploy that adds a permission asks before anything changes: it
    /// names what it adds, writes nothing, and goes live once confirmed.
    #[tokio::test]
    async fn a_deploy_that_adds_a_permission_pauses_until_confirmed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        deploy(svc, "acme", false).await.unwrap();
        source_with(svc, "Two", AI);
        assert_eq!(
            asked(deploy(svc, "acme", false).await),
            vec!["ai — may call your AI provider with your key"]
        );
        assert_eq!(page_title(svc).as_deref(), Some("One"));
        assert_eq!(rows(svc).await.len(), 1);
        deploy(svc, "acme", true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Two"));
        let recorded = svc
            .db
            .read(|c| store_rows::list_tx(c, Some("acme")))
            .await
            .unwrap();
        assert!(recorded[0].active && recorded[0].permissions.ai);
    }

    /// Rolling back to a deployment that reaches further asks the same way.
    #[tokio::test]
    async fn rollback_to_a_wider_deployment_asks_too() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source_with(svc, "One", AI);
        deploy(svc, "acme", true).await.unwrap();
        source_with(svc, "Two", "");
        deploy(svc, "acme", false).await.unwrap();
        assert_eq!(
            asked(rollback(svc, "acme", None, false).await),
            vec!["ai — may call your AI provider with your key"]
        );
        assert_eq!(page_title(svc).as_deref(), Some("Two"));
        rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));
    }

    /// `name`'s source in the project with a page and the `extra` manifest
    /// lines.
    fn other(svc: &Services, name: &str, extra: &str) {
        let dir = svc.layout.project_dir.join(".oxplow/extensions").join(name);
        write(
            &dir,
            "extension.yaml",
            &format!("manifest: 2\nname: {name}\n{extra}intent:\n  purpose: test\n"),
        );
        write(
            &dir,
            "pages/one.yaml",
            "title: T\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
    }

    /// Deploying an extension whose requirement is off refuses and changes
    /// nothing; once the requirement is deployed, it goes.
    #[tokio::test]
    async fn a_deploy_refuses_when_a_required_extension_is_off() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        other(svc, "tracker", "");
        source_with(svc, "One", "requires: [tracker]\n");
        let e = deploy(svc, "acme", true).await.unwrap_err();
        assert!(
            e.to_string()
                .contains("`acme` requires `tracker`, which is off (turn it on first)"),
            "{e}"
        );
        assert!(rows(svc).await.is_empty());
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        deploy(svc, "tracker", true).await.unwrap();
        deploy(svc, "acme", true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));
    }

    /// While an enabled extension requires `tracker`, turning `tracker`
    /// off or removing it refuses; once the dependant is off, both go —
    /// and the dependant can't come back on without it.
    #[tokio::test]
    async fn disabling_a_required_extension_is_refused_while_something_requires_it() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        other(svc, "tracker", "");
        source_with(svc, "One", "requires: [tracker]\n");
        deploy(svc, "tracker", true).await.unwrap();
        deploy(svc, "acme", true).await.unwrap();
        let refused = "`tracker` is required by `acme`; turn that off first";
        let e = set_enabled(svc, "tracker", false, true).await.unwrap_err();
        assert!(e.to_string().contains(refused), "{e}");
        let e = remove(svc, "tracker").await.unwrap_err();
        assert!(e.to_string().contains(refused), "{e}");
        assert!(oxplow_config::extension_settings(&root).is_enabled("tracker"));
        set_enabled(svc, "acme", false, true).await.unwrap();
        set_enabled(svc, "tracker", false, true).await.unwrap();
        let e = set_enabled(svc, "acme", true, true).await.unwrap_err();
        assert!(
            e.to_string()
                .contains("`acme` requires `tracker`, which is off"),
            "{e}"
        );
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        remove(svc, "tracker").await.unwrap();
    }

    /// `acme`'s source with a `build:` of `run` writing `dist/`, a
    /// component bundled at `dist/widget`, and an excluded `node_modules`.
    fn source_built(svc: &Services, title: &str, run: &str) -> PathBuf {
        let dir = source(svc, title);
        write(
            &dir,
            "extension.yaml",
            &format!(
                "manifest: 2\nname: acme\nbuild: {{ run: {run:?}, output: dist, exclude: [node_modules] }}\n\
                 custom_components:\n  - {{ id: widget, bundle: dist/widget }}\nintent:\n  purpose: test\n"
            ),
        );
        write(&dir, "src/widget.html", "<!doctype html>");
        write(&dir, "node_modules/x.txt", "dep");
        dir
    }

    const BUILD: &str = "mkdir -p dist/widget && cp src/widget.html dist/widget/index.html";

    fn build_line(run: &str) -> String {
        format!(
            "build — runs `{run}` in .oxplow/extensions/acme with your authority, on every deploy"
        )
    }

    /// A deploy runs the build, and the deployment holds its output but
    /// not the excluded folders. What the build wrote isn't the hash: an
    /// edit there redeploys nothing, a changed command does.
    #[tokio::test]
    async fn a_build_runs_on_deploy_and_its_output_is_in_the_deployment_but_not_its_hash() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let dir = source_built(svc, "One", BUILD);
        let first = deploy(svc, "acme", true).await.unwrap();
        let deployed = extension_store::deployment_dir(&root, "acme", &first.hash);
        assert!(deployed.join("dist/widget/index.html").is_file());
        assert!(deployed.join("src/widget.html").is_file());
        assert!(!deployed.join("node_modules").exists());
        assert_eq!(page_title(svc).as_deref(), Some("One"));
        let recorded = svc
            .db
            .read(|c| store_rows::list_tx(c, Some("acme")))
            .await
            .unwrap();
        assert_eq!(recorded[0].build_run.as_deref(), Some(BUILD));

        write(&dir, "dist/widget/index.html", "edited by hand");
        assert!(deploy(svc, "acme", true).await.unwrap().already);
        source_built(svc, "One", &format!("{BUILD} && true"));
        let second = deploy(svc, "acme", true).await.unwrap();
        assert!(!second.already);
        assert_ne!(first.hash, second.hash);
    }

    /// A build runs with the person's authority: a new or changed command
    /// asks first, as a permission does; the same one asks nothing, and
    /// rolling back to another command asks again.
    #[tokio::test]
    async fn a_new_or_changed_build_command_asks_first() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_built(svc, "One", BUILD);
        assert_eq!(
            asked(deploy(svc, "acme", false).await),
            vec![build_line(BUILD)]
        );
        assert!(rows(svc).await.is_empty());
        assert!(!root.join(".oxplow/extensions/acme/dist").exists());
        deploy(svc, "acme", true).await.unwrap();
        source_built(svc, "Two", BUILD);
        deploy(svc, "acme", false).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Two"));
        let changed = format!("{BUILD} && true");
        source_built(svc, "Three", &changed);
        assert_eq!(
            asked(deploy(svc, "acme", false).await),
            vec![build_line(&changed)]
        );
        deploy(svc, "acme", true).await.unwrap();
        assert_eq!(
            asked(rollback(svc, "acme", None, false).await),
            vec![build_line(BUILD)]
        );
        rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Two"));
    }

    /// A build that fails deploys nothing: not turned on, no row, nothing
    /// in the store, and the error says why.
    #[tokio::test]
    async fn a_failing_build_changes_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_built(svc, "One", "echo broken >&2; exit 3");
        let e = deploy(svc, "acme", true).await.unwrap_err().to_string();
        assert!(
            e.contains("building `acme`") && e.contains("exited 3") && e.contains("broken"),
            "{e}"
        );
        assert!(rows(svc).await.is_empty());
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        assert!(!root.join(extension_store::STORE_DIR).join("acme").exists());
    }

    /// The deployment is loaded once built: a build that doesn't write the
    /// bundle the manifest declares is refused and leaves nothing behind.
    #[tokio::test]
    async fn a_build_that_doesnt_write_what_it_declares_is_refused() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_built(
            svc,
            "One",
            "mkdir -p dist/elsewhere && touch dist/elsewhere/a",
        );
        let e = deploy(svc, "acme", true).await.unwrap_err().to_string();
        assert!(
            e.contains("extension `acme` doesn't load once built"),
            "{e}"
        );
        assert!(rows(svc).await.is_empty());
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        let store = root.join(extension_store::STORE_DIR).join("acme");
        assert!(!store.exists() || std::fs::read_dir(&store).unwrap().next().is_none());
    }

    /// Sharing a built extension leaves its output and excluded folders
    /// out of git, and unsharing takes those lines back out too.
    #[tokio::test]
    async fn sharing_ignores_the_build_output() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_built(svc, "One", BUILD);
        deploy(svc, "acme", true).await.unwrap();
        let ignored = |rel: &str| {
            std::process::Command::new("git")
                .args(["check-ignore", "-q", rel])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        };
        share(svc, "acme").unwrap();
        assert!(!ignored(".oxplow/extensions/acme/extension.yaml"));
        assert!(!ignored(".oxplow/extensions/acme/src/widget.html"));
        assert!(ignored(".oxplow/extensions/acme/dist/widget/index.html"));
        assert!(ignored(".oxplow/extensions/acme/node_modules/x.txt"));
        unshare(svc, "acme").unwrap();
        let text = std::fs::read_to_string(root.join(".oxplow/.gitignore")).unwrap();
        assert!(!text.contains("extensions/acme/"), "{text}");
    }

    /// Boot leaves a built extension with no deployment here for a person
    /// to deploy, its listing naming the build that deploy would run.
    #[tokio::test]
    async fn boot_leaves_a_built_extension_undeployed_with_the_build_line_pending() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_built(svc, "One", BUILD);
        write(&root, ".oxplow/personal.yaml", "enableExtensions: [acme]\n");
        svc.reload_config_from_disk().unwrap();
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        let acme = acme(svc).unwrap();
        assert_eq!(acme.deployment, Deployment::Undeployed);
        assert_eq!(acme.pending, vec![build_line(BUILD)]);
        assert!(!root.join(".oxplow/extensions/acme/dist").exists());
    }

    /// Boot leaves an extension that declares permissions undeployed, and
    /// its listing says what its deploy will ask for.
    #[tokio::test]
    async fn boot_leaves_a_declaring_extension_undeployed_and_says_what_it_needs() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_with(svc, "One", AI);
        write(&root, ".oxplow/personal.yaml", "enableExtensions: [acme]\n");
        svc.reload_config_from_disk().unwrap();
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        let acme = acme(svc).unwrap();
        assert_eq!(acme.deployment, Deployment::Undeployed);
        assert_eq!(
            acme.pending,
            vec!["ai — may call your AI provider with your key"]
        );
        assert_eq!(page_title(svc), None);
        let listed = svc
            .sql
            .query_sql(
                "SELECT pending FROM v_extension WHERE name = 'acme'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            listed.rows,
            vec![vec![oxplow_db::SqlCell::Text(
                "[\"ai — may call your AI provider with your key\"]".into()
            )]]
        );
    }

    /// An undeployed extension is listed for its source: when the source
    /// goes, the listing drops it without an activation (a running
    /// extension is not touched: it runs from its deployment).
    #[tokio::test]
    async fn an_undeployed_extension_leaves_the_listing_when_its_source_is_deleted() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_with(svc, "One", AI);
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        let listed = || async {
            svc.sql
                .query_sql(
                    "SELECT name FROM v_extension WHERE name = 'acme'",
                    vec![],
                    None,
                )
                .await
                .unwrap()
                .rows
                .len()
        };
        assert_eq!(listed().await, 1);
        std::fs::remove_dir_all(root.join(".oxplow/extensions/acme")).unwrap();
        svc.extension_activation
            .restate_sources(svc.clone())
            .await
            .unwrap();
        assert_eq!(listed().await, 0);
        // A source that comes back is listed again.
        source_with(svc, "One", AI);
        svc.extension_activation
            .restate_sources(svc.clone())
            .await
            .unwrap();
        assert_eq!(listed().await, 1);
    }

    /// An agent can't approve: its deploy that adds a permission becomes a
    /// proposal naming what it adds, and a person's approval deploys it.
    #[tokio::test]
    async fn an_agents_deploy_that_adds_permissions_becomes_a_proposal_whose_approval_deploys() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source_with(svc, "One", AI);
        let agent = oxplow_domain::Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let err = svc
            .commands
            .run(
                &agent,
                "oxplow.extension.deploy",
                serde_json::json!({ "name": "acme" }),
                true,
            )
            .await
            .unwrap_err();
        let oxplow_domain::CommandError::Proposed { proposal, .. } = err else {
            panic!("{err:?}");
        };
        assert_eq!(page_title(svc), None);
        let id = proposal;
        let kept = svc
            .commands
            .proposal_store()
            .get(id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            kept.preview["details"],
            serde_json::json!(["ai — may call your AI provider with your key"])
        );
        svc.commands
            .approve(&oxplow_domain::Actor::Human, id)
            .await
            .unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));
    }

    /// A bundled extension's source that declares what ships asks nothing:
    /// the shipped deployment's permissions are what runs.
    #[tokio::test]
    async fn a_bundled_deploy_with_the_shipped_permissions_asks_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let dir = root.join(".oxplow/extensions/oxplow-bundled");
        for (rel, bytes) in crate::bundled_extensions::find("oxplow-bundled")
            .unwrap()
            .files()
        {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        write(
            &dir,
            "pages/extra.yaml",
            "title: Extra\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
        let deployed = deploy(svc, "oxplow-bundled", false).await.unwrap();
        assert!(!deployed.already);
    }

    /// `v_extension` says what each extension declares it may reach, as
    /// the list `permissions:` is: `[]` for the baseline.
    #[tokio::test]
    async fn v_extension_lists_each_extensions_permissions() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        deploy(svc, "acme", true).await.unwrap();
        let listed = svc
            .sql
            .query_sql(
                "SELECT name, permissions FROM v_extension \
                 WHERE name IN ('acme', 'oxplow-foundation') ORDER BY name",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            listed.rows,
            vec![
                vec![
                    oxplow_db::SqlCell::Text("acme".into()),
                    oxplow_db::SqlCell::Text("[]".into())
                ],
                vec![
                    oxplow_db::SqlCell::Text("oxplow-foundation".into()),
                    oxplow_db::SqlCell::Text(
                        "[\"sql.read\",\"files.read\",\"ai.compute\",\"agent.write\"]".into()
                    )
                ],
            ]
        );
    }

    /// `v_extension_source` is each source as it is now, so a source that
    /// moved on from what runs shows; `v_extension.shared` says whether the
    /// extension is the team's.
    #[tokio::test]
    async fn v_extension_marks_a_source_that_moved_on_and_a_shared_one() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let row = || async {
            svc.sql
                .query_sql(
                    "SELECT s.hash IS NOT e.hash, e.shared, s.path FROM v_extension e \
                     JOIN v_extension_source s USING (name) WHERE e.name = 'acme'",
                    vec![],
                    None,
                )
                .await
                .unwrap()
                .rows
        };
        let int = |n| oxplow_db::SqlCell::Int(n);
        let path = oxplow_db::SqlCell::Text(".oxplow/extensions/acme".into());
        source(svc, "One");
        deploy(svc, "acme", true).await.unwrap();
        assert_eq!(row().await, vec![vec![int(0), int(0), path.clone()]]);
        // An edit changes nothing that runs; the source is read as it is.
        source(svc, "Two");
        assert_eq!(row().await, vec![vec![int(1), int(0), path]]);
        share(svc, "acme").unwrap();
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        assert_eq!(row().await[0][1], int(1));
    }

    /// The Extension page opened on `acme`: its params, the way a link to
    /// the extension's URI opens it.
    fn acme_page() -> crate::pages::runner::RunRequest {
        use crate::pages::runner::{RunRequest, RunTarget};
        RunRequest {
            target: RunTarget::Page("oxplow-foundation/extension".into()),
            params: [
                (
                    "uri".to_string(),
                    oxplow_db::SqlCell::Text("oxplow:oxplow/extension?name=acme".into()),
                ),
                ("name".to_string(), oxplow_db::SqlCell::Text("acme".into())),
            ]
            .into_iter()
            .collect(),
            ctx: crate::pages::spec::ViewContext::default(),
            viewer: oxplow_domain::Actor::Human,
        }
    }

    /// An extension's page allows Deploy only where it would change
    /// something: a source that moved on.
    #[tokio::test]
    async fn the_extension_page_offers_deploy_only_when_the_source_moved_on() {
        use crate::pages::open_runs::PageRuns;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        deploy(svc, "acme", true).await.unwrap();
        let acme = || async {
            let (_, run) = PageRuns::default()
                .open(svc, acme_page(), "w")
                .await
                .unwrap();
            let about = &run.loaders["about"].result;
            let col = |name: &str| about.columns.iter().position(|c| c == name).unwrap();
            (
                about.rows[0][col("can_deploy")].clone(),
                about.rows[0][col("can_disable")].clone(),
            )
        };
        let int = |n| oxplow_db::SqlCell::Int(n);
        assert_eq!(acme().await, (int(0), int(1)));
        source(svc, "Two");
        assert_eq!(acme().await, (int(1), int(1)));
    }

    /// An extension page's actions run their commands from inside a page
    /// action's read: disabling, enabling and removing there activate all
    /// the same.
    #[tokio::test]
    async fn the_extension_page_actions_disable_enable_and_remove() {
        use crate::pages::actions::{run_page_action, PageActionCall};
        use crate::pages::open_runs::PageRuns;
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        deploy(svc, "acme", true).await.unwrap();
        let runs = PageRuns::default();
        for action in ["disable", "enable", "remove"] {
            // Each press is on a fresh run: the row it acts on is the one
            // the run holds.
            let (id, _) = runs.open(svc, acme_page(), "w").await.unwrap();
            run_page_action(
                svc,
                &runs,
                PageActionCall {
                    run_id: id,
                    node: None,
                    action: action.into(),
                    row: None,
                    input: None,
                    drop: None,
                    on_behalf_of: oxplow_domain::Actor::Human,
                    confirmed: true,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{action}: {e}"));
        }
        assert!(rows(svc).await.is_empty());
        assert!(!svc.extension_catalog.live().contains_key("acme"));
    }

    /// A source runs nothing until it is deployed; a deploy runs it, turns
    /// it on for the person and records the deployment; the same source
    /// again changes nothing.
    #[tokio::test]
    async fn a_deploy_runs_a_source_and_only_a_deploy_does() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        assert_eq!(page_title(svc), None);
        assert_eq!(acme(svc).unwrap().deployment, Deployment::Undeployed);

        let deployed = deploy(svc, "acme", true).await.unwrap();
        assert!(!deployed.already);
        assert_eq!(page_title(svc).as_deref(), Some("One"));
        assert_eq!(
            acme(svc).unwrap().deployment,
            Deployment::Deployed {
                hash: deployed.hash.clone()
            }
        );
        assert!(oxplow_config::extension_settings(&root)
            .personal
            .enable
            .contains(&"acme".to_string()));
        assert_eq!(rows(svc).await, [(deployed.hash.clone(), true)]);
        assert!(extension_store::deployment_dir(&root, "acme", &deployed.hash).is_dir());

        let again = deploy(svc, "acme", true).await.unwrap();
        assert!(again.already);
        assert_eq!(again.hash, deployed.hash);

        // An edit runs nothing until it is deployed, whatever activates.
        source(svc, "Uno");
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));
        let second = deploy(svc, "acme", true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Uno"));
        assert_eq!(
            rows(svc).await,
            [(second.hash.clone(), true), (deployed.hash.clone(), false)]
        );

        // Back to the one before, and forward again by its hash.
        rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));
        rollback(svc, "acme", Some(&second.hash[..6]), true)
            .await
            .unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("Uno"));
    }

    /// A source that doesn't load is refused and changes nothing: not run,
    /// not turned on.
    #[tokio::test]
    async fn a_broken_source_changes_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let dir = source(svc, "One");
        write(&dir, "extension.yaml", "manifest: 2\nname: acme\n");
        let err = deploy(svc, "acme", true).await.unwrap_err();
        assert!(err.to_string().contains("doesn't load"), "{err}");
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        assert!(rows(svc).await.is_empty());
        assert_eq!(page_title(svc), None);
    }

    /// Off for the person stops it; removing forgets its deployments and
    /// keeps its source.
    #[tokio::test]
    async fn turning_off_and_removing_keep_the_source() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let dir = source(svc, "One");
        let deployed = deploy(svc, "acme", true).await.unwrap();

        set_enabled(svc, "acme", false, true).await.unwrap();
        assert_eq!(page_title(svc), None);
        set_enabled(svc, "acme", true, true).await.unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));

        remove(svc, "acme").await.unwrap();
        assert_eq!(page_title(svc), None);
        assert!(rows(svc).await.is_empty());
        assert!(!extension_store::deployment_dir(&root, "acme", &deployed.hash).exists());
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        assert!(dir.join("extension.yaml").is_file());
    }

    /// Sharing lets git see the source and moves the name to the team's
    /// list; unsharing undoes both.
    #[tokio::test]
    async fn sharing_moves_an_extension_between_the_lists_and_into_git() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", true).await.unwrap();
        let ignored = |rel: &str| {
            std::process::Command::new("git")
                .args(["check-ignore", "-q", rel])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        };
        assert!(ignored(".oxplow/extensions/acme/extension.yaml"));

        share(svc, "acme").unwrap();
        assert!(!ignored(".oxplow/extensions/acme/extension.yaml"));
        assert!(ignored(".oxplow/extensions/other/extension.yaml"));
        assert!(ignored(".oxplow/local.sqlite"));
        let settings = oxplow_config::extension_settings(&root);
        assert!(settings.team.contains(&"acme".to_string()));
        assert!(!settings.personal.enable.contains(&"acme".to_string()));

        unshare(svc, "acme").unwrap();
        assert!(ignored(".oxplow/extensions/acme/extension.yaml"));
        let settings = oxplow_config::extension_settings(&root);
        assert!(!settings.team.contains(&"acme".to_string()));
        assert!(settings.personal.enable.contains(&"acme".to_string()));
    }

    /// Boot deploys nothing: an enabled extension with a source and no
    /// deployment here is listed undeployed and runs nothing until a person
    /// deploys it; one listed with neither is listed as not here.
    #[tokio::test]
    async fn boot_deploys_nothing_an_enabled_source_is_undeployed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        write(
            &root,
            ".oxplow/personal.yaml",
            "enableExtensions: [acme, ghost]\n",
        );
        svc.reload_config_from_disk().unwrap();
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        assert_eq!(page_title(svc), None);
        assert_eq!(acme(svc).unwrap().deployment, Deployment::Undeployed);
        assert!(rows(svc).await.is_empty());
        let ghost = svc
            .extension_catalog
            .unpinned(&root, "a test reads the catalog")
            .iter()
            .find(|e| e.name == "ghost")
            .cloned()
            .unwrap();
        assert_eq!(ghost.deployment, Deployment::NotHere);
        assert!(!ghost.enabled && ghost.errors.is_empty());
    }

    /// While developing oxplow (`enableBundledExtensions: false`) its own
    /// extensions run from their sources, deployed like any extension's
    /// (the fixture deploys foundation, as `oxplow-dev extension deploy`
    /// does); `oxplow-bundled`, never deployed, is listed undeployed with
    /// the permissions its deploy will ask for.
    #[tokio::test]
    async fn developing_oxplow_its_own_extensions_run_from_their_sources() {
        let fx = crate::test_fixtures::services_developing_oxplow().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let source = |name: &str| root.join(".oxplow/extensions").join(name);
        let listed = |name: &str| {
            svc.extension_catalog
                .active()
                .unwrap()
                .extensions()
                .iter()
                .find(|e| e.name == name)
                .cloned()
                .unwrap()
        };
        assert_eq!(
            listed("oxplow-foundation").deployment,
            Deployment::Deployed {
                hash: extension_store::source_hash(&source("oxplow-foundation")).unwrap()
            }
        );
        assert!(svc.commands.spec("oxplow.config.get").is_some());
        let bundled = listed("oxplow-bundled");
        assert_eq!(bundled.deployment, Deployment::Undeployed);
        assert!(!bundled.pending.is_empty(), "it waits on its permissions");

        // A deploy of the unchanged source is the one running; nothing
        // ships to roll back to.
        let again = deploy(svc, "oxplow-foundation", false).await.unwrap();
        assert!(again.already);
        let e = rollback(svc, "oxplow-foundation", None, true)
            .await
            .unwrap_err();
        assert!(e.to_string().contains("no earlier deployment"), "{e}");
        let e = remove(svc, "oxplow-foundation").await.unwrap_err();
        assert!(e.to_string().contains("part of oxplow"), "{e}");

        // An edit deploys like any extension's, and rolls back.
        let manifest =
            std::fs::read_to_string(source("oxplow-foundation").join("extension.yaml")).unwrap();
        std::fs::write(
            source("oxplow-foundation").join("extension.yaml"),
            manifest.replacen("\ncommands:\n", "\ncommands:\n  - name: test.draft_again\n    summary: \"A second draft command.\"\n    scope: agent_input.write\n    op: draft\n", 1),
        )
        .unwrap();
        let edited = deploy(svc, "oxplow-foundation", false).await.unwrap();
        assert!(!edited.already);
        assert!(svc.commands.spec("oxplow.test.draft_again").is_some());
        rollback(svc, "oxplow-foundation", None, true)
            .await
            .unwrap();
        assert!(svc.commands.spec("oxplow.test.draft_again").is_none());
        assert_eq!(
            listed("oxplow-foundation").deployment,
            Deployment::Deployed { hash: again.hash }
        );
    }

    /// Foundation's commands are what its active deployment declares:
    /// deploying it with a new command registers that command with core's
    /// authority, as boot registers the shipped ones, and rolling back
    /// takes it away again.
    #[tokio::test]
    async fn deploying_foundation_registers_the_commands_its_deployment_declares() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let name = "oxplow-foundation";
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        let dir = root.join(".oxplow/extensions").join(name);
        let shipped = crate::bundled_extensions::find(name).unwrap();
        for (rel, bytes) in shipped.files() {
            let to = dir.join(rel);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::write(to, bytes).unwrap();
        }
        let manifest = std::fs::read_to_string(dir.join("extension.yaml")).unwrap();
        let added = "commands:\n  - name: test.draft_again\n    summary: \"A second draft command.\"\n    scope: agent_input.write\n    op: draft\n";
        assert!(manifest.contains("\ncommands:\n"));
        std::fs::write(
            dir.join("extension.yaml"),
            manifest.replacen("\ncommands:\n", &format!("\n{added}"), 1),
        )
        .unwrap();
        assert!(svc.commands.spec("oxplow.test.draft_again").is_none());

        deploy(svc, name, true).await.unwrap();
        assert!(svc.commands.spec("oxplow.test.draft_again").is_some());
        // Its other commands are still there, with core's authority: a
        // person's config read runs.
        assert!(svc.commands.spec("oxplow.agent.draft").is_some());
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.config.get",
                serde_json::json!({ "key": "extensions" }),
                false,
            )
            .await
            .unwrap();

        rollback(svc, name, None, true).await.unwrap();
        assert!(svc.commands.spec("oxplow.test.draft_again").is_none());
        assert!(svc.commands.spec("oxplow.agent.draft").is_some());
    }

    /// Foundation is always on: a deployment of it that doesn't load at
    /// boot (one an older oxplow wrote, a bad deploy from the repo) gives
    /// way to the shipped one, never to no foundation, and says so.
    #[tokio::test]
    async fn a_foundation_deployment_that_fails_at_boot_falls_back_to_the_shipped_one() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let project = svc.layout.project_dir.clone();
        let name = "oxplow-foundation";
        let hash = "00000000000000ba";
        let dir = extension_store::deployment_dir(&project, name, hash);
        write(
            &dir,
            "extension.yaml",
            "manifest: 2\nname: oxplow-foundation\nnot_a_key: 1\n",
        );
        let row = store_rows::DeploymentRow {
            name: name.into(),
            hash: hash.into(),
            deployed_at: "2026-10-19T00:00:00Z".into(),
            commit: None,
            dirty: false,
            source: "extensions/oxplow-foundation".into(),
            active: false,
            permissions: Permissions::default(),
            build_run: None,
        };
        svc.db
            .transaction(move |tx| {
                store_rows::record_tx(tx, &row)?;
                store_rows::set_active_tx(tx, &[(name.to_string(), hash.to_string())])
            })
            .await
            .unwrap();

        svc.extension_activation
            .initialize(svc.clone())
            .await
            .expect("foundation activates");
        // What runs is the shipped copy; what the person chose stays
        // recorded, so every boot tries it again and says why it doesn't.
        assert_eq!(active_hashes(svc, name).await, vec![hash.to_string()]);
        let active = svc.extension_catalog.active().unwrap();
        let foundation = active.extensions().iter().find(|e| e.name == name).unwrap();
        assert!(
            matches!(foundation.deployment, Deployment::Shipped { .. }),
            "{:?}",
            foundation.deployment
        );
        assert!(foundation.errors.is_empty(), "{:?}", foundation.errors);
        // Its problem says why; the shipped copy (here also its last
        // working version) runs.
        let problems = svc.extension_activation.problems();
        assert!(
            problems
                .get(name)
                .is_some_and(|p| p.iter().any(|m| m.render("").contains("not_a_key"))),
            "{problems:?}"
        );
        assert_eq!(
            svc.extension_catalog.live().get(name).map(String::as_str),
            Some(hash)
        );
    }

    /// Services over a fresh project that haven't activated: what a boot
    /// finds on disk is written first.
    fn unbooted() -> (tempfile::TempDir, Arc<Services>) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = Services::in_memory_inactive(dir.path()).unwrap();
        (dir, svc)
    }

    /// Record `files` as `name`'s active deployment `hash`, as a boot finds it.
    async fn active_deployment(svc: &Services, name: &str, hash: &str, files: &[(&str, String)]) {
        let dir = extension_store::deployment_dir(&svc.layout.project_dir, name, hash);
        for (rel, text) in files {
            write(&dir, rel, text);
        }
        let row = store_rows::DeploymentRow {
            name: name.into(),
            hash: hash.into(),
            deployed_at: "2026-10-19T00:00:00Z".into(),
            commit: None,
            dirty: false,
            source: format!("extensions/{name}"),
            active: false,
            permissions: Permissions::default(),
            build_run: None,
        };
        let (name, hash) = (name.to_string(), hash.to_string());
        svc.db
            .transaction(move |tx| {
                store_rows::record_tx(tx, &row)?;
                store_rows::set_active_tx(tx, &[(name.clone(), hash.clone())])
            })
            .await
            .unwrap();
    }

    /// The shipped files of `name`, as text, with `manifest` edited.
    fn shipped_with(name: &str, edit: impl Fn(String) -> String) -> Vec<(&'static str, String)> {
        crate::bundled_extensions::find(name)
            .unwrap()
            .files()
            .iter()
            .map(|(rel, bytes)| {
                let text = String::from_utf8_lossy(bytes).into_owned();
                (
                    *rel,
                    if *rel == "extension.yaml" {
                        edit(text)
                    } else {
                        text
                    },
                )
            })
            .collect()
    }

    fn running(svc: &Services, name: &str) -> Option<crate::extension::extensions::Extension> {
        svc.extension_catalog
            .active()
            .unwrap()
            .extensions()
            .iter()
            .find(|e| e.name == name && e.enabled)
            .cloned()
    }

    /// At a fresh boot, a deployment of an extension that ships with
    /// oxplow that can't load — foundation's or `oxplow-bundled`'s — gives
    /// way to the shipped copy and says so.
    #[tokio::test]
    async fn a_shipped_extensions_broken_deployment_gives_way_at_boot() {
        for name in ["oxplow-foundation", "oxplow-bundled"] {
            let (_dir, svc) = unbooted();
            let files = shipped_with(name, |m| format!("{m}not_a_key: 1\n"));
            let files: Vec<(&str, String)> = files.into_iter().collect();
            active_deployment(&svc, name, "00000000000000ba", &files).await;
            svc.extension_activation
                .initialize(svc.clone())
                .await
                .unwrap();
            let ext = running(&svc, name).unwrap_or_else(|| panic!("{name} runs"));
            assert!(
                matches!(ext.deployment, Deployment::Shipped { .. }),
                "{name}: {:?}",
                ext.deployment
            );
            let why = svc.extension_activation.problems();
            assert!(
                why.get(name).is_some_and(|p| p
                    .iter()
                    .any(|m| m.render("").contains("the shipped version runs"))),
                "{name}: {why:?}"
            );
            assert!(svc.commands.spec("oxplow.config.get").is_some());
        }
    }

    /// A deployment that loads but doesn't prepare (a command over an
    /// operation oxplow doesn't have) gives way the same way.
    #[tokio::test]
    async fn a_foundation_deployment_that_doesnt_prepare_gives_way_at_boot() {
        let (_dir, svc) = unbooted();
        let files = shipped_with("oxplow-foundation", |m| {
            m.replacen(
                "\ncommands:\n",
                "\ncommands:\n  - name: test.nothing\n    summary: \"Over no operation.\"\n    scope: agent_input.write\n    op: nothing_here\n",
                1,
            )
        });
        active_deployment(&svc, "oxplow-foundation", "00000000000000bb", &files).await;
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        let ext = running(&svc, "oxplow-foundation").unwrap();
        assert!(
            matches!(ext.deployment, Deployment::Shipped { .. }),
            "{:?}",
            ext.deployment
        );
        assert!(svc.commands.spec("oxplow.test.nothing").is_none());
        assert!(svc.commands.spec("oxplow.config.get").is_some());
    }

    /// While developing oxplow nothing ships and boot deploys nothing: a
    /// checkout with foundation's source but no deployment of it runs
    /// without foundation, and its problem — in the listing the shell's
    /// dialog reads, and still after a later activation — names the
    /// command that deploys it. Another extension deploys and runs.
    #[tokio::test]
    async fn developing_oxplow_with_no_foundation_deployment_says_how_to_deploy_it() {
        let (dir, svc) = unbooted();
        for b in crate::bundled_extensions::BUNDLED {
            let to = dir.path().join(".oxplow/extensions").join(b.name);
            for (rel, bytes) in b.files() {
                let file = to.join(rel);
                std::fs::create_dir_all(file.parent().unwrap()).unwrap();
                std::fs::write(file, bytes).unwrap();
            }
        }
        write(
            dir.path(),
            ".oxplow/project.yaml",
            "enableBundledExtensions: false\nextensions: [acme]\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        );
        write(
            dir.path(),
            ".oxplow/extensions/acme/pages/one.yaml",
            "title: One\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { viz: table, from: n }\n",
        );
        // Built after the files: the mode is read when it is made.
        let svc = {
            drop(svc);
            Services::in_memory_inactive(dir.path()).unwrap()
        };
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .expect("the boot goes on without it");
        if !cfg!(debug_assertions) {
            return;
        }
        assert!(running(&svc, "oxplow-foundation").is_none());
        assert!(running(&svc, "acme").is_none());
        for _ in 0..2 {
            let why = svc.extension_activation.problems();
            assert!(
                why.get("oxplow-foundation").is_some_and(|p| p
                    .iter()
                    .any(|m| m.render("").contains("oxplow-dev -- extension deploy"))),
                "{why:?}"
            );
            svc.extension_activation
                .activate(svc.clone())
                .await
                .unwrap();
        }
        deploy(&svc, "acme", true).await.unwrap();
        assert!(running(&svc, "acme").is_some());
    }

    /// What the next boot would run is found without touching the
    /// project: its active deployments activate in a throwaway, so a
    /// foundation deployment that no longer activates against this oxplow
    /// is said before anyone starts it, with whether its source moved on.
    /// Deploying the source (here, recording it active) clears it.
    #[tokio::test]
    async fn the_next_boot_is_checked_without_starting_oxplow() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let project = dir.path();
        let source = project.join(".oxplow/extensions/oxplow-foundation");
        for (rel, text) in shipped_with("oxplow-foundation", |t| t) {
            write(&source, rel, &text);
        }
        write(
            project,
            ".oxplow/project.yaml",
            "enableBundledExtensions: false\n",
        );
        if !cfg!(debug_assertions) {
            return;
        }
        let row = |hash: &str| DeploymentRow {
            name: "oxplow-foundation".into(),
            hash: hash.into(),
            deployed_at: "2026-10-19T00:00:00Z".into(),
            commit: None,
            dirty: false,
            source: source.display().to_string(),
            active: true,
            permissions: Permissions::default(),
            build_run: None,
        };

        // An older foundation, recorded active, that doesn't activate.
        let stale =
            extension_store::deployment_dir(project, "oxplow-foundation", "00000000000000ba");
        for (rel, text) in shipped_with("oxplow-foundation", |t| format!("{t}not_a_key: 1\n")) {
            write(&stale, rel, &text);
        }
        let next = next_boot(project, &[row("00000000000000ba")])
            .await
            .unwrap();
        let why = next
            .problems
            .get("oxplow-foundation")
            .expect("it is held")
            .iter()
            .map(|d| d.render(""))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            why.contains("not_a_key") && why.contains("00000000000000ba"),
            "{why}"
        );
        let runs = &next
            .runs
            .iter()
            .find(|(n, _)| n == "oxplow-foundation")
            .unwrap()
            .1;
        assert!(
            runs.contains("nothing") && runs.contains("its source has changed"),
            "{runs}"
        );

        // Its source, deployed: nothing is held, and it is what runs.
        let hash = extension_store::source_hash(&source).unwrap();
        extension_store::copy_into_store(project, "oxplow-foundation", &source, &hash).unwrap();
        let next = next_boot(project, &[row(&hash)]).await.unwrap();
        assert!(next.problems.is_empty(), "{:?}", next.problems);
        let runs = &next
            .runs
            .iter()
            .find(|(n, _)| n == "oxplow-foundation")
            .unwrap()
            .1;
        assert!(runs.contains(&hash) && !runs.contains("changed"), "{runs}");
        // The project itself was only read.
        assert!(!project.join(".oxplow/local.sqlite").exists());
    }

    /// `v_extension.errors` for `name`, as the Alerts page reads it.
    async fn listed_errors(svc: &Services, name: &str) -> String {
        let rows = svc
            .sql
            .query_sql(
                "SELECT errors FROM v_extension WHERE name = ?1",
                vec![oxplow_db::SqlCell::Text(name.into())],
                None,
            )
            .await
            .unwrap()
            .rows;
        match rows.first().map(|r| &r[0]) {
            Some(oxplow_db::SqlCell::Text(errors)) => errors.clone(),
            other => panic!("{name}: {other:?}"),
        }
    }

    /// While developing oxplow nothing replaces a foundation deployment
    /// that doesn't activate at boot (one an older oxplow recorded): it is
    /// held, and its problem — in the listing Alerts reads, and still after
    /// later activations — says which deployment failed, what is missing
    /// without it, and how to bring it back.
    #[tokio::test]
    async fn developing_oxplow_a_foundation_deployment_that_fails_at_boot_says_what_happened() {
        let (dir, svc) = unbooted();
        let source = dir.path().join(".oxplow/extensions/oxplow-foundation");
        for (rel, text) in shipped_with("oxplow-foundation", |t| t) {
            write(&source, rel, &text);
        }
        write(
            dir.path(),
            ".oxplow/project.yaml",
            "enableBundledExtensions: false\n",
        );
        // Built after the files: the mode is read when it is made.
        let svc = {
            drop(svc);
            Services::in_memory_inactive(dir.path()).unwrap()
        };
        active_deployment(
            &svc,
            "oxplow-foundation",
            "00000000000000ba",
            &shipped_with("oxplow-foundation", |t| format!("{t}not_a_key: 1\n")),
        )
        .await;
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        if !cfg!(debug_assertions) {
            return;
        }
        assert!(running(&svc, "oxplow-foundation").is_none());
        assert_eq!(
            active_hashes(&svc, "oxplow-foundation").await,
            vec!["00000000000000ba"]
        );
        for _ in 0..2 {
            let listed = listed_errors(&svc, "oxplow-foundation").await;
            for said in [
                "not_a_key",
                "deployment 00000000000000ba doesn't activate",
                "agent harnesses",
                "oxplow-dev -- extension deploy",
            ] {
                assert!(listed.contains(said), "{said}: {listed}");
            }
            svc.extension_activation
                .activate(svc.clone())
                .await
                .unwrap();
        }
    }

    /// A held extension is listed with why: its last working version's
    /// row, or — when nothing of it runs — its own row, off, with the
    /// reasons in `errors`.
    #[tokio::test]
    async fn a_held_extension_is_listed_with_why() {
        let (dir, svc) = unbooted();
        write(dir.path(), ".oxplow/project.yaml", "extensions: [acme]\n");
        write(
            dir.path(),
            ".oxplow/extensions/acme/extension.yaml",
            "manifest: 2\nname: acme\nnot_a_key: 1\nintent:\n  purpose: test\n",
        );
        active_deployment(
            &svc,
            "acme",
            "00000000000000cc",
            &[(
                "extension.yaml",
                "manifest: 2\nname: acme\nnot_a_key: 1\nintent:\n  purpose: test\n".into(),
            )],
        )
        .await;
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        let rows = svc
            .sql
            .query_sql(
                "SELECT enabled, errors FROM v_extension WHERE name = 'acme'",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0][0], oxplow_db::SqlCell::Int(0));
        let oxplow_db::SqlCell::Text(errors) = &rows[0][1] else {
            panic!("{rows:?}")
        };
        assert!(
            errors.contains("not_a_key") && errors.contains("isn't active until"),
            "{errors}"
        );
    }

    /// A deployment held at boot is still the one the person chose: it
    /// stays recorded active, so a later activation and the next boot try
    /// it again and list why, instead of forgetting it and leaving the
    /// extension undeployed (which a boot that deployed sources then
    /// replaced with whatever the source held).
    #[tokio::test]
    async fn a_deployment_held_at_boot_stays_recorded_and_is_reported_again() {
        let (dir, svc) = unbooted();
        write(dir.path(), ".oxplow/project.yaml", "extensions: [acme]\n");
        let broken = "manifest: 2\nname: acme\nnot_a_key: 1\nintent:\n  purpose: test\n";
        active_deployment(
            &svc,
            "acme",
            "00000000000000cc",
            &[("extension.yaml", broken.into())],
        )
        .await;
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        assert!(running(&svc, "acme").is_none());
        assert_eq!(active_hashes(&svc, "acme").await, vec!["00000000000000cc"]);
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        assert_eq!(active_hashes(&svc, "acme").await, vec!["00000000000000cc"]);
        // The next boot starts from the same record and says the same.
        svc.extension_catalog.set_live(Default::default());
        let missing = start(&svc).await.unwrap();
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(
            svc.extension_catalog.live().get("acme").map(String::as_str),
            Some("00000000000000cc")
        );
    }

    /// Everything in an extension's folder is copied into each deployment,
    /// so a big one deploys and says how big it is.
    #[tokio::test]
    async fn a_big_extension_deploys_with_a_warning() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let dir = source(svc, "One");
        std::fs::write(dir.join("data.bin"), vec![0u8; 26 * 1024 * 1024]).unwrap();
        let deployed = deploy(svc, "acme", true).await.unwrap();
        assert!(
            deployed
                .warnings
                .iter()
                .any(|w| w.contains("acme") && w.contains("26.0 MiB")),
            "{:?}",
            deployed.warnings
        );
        std::fs::remove_file(dir.join("data.bin")).unwrap();
        let small = deploy(svc, "acme", true).await.unwrap();
        assert!(
            !small.warnings.iter().any(|w| w.contains("MiB")),
            "{:?}",
            small.warnings
        );
    }

    /// What a loaded extension reports as worth fixing comes back from its
    /// deploy, so the agent that deployed it reads it: `acme` has no
    /// `intent.examples`.
    #[tokio::test]
    async fn a_deploy_returns_the_extension_s_warnings() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        let deployed = deploy(svc, "acme", true).await.unwrap();
        assert!(
            deployed
                .warnings
                .iter()
                .any(|w| w.contains("`intent.examples` is empty")),
            "{:?}",
            deployed.warnings
        );
    }

    /// A restart starts from the recorded deployments: the core baseline
    /// published first doesn't drop them.
    #[tokio::test]
    async fn a_restart_keeps_what_was_deployed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        source(svc, "One");
        deploy(svc, "acme", true).await.unwrap();
        source(svc, "Uno");
        svc.extension_catalog.set_live(Default::default());
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        assert_eq!(page_title(svc).as_deref(), Some("One"));
    }

    /// The bundled extensions run their shipped deployments from the store.
    #[tokio::test]
    async fn bundled_extensions_run_from_the_store() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        for b in crate::bundled_extensions::BUNDLED {
            let ext = svc
                .extension_catalog
                .unpinned(&root, "a test reads the catalog")
                .iter()
                .find(|e| e.name == b.name)
                .cloned()
                .unwrap();
            let Deployment::Shipped { hash } = &ext.deployment else {
                panic!("{} is {:?}", b.name, ext.deployment);
            };
            assert_eq!(
                Path::new(&ext.path),
                extension_store::deployment_dir(&root, b.name, hash)
            );
        }
    }
}
