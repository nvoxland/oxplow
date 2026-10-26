//! Declared cross-extension imports and public contracts end to end: two
//! extensions in a project's `.oxplow/extensions/`, activated together.

use oxplow_domain::uri::build::keyed;

/// The URI naming `owner`'s `items` model.
fn model_ref(owner: &str) -> String {
    keyed("oxplow/model", &format!("{owner}/items"))
}
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
    /// `(producer, version, optional)` read as `import_ref('dep')`; no
    /// version binds whatever the producer exports.
    import: Option<(&'a str, Option<u32>, bool)>,
    /// Other contributions it declares, one of each kind asked for.
    declares: Vec<Kind>,
    /// `(kind, visibility)` exports of those, at contract version 1.
    kind_exports: Vec<(Kind, &'a str)>,
    /// `(binding, producer, kind, version, optional)` imports of another
    /// extension's contribution of that kind. A `guarded` command of its
    /// own is withheld with any of them.
    kind_imports: Vec<(&'a str, &'a str, Kind, u32, bool)>,
    /// Mount the `seen` page in a slot (only when it imports a model).
    mounted: bool,
}

/// The contribution kinds a producer can declare, each named `thing`.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Collector,
    EventType,
    Command,
}

impl Kind {
    /// How `owner` declares and exports its `thing` of this kind.
    fn reference(self, owner: &str) -> String {
        match self {
            Kind::Collector => keyed("oxplow/collector", &format!("{owner}/thing")),
            Kind::EventType => keyed(
                "oxplow/event_type",
                &format!("{}.thing", owner.replace('-', "_")),
            ),
            Kind::Command => keyed("oxplow/command", &format!("{owner}.thing.get")),
        }
    }
}

impl<'a> Ext<'a> {
    fn new(name: &'a str) -> Self {
        Ext {
            name,
            model_version: 1,
            export: None,
            import: None,
            declares: Vec::new(),
            kind_exports: Vec::new(),
            kind_imports: Vec::new(),
            mounted: false,
        }
    }
    /// Declare a `thing` of `kind`.
    fn declares(mut self, kind: Kind) -> Self {
        self.declares.push(kind);
        self
    }
    /// Export its `thing` of `kind`.
    fn exports_kind(mut self, kind: Kind, visibility: &'a str) -> Self {
        self.kind_exports.push((kind, visibility));
        self
    }
    /// Import `producer`'s `thing` of `kind` as `binding`.
    fn imports_kind(
        mut self,
        binding: &'a str,
        producer: &'a str,
        kind: Kind,
        version: u32,
        optional: bool,
    ) -> Self {
        self.kind_imports
            .push((binding, producer, kind, version, optional));
        self
    }
    /// Mount its `seen` page in a slot.
    fn mounted(mut self) -> Self {
        self.mounted = true;
        self
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
        self.import = Some((producer, Some(version), optional));
        self
    }
    /// Import `producer`'s `items` at whatever version it exports.
    fn imports_any(mut self, producer: &'a str) -> Self {
        self.import = Some((producer, None, false));
        self
    }
}

/// Every extension has `items` (`v_<name>_items`: over the work items, or
/// over its import), an always-available `plain` model and, when it imports,
/// a `seen` page reading the import.
fn put(root: &Path, e: &Ext) {
    put_in(&root.join(format!(".oxplow/extensions/{}", e.name)), e);
}

