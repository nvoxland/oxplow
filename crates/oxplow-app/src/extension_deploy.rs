//! Deploying extensions (`.context/extensions.md` "Deployments"): what
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
use oxplow_domain::{DomainError, Timestamp};
use serde::Serialize;

use crate::extension_catalog::LiveChanges;
use crate::extension_store;
use crate::extensions::Permissions;
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
            DeployError::NeedsApproval { name, adds } => write!(
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
    b.files
        .iter()
        .find(|(rel, _)| *rel == "extension.yaml")
        .and_then(|(_, bytes)| {
            serde_yaml::from_slice::<crate::extensions::manifest_v2::ManifestV2>(bytes).ok()
        })
        .map(|m| m.permissions)
        .unwrap_or_default()
}

/// `name`'s deployment as it runs now, if one does.
fn running(svc: &Services, name: &str) -> Option<crate::extensions::Extension> {
    svc.extension_catalog.active().and_then(|snapshot| {
        snapshot
            .extensions()
            .iter()
            .find(|e| {
                e.name == name
                    && matches!(
                        e.deployment,
                        crate::extensions::Deployment::Deployed { .. }
                            | crate::extensions::Deployment::Shipped { .. }
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
        .or_else(|| crate::bundled_extensions::find(name).map(shipped_permissions))
        .unwrap_or_default()
}

/// The build command `name`'s running deployment was approved with.
fn active_build(svc: &Services, name: &str) -> Option<String> {
    running(svc, name).and_then(|e| e.build).map(|b| b.run)
}

/// What a deploy that runs a new or changed build command asks a person:
/// the command runs with their authority (`.context/extensions.md`
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
        })
    }
}

/// `name` as the active generation knows it, on or off.
fn active_entry(svc: &Services, name: &str) -> Option<crate::extensions::Extension> {
    svc.extension_catalog.active().and_then(|snapshot| {
        snapshot
            .extensions()
            .iter()
            .find(|e| e.name == name)
            .cloned()
    })
}

/// Each extension `name` requires is on (`.context/extensions.md`
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
    if crate::extensions::is_valid_name(name) {
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
            Err(error) => return Err(problems_of(error)),
        }
    }
}

