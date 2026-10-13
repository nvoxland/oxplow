//! Declared cross-extension imports and public contracts end to end: two
//! extensions in a project's `oxplow/extensions/`, activated together.

use oxplow_domain::DomainError;
use std::{path::Path, sync::Arc};

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

#[derive(Clone)]
struct Ext<'a> {
    name: &'a str,
    /// The `items` model's contract version, and the version an export of it declares.
    model_version: u32,
    /// `(visibility, export version)` of its `items` model.
    export: Option<(&'a str, u32)>,
    /// `(producer, version, optional)` read as `import_ref('dep')`.
    import: Option<(&'a str, u32, bool)>,
    shared: bool,
}

impl<'a> Ext<'a> {
    fn new(name: &'a str) -> Self {
        Ext {
            name,
            model_version: 1,
            export: None,
            import: None,
            shared: false,
        }
    }
    /// Export `items` at its own model version.
    fn exports(mut self, visibility: &'a str) -> Self {
        self.export = Some((visibility, self.model_version));
        self
    }
    fn at_version(mut self, version: u32) -> Self {
        self.model_version = version;
        if let Some((_, v)) = self.export.as_mut() {
            *v = version;
        }
        self
    }
    fn imports(mut self, producer: &'a str, version: u32, optional: bool) -> Self {
        self.import = Some((producer, version, optional));
        self
    }
    fn shared(mut self) -> Self {
        self.shared = true;
        self
    }
}

/// Every extension has `items` (`v_<name>_items`: over the work items, or
/// over its import), an always-available `plain` model and, when it imports,
/// a `seen` lens reading the import.
fn put(root: &Path, e: &Ext) {
    put_in(&root.join(format!("oxplow/extensions/{}", e.name)), e);
}

fn put_in(dir: &Path, e: &Ext) {
    let name = e.name;
    let version = e.model_version;
    let mut manifest = format!("manifest: 2\nname: {name}\n");
    if e.shared {
        manifest += "sharing: shared\nengine: \">=0.1\"\nversion: 0.1.0\n";
    }
    manifest += &format!(
        "intent:\n  purpose: p\nmodels:\n  - name: items\n    version: {version}\n    description: Items.\n    columns:\n      - {{ name: ref, type: TEXT, doc: Work item ref. }}\n  - name: plain\n    version: 1\n    description: Plain.\n    columns:\n      - {{ name: ref, type: TEXT, doc: Work item ref. }}\n"
    );
    if let Some((visibility, version)) = e.export {
        manifest += &format!(
            "exports:\n  - {{ contribution: {name}/model/items, version: {version}, visibility: {visibility} }}\n"
        );
    }
    let mut sql = "SELECT ref FROM ref('work_item')";
    if let Some((target, version, optional)) = e.import {
        manifest += &format!(
            "imports:\n  - {{ id: dep, contribution: {target}/model/items, version: {version}, optional: {optional} }}\n"
        );
        sql = "SELECT ref FROM import_ref('dep')";
        write(
            dir,
            "lenses/seen.yaml",
            "title: Seen\nquery: SELECT ref FROM import_ref('dep')\n",
        );
    }
    write(dir, "extension.yaml", &manifest);
    write(dir, "models/items.sql", sql);
    write(dir, "models/plain.sql", "SELECT ref FROM ref('work_item')");
}

async fn project(exts: &[Ext<'_>]) -> (tempfile::TempDir, Arc<crate::Services>) {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    for e in exts {
        put(dir.path(), e);
    }
    let svc = crate::Services::in_memory(dir.path()).await.unwrap();
    (dir, svc)
}

/// Activate; a held extension is not an activation failure.
async fn activate(svc: &Arc<crate::Services>) {
    svc.extension_activation
        .activate(svc.clone())
        .await
        .expect("the project activates");
}

async fn readable(svc: &crate::Services, view: &str) -> bool {
    svc.sql
        .query_sql(&format!("SELECT ref FROM {view}"), vec![], None)
        .await
        .is_ok()
}

fn problem(svc: &crate::Services, extension: &str) -> String {
    svc.extension_activation
        .problems()
        .get(extension)
        .map(|lines| lines.join("\n"))
        .unwrap_or_default()
}

async fn lens(svc: &crate::Services, id: &str) -> Result<crate::extensions::LensRun, DomainError> {
    crate::extensions::run_lens(
        &svc.sql,
        &svc.extension_catalog,
        &svc.layout.project_dir,
        id,
        Default::default(),
        &Default::default(),
    )
    .await
}

fn disable(root: &Path, names: &str) {
    write(
        root,
        ".oxplow/project.yaml",
        &format!("extensions:\n  disabled: [{names}]\n"),
    );
}

/// A stable model at a matching version is read by its consumer's model and lens.
#[tokio::test]
async fn a_consumer_of_a_compatible_stable_model_activates_and_reads_it() {
    let (_dir, svc) = project(&[
        Ext::new("prod").at_version(2).exports("stable"),
        Ext::new("cons").imports("prod", 2, false),
    ])
    .await;
    activate(&svc).await;
    assert_eq!(svc.extension_activation.problems().len(), 0);
    assert!(readable(&svc, "v_prod_items").await);
    assert!(readable(&svc, "v_cons_items").await);
    lens(&svc, "cons/seen")
        .await
        .expect("the lens reads the import");
    let cons = svc.listed_extensions(&svc.layout.project_dir).await;
    let cons = cons.iter().find(|e| e.name == "cons").unwrap();
    assert!(cons.errors.is_empty(), "{:?}", cons.errors);
}

#[tokio::test]
async fn a_breaking_version_mismatch_blocks_only_the_consumer() {
    let (_dir, svc) = project(&[
        Ext::new("prod").at_version(2).exports("stable"),
        Ext::new("cons").imports("prod", 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    let why = problem(&svc, "cons");
    assert!(
        why.contains("`dep`") && why.contains("prod/model/items"),
        "{why}"
    );
    assert!(!readable(&svc, "v_cons_items").await);
    assert!(!readable(&svc, "v_cons_plain").await);
    assert!(readable(&svc, "v_prod_items").await);
    assert!(readable(&svc, "v_other_items").await);
    assert!(problem(&svc, "prod").is_empty() && problem(&svc, "other").is_empty());
}

#[tokio::test]
async fn a_missing_required_producer_blocks_only_the_consumer() {
    let (_dir, svc) = project(&[
        Ext::new("cons").imports("gone", 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    let why = problem(&svc, "cons");
    assert!(
        why.contains("`dep`") && why.contains("gone/model/items") && why.contains("isn't here"),
        "{why}"
    );
    assert!(!readable(&svc, "v_cons_plain").await);
    assert!(readable(&svc, "v_other_items").await);
    assert!(problem(&svc, "other").is_empty());
}

/// Whatever blocks a consumer also blocks the consumers of that consumer.
#[tokio::test]
async fn a_blocked_consumer_cannot_supply_its_own_consumers() {
    let (_dir, svc) = project(&[
        Ext::new("mid").imports("gone", 1, false).exports("stable"),
        Ext::new("leaf").imports("mid", 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    assert!(!problem(&svc, "mid").is_empty());
    assert!(!problem(&svc, "leaf").is_empty());
    assert!(!readable(&svc, "v_leaf_plain").await);
    assert!(readable(&svc, "v_other_items").await);
}

/// An optional import of a missing or disabled producer withholds only what
/// reads it.
#[tokio::test]
async fn an_optional_import_of_a_missing_producer_degrades() {
    let (_dir, svc) = project(&[Ext::new("cons").imports("gone", 1, true)]).await;
    activate(&svc).await;
    assert!(svc.extension_activation.problems().is_empty());
    assert!(readable(&svc, "v_cons_plain").await);
    assert!(!readable(&svc, "v_cons_items").await);
    assert!(lens(&svc, "cons/seen").await.is_err());
}

#[tokio::test]
async fn an_optional_import_of_a_disabled_producer_degrades() {
    let (dir, svc) = project(&[
        Ext::new("prod").exports("stable"),
        Ext::new("cons").imports("prod", 1, true),
    ])
    .await;
    disable(dir.path(), "prod");
    activate(&svc).await;
    assert!(svc.extension_activation.problems().is_empty());
    assert!(readable(&svc, "v_cons_plain").await);
    assert!(!readable(&svc, "v_cons_items").await);
    assert!(!readable(&svc, "v_prod_items").await);
}

#[tokio::test]
async fn a_cyclic_import_is_diagnosed_deterministically() {
    let build = || async {
        project(&[
            Ext::new("a").exports("stable").imports("b", 1, false),
            Ext::new("b").exports("stable").imports("a", 1, false),
            Ext::new("other"),
        ])
        .await
    };
    let mut seen = Vec::new();
    for _ in 0..2 {
        let (_dir, svc) = build().await;
        activate(&svc).await;
        assert!(readable(&svc, "v_other_items").await);
        assert!(!readable(&svc, "v_a_plain").await && !readable(&svc, "v_b_plain").await);
        seen.push((problem(&svc, "a"), problem(&svc, "b")));
    }
    assert_eq!(seen[0], seen[1]);
    assert!(seen[0].0.contains("a -> b -> a"), "{}", seen[0].0);
    assert!(seen[0].1.contains("b -> a -> b"), "{}", seen[0].1);
}

#[tokio::test]
async fn a_private_export_is_not_importable() {
    let (_dir, svc) = project(&[
        Ext::new("prod").exports("private"),
        Ext::new("cons").imports("prod", 1, false),
    ])
    .await;
    activate(&svc).await;
    let why = problem(&svc, "cons");
    assert!(
        why.contains("`dep`") && why.contains("prod doesn't export it"),
        "{why}"
    );
    assert!(readable(&svc, "v_prod_items").await);
    assert!(!readable(&svc, "v_cons_plain").await);
}

#[tokio::test]
async fn an_unexported_model_is_not_importable() {
    let (_dir, svc) =
        project(&[Ext::new("prod"), Ext::new("cons").imports("prod", 1, false)]).await;
    activate(&svc).await;
    assert!(problem(&svc, "cons").contains("`dep`"));
    assert!(readable(&svc, "v_prod_items").await);
}

/// An experimental export serves private consumers only.
#[tokio::test]
async fn an_experimental_export_is_refused_to_a_sconsumer() {
    let (_dir, svc) = project(&[
        Ext::new("prod").exports("experimental"),
        Ext::new("scons").imports("prod", 1, false).shared(),
        Ext::new("pcons").imports("prod", 1, false),
    ])
    .await;
    activate(&svc).await;
    let why = problem(&svc, "scons");
    assert!(
        why.contains("`dep`") && why.contains("experimental"),
        "{why}"
    );
    assert!(problem(&svc, "pcons").is_empty());
    assert!(readable(&svc, "v_pcons_items").await);
    assert!(!readable(&svc, "v_scons_plain").await);
}

/// Disabling the producer of a running consumer withdraws it, or degrades
/// it when the import is optional, at the next activation.
#[tokio::test]
async fn disabling_the_producer_withdraws_a_required_consumer_and_degrades_an_optional_one() {
    let (dir, svc) = project(&[
        Ext::new("prod").exports("stable"),
        Ext::new("must").imports("prod", 1, false),
        Ext::new("may").imports("prod", 1, true),
    ])
    .await;
    activate(&svc).await;
    assert!(svc.extension_activation.problems().is_empty());
    for view in ["v_prod_items", "v_must_items", "v_may_items"] {
        assert!(readable(&svc, view).await, "{view}");
    }
    disable(dir.path(), "prod");
    activate(&svc).await;
    assert!(!readable(&svc, "v_prod_items").await);
    // Required: gone, though its last version was working; the reason names the owner.
    assert!(!readable(&svc, "v_must_items").await);
    assert!(!readable(&svc, "v_must_plain").await);
    assert!(
        problem(&svc, "must").contains("prod"),
        "{:?}",
        svc.extension_activation.problems()
    );
    // Optional: only what reads the import goes.
    assert!(!readable(&svc, "v_may_items").await);
    assert!(readable(&svc, "v_may_plain").await);
    assert!(problem(&svc, "may").is_empty());
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

/// Updating a producer to a version a consumer does not import is refused
/// before any file is replaced; a compatible update goes through.
#[test]
fn an_update_that_breaks_a_consumers_import_is_refused() {
    let repo = tempfile::tempdir().unwrap();
    put_in(repo.path(), &Ext::new("prod").exports("stable"));
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(repo.path(), &["add", "."]);
    git(repo.path(), &["commit", "-q", "-m", "v1"]);
    let url = repo.path().to_string_lossy().to_string();
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let sha = git(repo.path(), &["rev-parse", "HEAD"]);
    crate::extensions::install_extension(root, root, &url, None, &sha).unwrap();
    put(root, &Ext::new("cons").imports("prod", 1, false));
    let manifest = root.join("oxplow/extensions/prod/extension.yaml");
    let installed = std::fs::read_to_string(&manifest).unwrap();

    put_in(
        repo.path(),
        &Ext::new("prod").at_version(2).exports("stable"),
    );
    git(repo.path(), &["commit", "-q", "-am", "v2"]);
    let sha = git(repo.path(), &["rev-parse", "HEAD"]);
    let err = crate::extensions::update_extension(root, root, "prod", &sha)
        .unwrap_err()
        .to_string();
    assert!(err.contains("cons") && err.contains("dep"), "{err}");
    assert_eq!(std::fs::read_to_string(&manifest).unwrap(), installed);

    // A consumer on the new contract lets the same update through.
    put(root, &Ext::new("cons").imports("prod", 2, false));
    crate::extensions::update_extension(root, root, "prod", &sha).unwrap();
    assert!(std::fs::read_to_string(&manifest)
        .unwrap()
        .contains("version: 2"));
}