fn put_in(dir: &Path, e: &Ext) {
    let name = e.name;
    let version = e.model_version;
    let mut manifest = format!("manifest: 2\nname: {name}\n");
    manifest += &format!(
        "intent:\n  purpose: p\nmodels:\n  - name: items\n    version: {version}\n    description: Items.\n    columns:\n      - {{ name: uri, type: TEXT, doc: Work item ref. }}\n  - name: plain\n    version: 1\n    description: Plain.\n    columns:\n      - {{ name: uri, type: TEXT, doc: Work item ref. }}\n"
    );
    let mut exports = String::new();
    if let Some((visibility, version)) = e.export {
        exports += &format!(
            "  - {{ contribution: {}, version: {version}, visibility: {visibility} }}\n",
            model_ref(name)
        );
    }
    for (kind, visibility) in &e.kind_exports {
        exports += &format!(
            "  - {{ contribution: {}, version: 1, visibility: {visibility} }}\n",
            kind.reference(name)
        );
    }
    if !exports.is_empty() {
        manifest += &format!("exports:\n{exports}");
    }
    let mut commands = String::new();
    for kind in &e.declares {
        match kind {
            Kind::Collector => {
                manifest += "permissions: { files: read }\ncollectors:\n  - id: thing\n    runtime: starlark\n    entry: collectors/thing.star\n    trigger: { on: [snapshot.taken] }\n    facts: [x.y]\n";
                write(
                    dir,
                    "collectors/thing.star",
                    "def transform(x):\n    return {\"facts\": []}\n",
                );
            }
            Kind::EventType => {
                let ns = name.replace('-', "_");
                manifest += &format!("event_types:\n  types:\n    - {{ type: {ns}.thing, v: 1, schema: thing.json, summary: A thing. }}\n");
                write(dir, "thing.json", "{\"type\":\"object\"}");
            }
            Kind::Command => commands += &command("thing.get"),
        }
    }
    let mut imports = String::new();
    let mut sql = "SELECT uri FROM ref('work_item')";
    if let Some((target, version, optional)) = e.import {
        let version = version
            .map(|v| format!(", version: {v}"))
            .unwrap_or_default();
        imports += &format!(
            "  - {{ id: dep, contribution: {}{version}, optional: {optional} }}\n",
            model_ref(target)
        );
        sql = "SELECT uri FROM import_ref('dep')";
        let params = if e.mounted {
            "params: [{ name: path }]\n"
        } else {
            ""
        };
        write(
            dir,
            "pages/seen.yaml",
            &format!(
                "title: Seen\n{params}data:\n  seen: {{ sql: \"SELECT uri FROM import_ref('dep')\" }}\n\
                 view: {{ viz: table, from: seen }}\n"
            ),
        );
        if e.mounted {
            manifest += "ui:\n  slots:\n    - { slot: diff.file.header, page: seen }\n";
        }
    }
    for (binding, producer, kind, version, optional) in &e.kind_imports {
        imports += &format!(
            "  - {{ id: {binding}, contribution: {}, version: {version}, optional: {optional} }}\n",
            kind.reference(producer)
        );
    }
    if !imports.is_empty() {
        manifest += &format!("imports:\n{imports}");
    }
    if !e.kind_imports.is_empty() {
        // A command of its own that depends on every kind import.
        commands += &command("own.act");
        let bindings: Vec<_> = e.kind_imports.iter().map(|i| i.0).collect();
        manifest += &format!(
            "import_guards:\n  - {{ contribution: {}, imports: [{}] }}\n",
            keyed("oxplow/command", &format!("{name}.own.act")),
            bindings.join(", ")
        );
    }
    if !commands.is_empty() {
        manifest = manifest.replace("models:\n", &format!("commands:\n{commands}models:\n"));
    }
    write(dir, "extension.yaml", &manifest);
    write(dir, "handlers/get.star", HANDLER);
    write(dir, "models/items.sql", sql);
    write(dir, "models/plain.sql", "SELECT uri FROM ref('work_item')");
}

const HANDLER: &str = "def transform(x):\n    return {\"commands\": [], \"result\": {}}\n";

fn command(local: &str) -> String {
    format!("  - name: {local}\n    summary: A command.\n    input_schema: {{ type: object }}\n    entry: handlers/get.star\n    access: read\n")
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
    crate::test_fixtures::activate_with_problems(svc).await;
}