fn problems_of(error: TxError<VocabularyErrors>) -> DomainError {
    match error {
        TxError::Aborted(problems) => invalid(
            problems
                .iter()
                .map(|(name, why)| format!("{name}: {}", why.join("; ")))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        TxError::Storage(error) => error,
    }
}

/// Where `name`'s source is, in the checkout at `root`, checked to hold a
/// manifest.
fn source_of(project: &Path, root: &Path, name: &str) -> Result<PathBuf, DomainError> {
    let dir = oxplow_config::extension_settings(project).source(root, name);
    if dir.join("extension.yaml").is_file() {
        Ok(dir)
    } else {
        Err(invalid(format!(
            "no source for extension `{name}`: {} has no extension.yaml",
            dir.display()
        )))
    }
}

/// The live-set change that runs `hash` for `name`: none for a bundled
/// extension whose shipped deployment has that hash.
fn target_of(name: &str, hash: &str) -> Option<String> {
    let shipped = crate::bundled_extensions::find(name)
        .map(|b| extension_store::files_hash(b.files.iter().copied()));
    (shipped.as_deref() != Some(hash)).then(|| hash.to_string())
}

/// Deploy `name` from its source in the checkout at `root`, made from
/// `from_stream` (`None`: oxplow itself, at startup). Turns it on in the
/// person's layer when nothing turns it on yet. A source that declares
/// permissions beyond what runs now goes live only `confirmed` by a
/// person; otherwise nothing changes and it says what it would add.
pub async fn deploy(
    svc: &Arc<Services>,
    name: &str,
    root: &Path,
    from_stream: Option<i64>,
    confirmed: bool,
) -> Result<Deployed, DeployError> {
    valid(name)?;
    let project = svc.layout.project_dir.clone();
    let source = source_of(&project, root, name)?;
    let (hash, size, loaded) = {
        let (source, owned) = (source.clone(), name.to_string());
        tokio::task::spawn_blocking(move || {
            let hash = extension_store::source_hash(&source)?;
            let size = extension_store::source_size(&source)?;
            Ok::<_, std::io::Error>((
                hash,
                size,
                crate::extensions::load_extension_at(&source, &owned),
            ))
        })
        .await
        .map_err(storage)?
        .map_err(|e| invalid(format!("extension `{name}`: {e}")))?
    };
    let target = target_of(name, &hash);
    let live_now = svc.extension_catalog.live().get(name) == target.as_ref();
    requirements_met(
        &oxplow_config::extension_settings(&project),
        name,
        &loaded.requires,
    )?;
    if !live_now {
        if !loaded.errors.is_empty() {
            return Err(invalid(format!(
                "extension `{name}` doesn't load:\n{}",
                loaded.errors.join("\n")
            ))
            .into());
        }
        approved(
            svc,
            name,
            &loaded.permissions,
            loaded.build.as_ref().map(|b| b.run.as_str()),
            &crate::extensions::shown(&project, &source),
            confirmed,
        )?;
    }
    let enabled_now = turn_on(svc, name)?;
    if live_now {
        return Ok(Deployed {
            name: name.to_string(),
            hash,
            already: true,
            warnings: Vec::new(),
        });
    }
    if target.is_some() {
        let built_before = extension_store::deployment_dir(&project, name, &hash).is_dir();
        if let (Some(build), false) = (&loaded.build, built_before) {
            let (dir, run, env) = (source.clone(), build.run.clone(), svc.providers.host_env());
            let ran = tokio::task::spawn_blocking(move || {
                crate::extension_build::run(&dir, &run, &env, crate::extension_build::TIMEOUT)
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
                crate::extensions::load_extension_at(&dir, &owned)
            })
            .await
            .map_err(storage)?;
            if !built.errors.is_empty() {
                extension_store::remove(&project, name, std::slice::from_ref(&hash));
                undo_turn_on(svc, name, enabled_now);
                return Err(invalid(format!(
                    "extension `{name}` doesn't load once built:\n{}",
                    built.errors.join("\n")
                ))
                .into());
            }
        }
        let row = DeploymentRow {
            name: name.to_string(),
            hash: hash.clone(),
            deployed_at: Timestamp::now().to_string(),
            from_stream,
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
        warnings: size_warnings(name, size),
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
                [] => match crate::bundled_extensions::find(name) {
                    Some(b)
                        if extension_store::files_hash(b.files.iter().copied())
                            .starts_with(prefix) =>
                    {
                        None
                    }
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
            match (previous, crate::bundled_extensions::find(name)) {
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
        crate::bundled_extensions::find(name)
            .map(|b| extension_store::files_hash(b.files.iter().copied()))
            .unwrap_or_default()
    });
    if live == hash {
        return Ok(Deployed {
            name: name.to_string(),
            hash: shown,
            already: true,
            warnings: Vec::new(),
        });
    }
    let target_row = hash
        .as_ref()
        .and_then(|hash| rows.iter().find(|r| &r.hash == hash));
    let target_permissions = match &hash {
        Some(_) => target_row
            .map(|r| r.permissions.clone())
            .unwrap_or_default(),
        None => crate::bundled_extensions::find(name)
            .map(shipped_permissions)
            .unwrap_or_default(),
    };
    let shown_source = target_row
        .map(|r| crate::extensions::shown(&svc.layout.project_dir, Path::new(&r.source)))
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
        && crate::bundled_extensions::find(name).is_none()
        && !svc.extension_catalog.live().contains_key(name);
    if undeployed {
        let root = svc.layout.project_dir.clone();
        deploy(svc, name, &root, None, confirmed).await?;
        return Ok(());
    }
    Ok(activate(svc, &LiveChanges::new()).await?)
}

/// Take `name` out of both lists, stop running it, and forget its
/// deployments and its stored state and settings. Its source stays.
pub async fn remove(svc: &Arc<Services>, name: &str) -> Result<(), DomainError> {
    valid(name)?;
    if crate::bundled_extensions::find(name).is_some() {
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
    if source != project.join(crate::extensions::EXTENSIONS_DIR).join(name) {
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
/// that runs it. No load check: activation holds one that doesn't load, as
/// at startup.
async fn stage(
    svc: &Arc<Services>,
    name: &str,
    source: &Path,
    from_stream: Option<i64>,
    permissions: Permissions,
) -> Result<Option<String>, DomainError> {
    let hash = {
        let (project, owned, source) = (
            svc.layout.project_dir.clone(),
            name.to_string(),
            source.to_path_buf(),
        );
        tokio::task::spawn_blocking(move || {
            let hash = extension_store::source_hash(&source)?;
            if target_of(&owned, &hash).is_some() {
                extension_store::copy_into_store(&project, &owned, &source, &hash)?;
            }
            Ok::<_, std::io::Error>(hash)
        })
        .await
        .map_err(storage)?
        .map_err(|e| storage(format!("stage `{name}`: {e}")))?
    };
    let Some(target) = target_of(name, &hash) else {
        return Ok(None);
    };
    let row = DeploymentRow {
        name: name.to_string(),
        hash: target.clone(),
        deployed_at: Timestamp::now().to_string(),
        from_stream,
        source: source.to_string_lossy().into_owned(),
        active: false,
        permissions,
        build_run: None,
    };
    svc.db
        .transaction(move |tx| store_rows::record_tx(tx, &row))
        .await?;
    Ok(Some(target))
}

/// Before the first activation: start the live set from the deployments
/// recorded as active whose folder is still in the store, then stage each
/// enabled extension that has none here from the main checkout's source.
/// What a startup deploy can't do becomes that extension's problem; the
/// rest start.
pub(crate) async fn start(svc: &Arc<Services>) -> Result<LiveChanges, DomainError> {
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
    let settings = oxplow_config::extension_settings(&project);
    let mut names: Vec<String> = settings.enabled().into_iter().map(|(n, _)| n).collect();
    names.extend(
        crate::bundled_extensions::BUNDLED
            .iter()
            .filter(|b| b.required)
            .map(|b| b.name.to_string()),
    );
    let mut changes = LiveChanges::new();
    for name in names {
        if svc.extension_catalog.live().contains_key(&name) {
            continue;
        }
        let source = settings.source(&project, &name);
        if !source.join("extension.yaml").is_file() {
            continue;
        }
        // One that would add permissions waits for a person to deploy it
        // (its listing says what for); a bundled one's baseline is what
        // ships.
        // One with a build waits too: nothing has approved its command.
        let loaded = crate::extensions::load_extension_at(&source, &name);
        let declared = loaded.permissions;
        let baseline = crate::bundled_extensions::find(&name)
            .map(shipped_permissions)
            .unwrap_or_default();
        if !declared.added_over(&baseline).is_empty() || loaded.build.is_some() {
            continue;
        }
        match stage(svc, &name, &source, None, declared).await {
            Ok(Some(target)) => {
                changes.insert(name, Some(target));
            }
            Ok(None) => {}
            Err(error) => tracing::warn!(%name, %error, "startup deploy failed"),
        }
    }
    Ok(changes)
}

/// For tests that write an extension's files and activate, as an agent
/// that wrote them would deploy them: every source in the project whose
/// deployment isn't the live one is staged, turned on for the person
/// unless something turned it off, and activated with the rest — one that
/// doesn't load is held, as at startup.
#[cfg(test)]
pub(crate) async fn stage_sources(svc: &Arc<Services>) -> LiveChanges {
    let project = svc.layout.project_dir.clone();
    let mut settings = oxplow_config::extension_settings(&project);
    let mut changes = LiveChanges::new();
    let mut turned_on = false;
    for name in crate::extensions::known_names(&project, &settings) {
        let source = settings.source(&project, &name);
        if !source.join("extension.yaml").is_file() || settings.personal.disable.contains(&name) {
            continue;
        }
        let declared = crate::extensions::load_extension_at(&source, &name).permissions;
        let target = stage(svc, &name, &source, None, declared)
            .await
            .expect("a source stages");
        if svc.extension_catalog.live().get(&name) != target.as_ref() {
            changes.insert(name.clone(), target);
        }
        if !settings.is_enabled(&name) && crate::bundled_extensions::find(&name).is_none() {
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
    use crate::extensions::Deployment;

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// `acme`'s source in the project, with one lens titled `title`.
    fn source(svc: &Services, title: &str) -> PathBuf {
        let dir = svc.layout.project_dir.join(".oxplow/extensions/acme");
        write(
            &dir,
            "extension.yaml",
            "manifest: 2\nname: acme\nintent:\n  purpose: test\n",
        );
        write(
            &dir,
            "lenses/one.yaml",
            &format!("title: {title}\nquery: \"SELECT 1 AS n\"\n"),
        );
        dir
    }

    fn lens_title(svc: &Services) -> Option<String> {
        svc.extension_catalog
            .find_lens(&svc.layout.project_dir, "acme/one")
            .ok()
            .map(|l| l.title)
    }

    fn acme(svc: &Services) -> Option<crate::extensions::Extension> {
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

    /// `acme`'s source declaring `permissions`, its lens titled `title`.
    fn source_with(svc: &Services, title: &str, permissions: &str) -> PathBuf {
        let dir = source(svc, title);
        write(
            &dir,
            "extension.yaml",
            &format!("manifest: 2\nname: acme\n{permissions}intent:\n  purpose: test\n"),
        );
        dir
    }

    const AI: &str = "permissions: { ai: true }\n";

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
        let root = svc.layout.project_dir.clone();
        source_with(svc, "One", AI);
        deploy(svc, "acme", &root, None, true).await.unwrap();
        source_with(svc, "Two", AI);
        deploy(svc, "acme", &root, None, false).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Two"));
        source_with(svc, "Three", "");
        deploy(svc, "acme", &root, None, false).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Three"));
    }

    /// A deploy that adds a permission asks before anything changes: it
    /// names what it adds, writes nothing, and goes live once confirmed.
    #[tokio::test]
    async fn a_deploy_that_adds_a_permission_pauses_until_confirmed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", &root, None, false).await.unwrap();
        source_with(svc, "Two", AI);
        assert_eq!(
            asked(deploy(svc, "acme", &root, None, false).await),
            vec!["ai — may call your AI provider with your key"]
        );
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
        assert_eq!(rows(svc).await.len(), 1);
        deploy(svc, "acme", &root, None, true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Two"));
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
        let root = svc.layout.project_dir.clone();
        source_with(svc, "One", AI);
        deploy(svc, "acme", &root, None, true).await.unwrap();
        source_with(svc, "Two", "");
        deploy(svc, "acme", &root, None, false).await.unwrap();
        assert_eq!(
            asked(rollback(svc, "acme", None, false).await),
            vec!["ai — may call your AI provider with your key"]
        );
        assert_eq!(lens_title(svc).as_deref(), Some("Two"));
        rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
    }

    /// `name`'s source in the project with a lens and the `extra` manifest
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
            "lenses/one.yaml",
            "title: T\nquery: \"SELECT 1 AS n\"\n",
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
        let e = deploy(svc, "acme", &root, None, true).await.unwrap_err();
        assert!(
            e.to_string()
                .contains("`acme` requires `tracker`, which is off (turn it on first)"),
            "{e}"
        );
        assert!(rows(svc).await.is_empty());
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        deploy(svc, "tracker", &root, None, true).await.unwrap();
        deploy(svc, "acme", &root, None, true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
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
        deploy(svc, "tracker", &root, None, true).await.unwrap();
        deploy(svc, "acme", &root, None, true).await.unwrap();
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
        let first = deploy(svc, "acme", &root, None, true).await.unwrap();
        let deployed = extension_store::deployment_dir(&root, "acme", &first.hash);
        assert!(deployed.join("dist/widget/index.html").is_file());
        assert!(deployed.join("src/widget.html").is_file());
        assert!(!deployed.join("node_modules").exists());
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
        let recorded = svc
            .db
            .read(|c| store_rows::list_tx(c, Some("acme")))
            .await
            .unwrap();
        assert_eq!(recorded[0].build_run.as_deref(), Some(BUILD));

        write(&dir, "dist/widget/index.html", "edited by hand");
        assert!(
            deploy(svc, "acme", &root, None, true)
                .await
                .unwrap()
                .already
        );
        source_built(svc, "One", &format!("{BUILD} && true"));
        let second = deploy(svc, "acme", &root, None, true).await.unwrap();
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
            asked(deploy(svc, "acme", &root, None, false).await),
            vec![build_line(BUILD)]
        );
        assert!(rows(svc).await.is_empty());
        assert!(!root.join(".oxplow/extensions/acme/dist").exists());
        deploy(svc, "acme", &root, None, true).await.unwrap();
        source_built(svc, "Two", BUILD);
        deploy(svc, "acme", &root, None, false).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Two"));
        let changed = format!("{BUILD} && true");
        source_built(svc, "Three", &changed);
        assert_eq!(
            asked(deploy(svc, "acme", &root, None, false).await),
            vec![build_line(&changed)]
        );
        deploy(svc, "acme", &root, None, true).await.unwrap();
        assert_eq!(
            asked(rollback(svc, "acme", None, false).await),
            vec![build_line(BUILD)]
        );
        rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Two"));
    }

    /// A build that fails deploys nothing: not turned on, no row, nothing
    /// in the store, and the error says why.
    #[tokio::test]
    async fn a_failing_build_changes_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source_built(svc, "One", "echo broken >&2; exit 3");
        let e = deploy(svc, "acme", &root, None, true)
            .await
            .unwrap_err()
            .to_string();
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
        let e = deploy(svc, "acme", &root, None, true)
            .await
            .unwrap_err()
            .to_string();
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
        deploy(svc, "acme", &root, None, true).await.unwrap();
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

    /// At startup a built extension with no deployment here waits for a
    /// person, its listing naming the build it would run.
    #[tokio::test]
    async fn startup_leaves_a_built_extension_undeployed_with_the_build_line_pending() {
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

    /// At startup an extension that is on but undeployed deploys by itself
    /// only when it asks for nothing; one that declares permissions waits,
    /// and says what for.
    #[tokio::test]
    async fn startup_leaves_a_declaring_extension_undeployed_and_says_what_it_needs() {
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
        assert_eq!(lens_title(svc), None);
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
        assert_eq!(lens_title(svc), None);
        let id: i64 = proposal.strip_prefix("proposal:").unwrap().parse().unwrap();
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
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
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
            .files
        {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        write(
            &dir,
            "lenses/extra.yaml",
            "title: Extra\nquery: \"SELECT 1 AS n\"\n",
        );
        let deployed = deploy(svc, "oxplow-bundled", &root, None, false)
            .await
            .unwrap();
        assert!(!deployed.already);
    }

    /// `v_extension` says what each extension declares it may reach:
    /// `{}` for the baseline.
    #[tokio::test]
    async fn v_extension_lists_each_extensions_permissions() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", &root, None, true).await.unwrap();
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
                    oxplow_db::SqlCell::Text("{}".into())
                ],
                vec![
                    oxplow_db::SqlCell::Text("oxplow-foundation".into()),
                    oxplow_db::SqlCell::Text("{\"files\":\"read\",\"ai\":true}".into())
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
        let root = svc.layout.project_dir.clone();
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
        deploy(svc, "acme", &root, None, true).await.unwrap();
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

    /// The Extensions page lists each extension and offers Deploy only
    /// where it would change something: a source that moved on.
    #[tokio::test]
    async fn the_extensions_page_offers_deploy_only_when_the_source_moved_on() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", &root, None, true).await.unwrap();
        let acme = || async {
            let run = crate::extensions::run_lens(
                &svc.sql,
                &svc.extension_catalog,
                &root,
                "oxplow-foundation/extensions-list",
                Default::default(),
                &crate::extensions::LensContext::default(),
            )
            .await
            .unwrap();
            let col = |name: &str| run.result.columns.iter().position(|c| c == name).unwrap();
            let row = run
                .result
                .rows
                .iter()
                .find(|r| r[col("name")] == oxplow_db::SqlCell::Text("acme".into()))
                .unwrap()
                .clone();
            (
                row[col("source_state")].clone(),
                row[col("can_deploy")].clone(),
                row[col("can_disable")].clone(),
            )
        };
        let text = |t: &str| oxplow_db::SqlCell::Text(t.into());
        let int = |n| oxplow_db::SqlCell::Int(n);
        assert_eq!(acme().await, (text(""), int(0), int(1)));
        source(svc, "Two");
        assert_eq!(acme().await, (text("out of date"), int(1), int(1)));
    }

    /// The Extensions page's row actions run their commands from inside a
    /// lens action's read: disabling, enabling and removing there activate
    /// all the same.
    #[tokio::test]
    async fn the_extensions_page_row_actions_disable_enable_and_remove() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", &root, None, true).await.unwrap();
        let press = |action: &str| crate::lens_actions::LensActionCall {
            lens_id: "oxplow-foundation/extensions-list".into(),
            action_id: action.into(),
            params: Default::default(),
            row: Some(
                [
                    ("name".to_string(), oxplow_db::SqlCell::Text("acme".into())),
                    (format!("can_{action}"), oxplow_db::SqlCell::Int(1)),
                ]
                .into(),
            ),
            input: None,
            on_behalf_of: oxplow_domain::Actor::Human,
            confirmed: true,
        };
        for action in ["disable", "enable", "remove"] {
            crate::lens_actions::run_lens_action(
                svc,
                &root,
                press(action),
                &crate::extensions::LensContext::default(),
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
        assert_eq!(lens_title(svc), None);
        assert_eq!(acme(svc).unwrap().deployment, Deployment::Undeployed);

        let deployed = deploy(svc, "acme", &root, Some(1), true).await.unwrap();
        assert!(!deployed.already);
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
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

        let again = deploy(svc, "acme", &root, Some(1), true).await.unwrap();
        assert!(again.already);
        assert_eq!(again.hash, deployed.hash);

        // An edit runs nothing until it is deployed, whatever activates.
        source(svc, "Uno");
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
        let second = deploy(svc, "acme", &root, Some(1), true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Uno"));
        assert_eq!(
            rows(svc).await,
            [(second.hash.clone(), true), (deployed.hash.clone(), false)]
        );

        // Back to the one before, and forward again by its hash.
        rollback(svc, "acme", None, true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
        rollback(svc, "acme", Some(&second.hash[..6]), true)
            .await
            .unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Uno"));
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
        let err = deploy(svc, "acme", &root, None, true).await.unwrap_err();
        assert!(err.to_string().contains("doesn't load"), "{err}");
        assert!(!oxplow_config::extension_settings(&root).is_enabled("acme"));
        assert!(rows(svc).await.is_empty());
        assert_eq!(lens_title(svc), None);
    }

    /// Off for the person stops it; removing forgets its deployments and
    /// keeps its source.
    #[tokio::test]
    async fn turning_off_and_removing_keep_the_source() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let dir = source(svc, "One");
        let deployed = deploy(svc, "acme", &root, None, true).await.unwrap();

        set_enabled(svc, "acme", false, true).await.unwrap();
        assert_eq!(lens_title(svc), None);
        set_enabled(svc, "acme", true, true).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));

        remove(svc, "acme").await.unwrap();
        assert_eq!(lens_title(svc), None);
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
        deploy(svc, "acme", &root, None, true).await.unwrap();
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

    /// At startup an enabled extension with no deployment here deploys
    /// from its source; one listed with neither is listed as not here.
    #[tokio::test]
    async fn startup_deploys_what_is_on_and_lists_what_is_not_here() {
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
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
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
            from_stream: None,
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
        let active = svc.extension_catalog.active().unwrap();
        let foundation = active.extensions().iter().find(|e| e.name == name).unwrap();
        assert!(
            matches!(foundation.deployment, Deployment::Shipped { .. }),
            "{:?}",
            foundation.deployment
        );
        assert!(foundation.errors.is_empty(), "{:?}", foundation.errors);
        let problems = svc.extension_activation.problems();
        assert!(
            problems
                .get(name)
                .is_some_and(|p| p.iter().any(|m| m.contains("the shipped version runs"))),
            "{problems:?}"
        );
        assert!(!svc.extension_catalog.live().contains_key(name));
    }

    /// Everything in an extension's folder is copied into each deployment,
    /// so a big one deploys and says how big it is.
    #[tokio::test]
    async fn a_big_extension_deploys_with_a_warning() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        let dir = source(svc, "One");
        std::fs::write(dir.join("data.bin"), vec![0u8; 26 * 1024 * 1024]).unwrap();
        let deployed = deploy(svc, "acme", &root, None, true).await.unwrap();
        assert!(
            deployed
                .warnings
                .iter()
                .any(|w| w.contains("acme") && w.contains("26.0 MiB")),
            "{:?}",
            deployed.warnings
        );
        std::fs::remove_file(dir.join("data.bin")).unwrap();
        let small = deploy(svc, "acme", &root, None, true).await.unwrap();
        assert!(small.warnings.is_empty(), "{:?}", small.warnings);
    }

    /// A restart starts from the recorded deployments: the core baseline
    /// published first doesn't drop them.
    #[tokio::test]
    async fn a_restart_keeps_what_was_deployed() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", &root, None, true).await.unwrap();
        source(svc, "Uno");
        svc.extension_catalog.set_live(Default::default());
        svc.extension_activation
            .initialize(svc.clone())
            .await
            .unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
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
