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
/// person's layer when nothing turns it on yet.
pub async fn deploy(
    svc: &Arc<Services>,
    name: &str,
    root: &Path,
    from_stream: Option<i64>,
) -> Result<Deployed, DomainError> {
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
    let enabled_now = turn_on(svc, name)?;
    if svc.extension_catalog.live().get(name) == target.as_ref() {
        return Ok(Deployed {
            name: name.to_string(),
            hash,
            already: true,
            warnings: Vec::new(),
        });
    }
    if !loaded.errors.is_empty() {
        undo_turn_on(svc, name, enabled_now);
        return Err(invalid(format!(
            "extension `{name}` doesn't load:\n{}",
            loaded.errors.join("\n")
        )));
    }
    if target.is_some() {
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
        if let Err(e) = copied {
            undo_turn_on(svc, name, enabled_now);
            return Err(storage(format!("deploy `{name}`: {e}")));
        }
        let row = DeploymentRow {
            name: name.to_string(),
            hash: hash.clone(),
            deployed_at: Timestamp::now().to_string(),
            from_stream,
            source: source.to_string_lossy().into_owned(),
            active: false,
        };
        svc.db
            .transaction(move |tx| store_rows::record_tx(tx, &row))
            .await?;
    }
    let changes = LiveChanges::from([(name.to_string(), target)]);
    if let Err(error) = activate(svc, &changes).await {
        undo_turn_on(svc, name, enabled_now);
        return Err(error);
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
) -> Result<Deployed, DomainError> {
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
                    _ => return Err(invalid(format!("`{name}` has no deployment `{prefix}`"))),
                },
                _ => {
                    return Err(invalid(format!(
                        "`{prefix}` names more than one deployment of `{name}`"
                    )))
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
                    )))
                }
            }
        }
    };
    if let Some(hash) = &hash {
        if !extension_store::deployment_dir(&svc.layout.project_dir, name, hash).is_dir() {
            return Err(invalid(format!(
                "deployment `{hash}` of `{name}` is no longer in the store"
            )));
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
) -> Result<(), DomainError> {
    valid(name)?;
    if crate::bundled_extensions::is_required(name) {
        return Err(invalid(format!(
            "`{name}` is part of oxplow; it is always on"
        )));
    }
    let mut settings = oxplow_config::extension_settings(&svc.layout.project_dir);
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
        deploy(svc, name, &root, None).await?;
        return Ok(());
    }
    activate(svc, &LiveChanges::new()).await
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
/// folder and, since `*` matches a file at any depth, everything in it.
fn share_lines(name: &str) -> [String; 4] {
    [
        "!extensions/".to_string(),
        "extensions/*".to_string(),
        format!("!extensions/{name}/"),
        format!("!extensions/{name}/**"),
    ]
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
    for line in share_lines(name) {
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
    let [_, _, folder, files] = share_lines(name);
    let text = std::fs::read_to_string(&ignore).unwrap_or_default();
    let lines: Vec<&str> = text
        .lines()
        .filter(|l| *l != folder && *l != files)
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
        match stage(svc, &name, &source, None).await {
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
        let target = stage(svc, &name, &source, None)
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

    /// `v_extension` says what each extension declares it may reach:
    /// `{}` for the baseline.
    #[tokio::test]
    async fn v_extension_lists_each_extensions_permissions() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let root = svc.layout.project_dir.clone();
        source(svc, "One");
        deploy(svc, "acme", &root, None).await.unwrap();
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
                    oxplow_db::SqlCell::Text("{\"ai\":true}".into())
                ],
            ]
        );
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

        let deployed = deploy(svc, "acme", &root, Some(1)).await.unwrap();
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

        let again = deploy(svc, "acme", &root, Some(1)).await.unwrap();
        assert!(again.already);
        assert_eq!(again.hash, deployed.hash);

        // An edit runs nothing until it is deployed, whatever activates.
        source(svc, "Uno");
        svc.extension_activation
            .activate(svc.clone())
            .await
            .unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
        let second = deploy(svc, "acme", &root, Some(1)).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("Uno"));
        assert_eq!(
            rows(svc).await,
            [(second.hash.clone(), true), (deployed.hash.clone(), false)]
        );

        // Back to the one before, and forward again by its hash.
        rollback(svc, "acme", None).await.unwrap();
        assert_eq!(lens_title(svc).as_deref(), Some("One"));
        rollback(svc, "acme", Some(&second.hash[..6]))
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
        let err = deploy(svc, "acme", &root, None).await.unwrap_err();
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
        let deployed = deploy(svc, "acme", &root, None).await.unwrap();

        set_enabled(svc, "acme", false).await.unwrap();
        assert_eq!(lens_title(svc), None);
        set_enabled(svc, "acme", true).await.unwrap();
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
        deploy(svc, "acme", &root, None).await.unwrap();
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
        let deployed = deploy(svc, "acme", &root, None).await.unwrap();
        assert!(
            deployed
                .warnings
                .iter()
                .any(|w| w.contains("acme") && w.contains("26.0 MiB")),
            "{:?}",
            deployed.warnings
        );
        std::fs::remove_file(dir.join("data.bin")).unwrap();
        let small = deploy(svc, "acme", &root, None).await.unwrap();
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
        deploy(svc, "acme", &root, None).await.unwrap();
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