async fn readable(svc: &crate::Services, view: &str) -> bool {
    svc.sql
        .query_sql(&format!("SELECT uri FROM {view}"), vec![], None)
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

/// Page `id` run as a person: an error when it isn't there (withheld)
/// or a loader of it failed.
async fn page(svc: &crate::Services, id: &str) -> Result<crate::pages::runner::PageRun, String> {
    use crate::pages::runner::{PageRunner, RunRequest, RunTarget};
    let run = PageRunner::of(svc)
        .run(RunRequest {
            target: RunTarget::Page(id.to_string()),
            params: Default::default(),
            ctx: Default::default(),
            viewer: oxplow_domain::Actor::Human,
        })
        .await
        .map_err(|e| e.to_string())?;
    match run.loaders.values().find_map(|l| l.error.clone()) {
        Some(error) => Err(error),
        None => Ok(run),
    }
}

fn disable(root: &Path, names: &str) {
    write(
        root,
        ".oxplow/personal.yaml",
        &format!("disableExtensions: [{names}]\n"),
    );
}

/// A stable model at a matching version is read by its consumer's model and page.
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
    page(&svc, "cons/seen")
        .await
        .expect("the page reads the import");
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
        why.contains("`dep`") && why.contains("oxplow:oxplow/model?model=prod/items"),
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
        why.contains("`dep`")
            && why.contains("oxplow:oxplow/model?model=gone/items")
            && why.contains("isn't here"),
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
    assert!(page(&svc, "cons/seen").await.is_err());
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
        // The messages name the deployment's folder, which sits in the
        // temp project.
        let here = _dir.path().to_string_lossy().to_string();
        seen.push((
            problem(&svc, "a").replace(&here, ""),
            problem(&svc, "b").replace(&here, ""),
        ));
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

/// The extension as the catalog lists it.
async fn listed(svc: &crate::Services, name: &str) -> crate::extensions::Extension {
    svc.listed_extensions(&svc.layout.project_dir)
        .await
        .into_iter()
        .find(|e| e.name == name)
        .unwrap_or_else(|| panic!("{name} is listed"))
}

async fn binding_available(svc: &crate::Services, extension: &str, id: &str) -> bool {
    listed(svc, extension)
        .await
        .public_contracts
        .bindings
        .iter()
        .find(|b| b.id == id)
        .unwrap_or_else(|| panic!("{extension} has no binding {id}"))
        .available
}

/// Whether the extension still has its guarded `act` command.
async fn has_act(svc: &crate::Services, extension: &str) -> bool {
    let act = format!("{extension}.own.act");
    listed(svc, extension)
        .await
        .commands
        .iter()
        .any(|c| c.name == act || c.name == "own.act")
}

const ALL_KINDS: [Kind; 3] = [Kind::Collector, Kind::EventType, Kind::Command];

/// A stable collector, event type or command is importable as well as a
/// model; the consumer's guarded command stays.
#[tokio::test]
async fn a_stable_contribution_of_every_kind_is_importable() {
    for kind in ALL_KINDS {
        let (_dir, svc) = project(&[
            Ext::new("prod").declares(kind).exports_kind(kind, "stable"),
            Ext::new("cons").imports_kind("dep", "prod", kind, 1, false),
        ])
        .await;
        activate(&svc).await;
        assert!(
            svc.extension_activation.problems().is_empty(),
            "{}",
            problem(&svc, "cons")
        );
        assert!(binding_available(&svc, "cons", "dep").await);
        assert!(has_act(&svc, "cons").await);
    }
}

/// A required import of a missing or unexported contribution of any kind
/// blocks the consumer alone.
#[tokio::test]
async fn a_missing_or_unexported_contribution_of_every_kind_blocks_the_consumer() {
    for kind in ALL_KINDS {
        let (_dir, svc) = project(&[
            Ext::new("prod").declares(kind),
            Ext::new("cons").imports_kind("dep", "prod", kind, 1, false),
            Ext::new("lost").imports_kind("dep", "gone", kind, 1, false),
            Ext::new("other"),
        ])
        .await;
        activate(&svc).await;
        assert!(
            problem(&svc, "cons").contains("`dep`"),
            "{}",
            problem(&svc, "cons")
        );
        assert!(
            problem(&svc, "lost").contains("`dep`"),
            "{}",
            problem(&svc, "lost")
        );
        assert!(!readable(&svc, "v_cons_plain").await);
        assert!(readable(&svc, "v_other_items").await);
        assert!(readable(&svc, "v_prod_items").await);
    }
}

/// An optional import of a contribution that isn't there withholds what is
/// guarded by it and nothing else, whatever the kind.
#[tokio::test]
async fn an_optional_import_of_every_kind_withholds_only_what_it_guards() {
    for kind in ALL_KINDS {
        let (_dir, svc) =
            project(&[Ext::new("cons").imports_kind("dep", "gone", kind, 1, true)]).await;
        activate(&svc).await;
        assert!(
            svc.extension_activation.problems().is_empty(),
            "{:?}",
            svc.extension_activation.problems()
        );
        assert!(!binding_available(&svc, "cons", "dep").await);
        assert!(!has_act(&svc, "cons").await, "the guarded command goes");
        assert!(readable(&svc, "v_cons_plain").await);
        let withheld = listed(&svc, "cons").await.public_contracts.withheld;
        assert!(
            withheld
                .iter()
                .any(|w| w.contribution.to_string().contains("own.act") && w.imports == ["dep"]),
            "{withheld:?}"
        );
    }
}

/// A contract version other than the one exported blocks, whatever the kind.
#[tokio::test]
async fn a_version_mismatch_of_every_kind_blocks_the_consumer() {
    for kind in ALL_KINDS {
        let (_dir, svc) = project(&[
            Ext::new("prod").declares(kind).exports_kind(kind, "stable"),
            Ext::new("cons").imports_kind("dep", "prod", kind, 2, false),
        ])
        .await;
        activate(&svc).await;
        assert!(
            problem(&svc, "cons").contains("`dep`"),
            "{}",
            problem(&svc, "cons")
        );
    }
}

/// Two consumers of one producer, and a consumer of both: all activate, the
/// producer first.
#[tokio::test]
async fn a_diamond_of_imports_activates_dependencies_first() {
    let (_dir, svc) = project(&[
        Ext::new("bottom")
            .exports("stable")
            .declares(Kind::Command)
            .exports_kind(Kind::Command, "stable"),
        Ext::new("left")
            .imports("bottom", 1, false)
            .declares(Kind::Command)
            .exports_kind(Kind::Command, "stable"),
        Ext::new("right")
            .imports("bottom", 1, false)
            .declares(Kind::Collector)
            .exports_kind(Kind::Collector, "stable"),
        Ext::new("top")
            .imports_kind("l", "left", Kind::Command, 1, false)
            .imports_kind("r", "right", Kind::Collector, 1, false),
    ])
    .await;
    activate(&svc).await;
    assert!(
        svc.extension_activation.problems().is_empty(),
        "{:?}",
        svc.extension_activation.problems()
    );
    let mut order = Vec::new();
    for name in ["bottom", "left", "right", "top"] {
        order.push(
            listed(&svc, name)
                .await
                .public_contracts
                .activation_index
                .expect("ordered"),
        );
    }
    let (b, l, r, t) = (order[0], order[1], order[2], order[3]);
    assert!(b < l && b < r && l < t && r < t, "{b} {l} {r} {t}");
    assert!(readable(&svc, "v_left_items").await);
    assert!(binding_available(&svc, "top", "l").await);
    assert!(binding_available(&svc, "top", "r").await);
}

/// A diamond whose shared base fails blocks both sides and the top, and
/// nothing else.
#[tokio::test]
async fn a_failed_base_of_a_diamond_blocks_everything_above_it() {
    let (_dir, svc) = project(&[
        Ext::new("bottom")
            .imports("gone", 1, false)
            .exports("stable"),
        Ext::new("left")
            .imports("bottom", 1, false)
            .declares(Kind::Command)
            .exports_kind(Kind::Command, "stable"),
        Ext::new("right")
            .imports("bottom", 1, false)
            .declares(Kind::Collector)
            .exports_kind(Kind::Collector, "stable"),
        Ext::new("top")
            .imports_kind("l", "left", Kind::Command, 1, false)
            .imports_kind("r", "right", Kind::Collector, 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    for name in ["bottom", "left", "right", "top"] {
        assert!(!problem(&svc, name).is_empty(), "{name}");
    }
    assert!(problem(&svc, "other").is_empty());
    assert!(readable(&svc, "v_other_items").await);
}

/// A cycle through three extensions blocks every one of them, with the same
/// path at each, and not an extension beside it.
#[tokio::test]
async fn a_three_way_cycle_blocks_all_three() {
    let (_dir, svc) = project(&[
        Ext::new("a").exports("stable").imports("b", 1, false),
        Ext::new("b").exports("stable").imports("c", 1, false),
        Ext::new("c").exports("stable").imports("a", 1, false),
        Ext::new("d").imports("a", 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    for (name, path) in [
        ("a", "a -> b -> c -> a"),
        ("b", "b -> c -> a -> b"),
        ("c", "c -> a -> b -> c"),
    ] {
        let why = problem(&svc, name);
        assert!(why.contains(path), "{name}: {why}");
        assert!(!readable(&svc, &format!("v_{name}_plain")).await);
    }
    assert!(
        !problem(&svc, "d").is_empty(),
        "a consumer of a cycle is blocked"
    );
    assert!(problem(&svc, "other").is_empty());
    assert!(readable(&svc, "v_other_items").await);
}

/// An extension importing its own export is an error for it alone.
#[tokio::test]
async fn a_self_import_blocks_only_itself() {
    let (_dir, svc) = project(&[
        Ext::new("me").exports("stable").imports("me", 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    assert!(!problem(&svc, "me").is_empty());
    assert!(!readable(&svc, "v_me_plain").await);
    assert!(problem(&svc, "other").is_empty());
    assert!(readable(&svc, "v_other_items").await);
}

/// An optional edge that is available counts toward a cycle; one nothing
/// serves doesn't make one.
#[tokio::test]
async fn an_available_optional_edge_still_closes_a_cycle() {
    let (_dir, svc) = project(&[
        Ext::new("a").exports("stable").imports("b", 1, false),
        Ext::new("b").exports("stable").imports("a", 1, true),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    assert!(
        problem(&svc, "a").contains("a -> b -> a"),
        "{}",
        problem(&svc, "a")
    );
    assert!(
        problem(&svc, "b").contains("b -> a -> b"),
        "{}",
        problem(&svc, "b")
    );
    assert!(readable(&svc, "v_other_items").await);
}

/// Repairing a producer that was held back lets its consumer, held
/// because of it, activate at the next activation, with no edit of its own.
#[tokio::test]
async fn a_repaired_producer_brings_back_its_consumer() {
    let (dir, svc) = project(&[
        Ext::new("prod").exports("stable").imports("base", 1, false),
        Ext::new("cons").imports("prod", 1, false),
        Ext::new("other"),
    ])
    .await;
    activate(&svc).await;
    assert!(!problem(&svc, "prod").is_empty());
    assert!(!problem(&svc, "cons").is_empty());
    assert!(!readable(&svc, "v_cons_plain").await);

    // The base arrives: the producer, and with it the consumer, recover.
    put(dir.path(), &Ext::new("base").exports("stable"));
    activate(&svc).await;
    assert!(
        svc.extension_activation.problems().is_empty(),
        "{:?}",
        svc.extension_activation.problems()
    );
    for view in ["v_base_items", "v_prod_items", "v_cons_items"] {
        assert!(readable(&svc, view).await, "{view}");
    }
    page(&svc, "cons/seen")
        .await
        .expect("the page reads the import");

    // And withdrawn again when the base goes.
    std::fs::remove_dir_all(dir.path().join(".oxplow/extensions/base")).unwrap();
    crate::extension_deploy::remove(&svc, "base").await.unwrap();
    activate(&svc).await;
    assert!(!readable(&svc, "v_cons_plain").await);
    assert!(readable(&svc, "v_other_items").await);
}

/// A producer's version fixed to the consumer's lets the consumer in.
#[tokio::test]
async fn a_producer_at_the_right_version_after_a_mismatch_recovers_its_consumer() {
    let (dir, svc) = project(&[
        Ext::new("prod").at_version(2).exports("stable"),
        Ext::new("cons").imports("prod", 1, false),
    ])
    .await;
    activate(&svc).await;
    assert!(!problem(&svc, "cons").is_empty());
    put(dir.path(), &Ext::new("prod").exports("stable"));
    activate(&svc).await;
    assert!(svc.extension_activation.problems().is_empty());
    assert!(readable(&svc, "v_cons_items").await);
}

/// An import with no version binds whatever its producer exports, and
/// keeps binding when the producer moves on.
#[tokio::test]
async fn an_import_without_a_version_binds_whatever_is_exported() {
    let (dir, svc) = project(&[
        Ext::new("prod").at_version(2).exports("stable"),
        Ext::new("cons").imports_any("prod"),
    ])
    .await;
    activate(&svc).await;
    assert!(svc.extension_activation.problems().is_empty());
    assert!(readable(&svc, "v_cons_items").await);
    let bound = |e: crate::extensions::Extension| {
        e.public_contracts
            .bindings
            .iter()
            .find(|b| b.id == "dep")
            .map(|b| (b.version, b.available))
    };
    assert_eq!(bound(listed(&svc, "cons").await), Some((Some(2), true)));
    put(
        dir.path(),
        &Ext::new("prod").at_version(3).exports("stable"),
    );
    activate(&svc).await;
    assert!(svc.extension_activation.problems().is_empty());
    assert_eq!(bound(listed(&svc, "cons").await), Some((Some(3), true)));
}

/// A page withheld by an unavailable optional import takes the slot mount
/// that shows it; an available import keeps both.
#[tokio::test]
async fn a_guarded_page_prunes_its_slot_mounts() {
    let (dir, svc) = project(&[Ext::new("cons").imports("prod", 1, true).mounted()]).await;
    activate(&svc).await;
    let cons = listed(&svc, "cons").await;
    assert!(cons.ui.page_slots.is_empty(), "{:?}", cons.ui.page_slots);
    assert!(cons.page_files.iter().all(|p| p.slug != "seen"));
    assert!(
        cons.public_contracts
            .withheld
            .iter()
            .any(|w| w.contribution.to_string().contains("seen")),
        "{:?}",
        cons.public_contracts.withheld
    );

    put(dir.path(), &Ext::new("prod").exports("stable"));
    activate(&svc).await;
    let cons = listed(&svc, "cons").await;
    assert_eq!(
        cons.ui.page_slots.len(),
        1,
        "{:?} {:?}",
        cons.errors,
        cons.ui.page_slots
    );
    assert!(cons.page_files.iter().any(|p| p.slug == "seen"));
    assert!(cons.public_contracts.withheld.is_empty());
}
