//! An extension's life, one scenario per story: every transition goes
//! through the command bus as a person or agent would run it
//! (`oxplow.extension.deploy`, `disable`, `enable`, `rollback`), and every
//! read goes through what users see (`v_*` datasets over the SQL gateway,
//! the extension's own commands, page runs). The single transitions are
//! pinned by `extension_deploy`'s tests; here a transition meets the rest
//! of the extension. See `.context/extensions-deploy.md` "Lifecycle
//! scenarios".

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

use crate::extension::extension_job_stream_tests::{
    ask, completions, fixture_idle, reply, rows as job_rows, until, MANIFEST,
};
use crate::extension::extension_private_state_tests::{declare, write, write_state, CONFIGURATION};
use crate::test_fixtures::lifecycle::{
    booted, log_claim, page, problem, readable, CONFIGURATION_V2,
};
use crate::test_fixtures::lifecycle::{write_extension, EXT};
use crate::test_fixtures::EffortFixture;

const NAME: &str = "alpha";

/// A version of the `alpha` extension. Every version has the private-state
/// and configuration commands of `extension_private_state_tests::declare`,
/// `alpha.state.peek` answering `{ v, state }` (so the answer follows the
/// version and shows what it reads of its private state), the dataset
/// `items`, the page `one`, the collector `seen` and the effect `note`.
#[derive(Clone)]
struct Alpha {
    /// What the version says in its command, collector, effect and page.
    v: u32,
    /// The `configuration:` block, as `declare` takes it.
    configuration: &'static str,
    /// Its `items` dataset has a `size` column (and its contract is at 2).
    sized: bool,
    /// Its `items` dataset's SQL does not compile.
    broken_dataset: bool,
    /// It exports `items` (`stable`), at the dataset's own version.
    exported: bool,
    /// Its collector settles for 30 s before it runs for an event.
    paced: bool,
}

impl Alpha {
    fn new(v: u32) -> Self {
        Alpha {
            v,
            configuration: "",
            sized: false,
            broken_dataset: false,
            exported: false,
            paced: false,
        }
    }

    fn configured(mut self, configuration: &'static str) -> Self {
        self.configuration = configuration;
        self
    }

    fn sized(mut self) -> Self {
        self.sized = true;
        self
    }

    fn exported(mut self) -> Self {
        self.exported = true;
        self
    }

    fn paced(mut self) -> Self {
        self.paced = true;
        self
    }

    fn broken(mut self) -> Self {
        self.broken_dataset = true;
        self
    }

    /// Write it as `alpha`'s source in `root`'s `.oxplow/extensions`.
    fn write(&self, root: &Path) {
        declare(root, NAME, self.configuration);
        let dir = format!(".oxplow/extensions/{NAME}");
        let manifest = root.join(&dir).join("extension.yaml");
        let (version, fields) = if self.sized {
            (
                2,
                "      - { name: uri, type: TEXT, doc: The uri. }\n      - { name: size, type: TEXT, doc: The size. }\n",
            )
        } else {
            (1, "      - { name: uri, type: TEXT, doc: The uri. }\n")
        };
        let exports = if self.exported {
            format!(
                "exports:\n  - {{ contribution: {}, version: {version}, visibility: stable }}\n",
                dataset_ref(NAME)
            )
        } else {
            String::new()
        };
        let settle = if self.paced { "\n    settle: 30s" } else { "" };
        let mut text = std::fs::read_to_string(&manifest).unwrap();
        text += &exports;
        text += &format!(
            "datasets:
  - name: items
    version: {version}
    description: Items.
    fields:
{fields}collectors:
  - id: seen
    runtime: starlark
    entry: collectors/seen.star
    on: [effort.claim_verified]{settle}
    input: \"SELECT :event_seq AS ev\"
    sync: upsert
    entities:
      - {{ name: seen, key: ev, columns: {{ ev: int, who: text }} }}
effects:
  - id: note
    summary: Note a verified claim.
    on: [effort.claim_verified]
    entry: effects/note.star
event_types:
  types:
    - {{ type: alpha.noted, v: 1, schema: noted.json, summary: A claim was noted. }}
"
        );
        std::fs::write(&manifest, text).unwrap();
        let v = self.v;
        write(root, &format!("{dir}/noted.json"), "{\"type\": \"object\"}");
        write(
            root,
            &format!("{dir}/handlers/peek.star"),
            &format!(
                "def transform(x):\n    return {{\"commands\": [], \"result\": {{\"v\": {v}, \"state\": scope(\"state.read\", {{\"scope\": \"project\", \"area\": \"state\"}})}}}}\n"
            ),
        );
        write(
            root,
            &format!("{dir}/collectors/seen.star"),
            &format!("def transform(input):\n    return {{\"entities\": {{\"seen\": [{{\"ev\": r[\"ev\"], \"who\": \"v{v}\"}} for r in input[\"rows\"]]}}}}\n"),
        );
        write(
            root,
            &format!("{dir}/effects/note.star"),
            &format!("def transform(x):\n    return {{\"commands\": [], \"events\": [{{\"type\": \"alpha.noted\", \"payload\": {{\"from\": \"v{v}\"}}}}]}}\n"),
        );
        let sql = match (self.broken_dataset, self.sized) {
            (true, _) => "SELECT nope FROM no_such_table",
            (false, true) => "SELECT uri, uri AS size FROM ref('work_item')",
            (false, false) => "SELECT uri FROM ref('work_item')",
        };
        write(root, &format!("{dir}/datasets/items.sql"), sql);
        write(
            root,
            &format!("{dir}/pages/one.yaml"),
            &format!(
                "title: One v{v}\nloaders:\n  n: {{ sql: \"SELECT uri FROM v_alpha_items\" }}\nview: {{ viz: table, from: n }}\n"
            ),
        );
    }
}

/// Run `command` as a person who confirms what it asks.
async fn bus(
    svc: &Arc<crate::Services>,
    command: &str,
    input: Value,
) -> Result<Value, CommandError> {
    svc.commands
        .run(&Actor::Human, command, input, true)
        .await
        .map(|outcome| outcome.result)
}

async fn deploy(svc: &Arc<crate::Services>, name: &str) -> Result<Value, CommandError> {
    bus(svc, "oxplow.extension.deploy", json!({ "name": name })).await
}

async fn disable(svc: &Arc<crate::Services>, name: &str) {
    bus(svc, "oxplow.extension.disable", json!({ "name": name }))
        .await
        .unwrap();
}

async fn enable(svc: &Arc<crate::Services>, name: &str) {
    bus(svc, "oxplow.extension.enable", json!({ "name": name }))
        .await
        .unwrap();
}

async fn rollback(svc: &Arc<crate::Services>, name: &str) -> Result<Value, CommandError> {
    bus(svc, "oxplow.extension.rollback", json!({ "name": name })).await
}

/// The rows of `sql` as a user's query reads them.
async fn rows(fx: &EffortFixture, sql: &str) -> Vec<Vec<Value>> {
    let out = fx.svc.sql.query_sql(sql, vec![], None).await.unwrap();
    serde_json::from_value(serde_json::to_value(out.rows).unwrap()).unwrap()
}

/// The columns of `view`, in order.
async fn columns(fx: &EffortFixture, view: &str) -> Vec<String> {
    fx.svc
        .sql
        .query_sql(&format!("SELECT * FROM {view}"), vec![], None)
        .await
        .unwrap()
        .columns
}

/// `alpha.state.peek`'s answer, or why it can't be asked.
async fn peek(fx: &EffortFixture) -> Result<Value, CommandError> {
    bus(&fx.svc, "alpha.state.peek", json!({}))
        .await
        .map(|out| out["result"].clone())
}

/// The title of page `alpha/one` as the catalog runs it, if it does.
fn page_title(fx: &EffortFixture) -> Option<String> {
    fx.svc
        .extension_catalog
        .find_page(&fx.svc.layout.project_dir, "alpha/one")
        .ok()
        .map(|l| l.title)
}

/// A claim is verified: the collector and the effect react to it, as the
/// event pump runs them.
async fn verified(fx: &EffortFixture) {
    log_claim(&fx.svc, fx.effort, "reviewer").await;
    assert!(
        fx.svc.event_pump.settle_all(Duration::from_secs(10)).await,
        "the consumers handled it"
    );
}

/// What the collector wrote: `(who)` per event, oldest first.
async fn seen(fx: &EffortFixture) -> Vec<String> {
    rows(fx, "SELECT who FROM v_alpha_seen ORDER BY ev")
        .await
        .into_iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect()
}

/// What the effect said, in order.
async fn noted(fx: &EffortFixture) -> Vec<String> {
    rows(
        fx,
        "SELECT json_extract(payload, '$.from') FROM v_event WHERE type = 'alpha.noted' ORDER BY seq",
    )
    .await
    .into_iter()
    .map(|r| r[0].as_str().unwrap().to_string())
    .collect()
}

/// A redeploy that changes a command's answer, a dataset's columns, a
/// collector and an effect: all four follow, and what the old collector
/// wrote stays.
#[tokio::test(flavor = "multi_thread")]
async fn a_redeploy_changes_the_command_the_dataset_the_collector_and_the_effect() {
    let fx = booted().await;
    let root = fx.svc.layout.project_dir.clone();
    Alpha::new(1).write(&root);
    let first = deploy(&fx.svc, NAME).await.unwrap();
    assert_eq!(first["already"], json!(false), "{first}");

    assert_eq!(peek(&fx).await.unwrap()["v"], json!(1));
    assert_eq!(columns(&fx, "v_alpha_items").await, ["uri"]);
    verified(&fx).await;
    assert_eq!(seen(&fx).await, ["v1"]);
    assert_eq!(noted(&fx).await, ["v1"]);

    Alpha::new(2).sized().write(&root);
    let second = deploy(&fx.svc, NAME).await.unwrap();
    assert_eq!(second["already"], json!(false), "{second}");
    assert_ne!(first["hash"], second["hash"]);

    assert_eq!(peek(&fx).await.unwrap()["v"], json!(2));
    assert_eq!(columns(&fx, "v_alpha_items").await, ["uri", "size"]);
    assert_eq!(page_title(&fx).as_deref(), Some("One v2"));
    verified(&fx).await;
    assert_eq!(
        seen(&fx).await,
        ["v1", "v2"],
        "the rows the first collector wrote stay beside the second's"
    );
    assert_eq!(noted(&fx).await, ["v1", "v2"]);
}

/// `name` reads `alpha`'s `items` as the import `dep`, in a dataset of its
/// own and in its page `seen`. `version` pins the contract it was written
/// against; none binds whatever `alpha` exports.
fn write_consumer(root: &Path, name: &str, version: Option<u32>) {
    let pinned = version
        .map(|v| format!(", version: {v}"))
        .unwrap_or_default();
    let dir = format!(".oxplow/extensions/{name}");
    write(
        root,
        &format!("{dir}/extension.yaml"),
        &format!(
            "manifest: 2\nname: {name}\nintent:\n  purpose: p\ndatasets:\n  - name: items\n    version: 1\n    description: Items.\n    fields:\n      - {{ name: uri, type: TEXT, doc: The uri. }}\nimports:\n  - {{ id: dep, contribution: {}{pinned}, optional: false }}\n",
            dataset_ref(NAME)
        ),
    );
    write(
        root,
        &format!("{dir}/datasets/items.sql"),
        "SELECT uri FROM import_ref('dep')",
    );
    write(
        root,
        &format!("{dir}/pages/seen.yaml"),
        "title: Seen\nloaders:\n  seen: { sql: \"SELECT uri FROM import_ref('dep')\" }\nview: { viz: table, from: seen }\n",
    );
}

fn dataset_ref(owner: &str) -> String {
    oxplow_domain::uri::build::keyed("oxplow/dataset", &format!("{owner}/items"))
}

/// Everything `alpha` gives, and `gamma` over it, is there.
async fn all_there(fx: &EffortFixture) {
    assert!(readable(&fx.svc, "v_alpha_items").await, "dataset");
    assert!(readable(&fx.svc, "v_gamma_items").await, "consumer");
    assert!(page(&fx.svc, "alpha/one").await.is_ok(), "page");
    assert!(page(&fx.svc, "gamma/seen").await.is_ok(), "consumer page");
    assert_eq!(peek(fx).await.unwrap()["v"], json!(1), "command");
}

/// Switching an extension off takes away its dataset, page, command and
/// whoever imports its dataset; switching it on brings them back, and a
/// collector run that was due waits for it.
#[tokio::test(flavor = "multi_thread")]
async fn disabling_then_enabling_an_extension_takes_away_and_brings_back_what_it_gave() {
    let fx = booted().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    Alpha::new(1).exported().paced().write(&root);
    write_consumer(&root, "gamma", Some(1));
    deploy(&fx.svc, NAME).await.unwrap();
    deploy(&fx.svc, "gamma").await.unwrap();

    all_there(&fx).await;

    // A claim is verified; the collector settles for 30 s before it runs.
    verified(&fx).await;
    assert_eq!(
        rows(&fx, "SELECT owner, id FROM v_pending_run").await,
        [[json!("alpha"), json!("seen")]]
    );
    assert!(seen(&fx).await.is_empty(), "not run yet");

    disable(&fx.svc, NAME).await;
    assert!(!readable(svc, "v_alpha_items").await, "dataset is gone");
    assert!(page(svc, "alpha/one").await.is_err(), "page is gone");
    assert!(peek(&fx).await.is_err(), "command is gone");
    assert!(!readable(svc, "v_gamma_items").await, "consumer withdrawn");
    assert!(page(svc, "gamma/seen").await.is_err(), "consumer page");

    enable(&fx.svc, NAME).await;
    all_there(&fx).await;
    assert!(
        problem(svc, "gamma").is_empty(),
        "the consumer is whole again: {}",
        problem(svc, "gamma")
    );

    let later =
        oxplow_domain::Timestamp::from_unix_ms(oxplow_domain::Timestamp::now().unix_ms() + 31_000);
    assert_eq!(
        crate::pacing::run_due(svc, later).await.unwrap(),
        1,
        "the collector that was due runs again"
    );
    assert_eq!(seen(&fx).await, ["v1"]);
}

/// Rolling back brings the earlier version's dataset, command and page
/// back; what the later version wrote to private state stays readable, and
/// a configuration layer it wrote at a newer schema version fails closed
/// until the earlier version's own command writes it again.
#[tokio::test(flavor = "multi_thread")]
async fn a_rollback_restores_the_earlier_version_over_what_the_later_one_stored() {
    let fx = booted().await;
    let root = fx.svc.layout.project_dir.clone();
    Alpha::new(1).configured(CONFIGURATION).write(&root);
    let first = deploy(&fx.svc, NAME).await.unwrap();
    Alpha::new(2)
        .sized()
        .configured(CONFIGURATION_V2)
        .write(&root);
    deploy(&fx.svc, NAME).await.unwrap();

    // The later version writes its private state and a layer at its schema.
    bus(
        &fx.svc,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"by": "v2"})),
    )
    .await
    .unwrap();
    bus(
        &fx.svc,
        "alpha.config.write",
        json!({"schema_version": 2, "project": {"expected": null, "values": {"limit": 20}}}),
    )
    .await
    .unwrap();
    assert_eq!(columns(&fx, "v_alpha_items").await, ["uri", "size"]);

    let back = rollback(&fx.svc, NAME).await.unwrap();
    assert_eq!(back["hash"], first["hash"], "{back}");

    assert_eq!(columns(&fx, "v_alpha_items").await, ["uri"]);
    assert_eq!(page_title(&fx).as_deref(), Some("One v1"));
    verified(&fx).await;
    assert_eq!(seen(&fx).await, ["v1"], "the earlier collector runs");
    let answer = peek(&fx).await.unwrap();
    assert_eq!(answer["v"], json!(1), "the earlier command answers");
    assert_eq!(
        answer["state"]["values"],
        json!({"by": "v2"}),
        "private state outlives the version that wrote it"
    );

    // The layer is at schema 2; version 1 declares 1: it fails closed.
    let refused = bus(&fx.svc, "alpha.config.read", json!({}))
        .await
        .expect_err("a layer at another schema version is not read");
    let CommandError::Invalid { field, .. } = refused else {
        panic!("{refused:?}")
    };
    assert_eq!(field.as_deref(), Some("/schema_version"));

    // The layer stays at schema 2: the store takes no downgrade, so version
    // 1's own command cannot write it either (see the ignored test below).
    // Deploying the later version again reads it as it was.
    deploy(&fx.svc, NAME).await.unwrap();
    let config = bus(&fx.svc, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"]["limit"], json!(20));
}

/// What `data-model.md` says of a layer at another schema version: it
/// fails closed until the extension's own command rewrites it — after a
/// rollback too, where the layer is newer than the running version.
#[tokio::test(flavor = "multi_thread")]
async fn a_rolled_back_version_rewrites_the_configuration_layer_the_later_one_wrote() {
    let fx = booted().await;
    let root = fx.svc.layout.project_dir.clone();
    Alpha::new(1).configured(CONFIGURATION).write(&root);
    let first = deploy(&fx.svc, NAME).await.unwrap();
    Alpha::new(2)
        .sized()
        .configured(CONFIGURATION_V2)
        .write(&root);
    deploy(&fx.svc, NAME).await.unwrap();
    bus(
        &fx.svc,
        "alpha.config.write",
        json!({"schema_version": 2, "project": {"expected": null, "values": {"limit": 20}}}),
    )
    .await
    .unwrap();
    let back = rollback(&fx.svc, NAME).await.unwrap();
    assert_eq!(back["hash"], first["hash"], "{back}");

    bus(
        &fx.svc,
        "alpha.config.write",
        json!({"schema_version": 1,
               "project": {"expected": {"schema_version": 2, "revision": 1},
                           "values": {"limit": 5}}}),
    )
    .await
    .expect("version 1's command rewrites the layer");
    let config = bus(&fx.svc, "alpha.config.read", json!({})).await.unwrap();
    assert_eq!(config["values"]["limit"], json!(5));
}

/// A version whose dataset does not compile loads but does not activate:
/// the deploy fails and says why, and the version before it goes on
/// working, private state included.
#[tokio::test(flavor = "multi_thread")]
async fn a_deploy_that_loads_but_does_not_activate_leaves_the_running_version_working() {
    let fx = booted().await;
    let root = fx.svc.layout.project_dir.clone();
    Alpha::new(1).write(&root);
    let first = deploy(&fx.svc, NAME).await.unwrap();
    bus(
        &fx.svc,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"kept": true})),
    )
    .await
    .unwrap();

    Alpha::new(2).broken().write(&root);
    let err = deploy(&fx.svc, NAME)
        .await
        .expect_err("a dataset that does not compile does not go live");
    let why = err.to_string();
    assert!(
        why.contains("alpha") && why.contains("items"),
        "the error names the extension and its dataset: {why}"
    );

    // The version that was running still is.
    assert_eq!(peek(&fx).await.unwrap()["v"], json!(1));
    assert_eq!(columns(&fx, "v_alpha_items").await, ["uri"]);
    assert_eq!(page_title(&fx).as_deref(), Some("One v1"));
    verified(&fx).await;
    assert_eq!(seen(&fx).await, ["v1"], "its collector still runs");
    assert_eq!(noted(&fx).await, ["v1"], "and its effect");
    assert_eq!(
        peek(&fx).await.unwrap()["state"]["values"],
        json!({"kept": true})
    );
    assert_eq!(
        rows(
            &fx,
            "SELECT hash FROM v_extension_deployment WHERE name = 'alpha' AND active = 1"
        )
        .await,
        [[first["hash"].clone()]],
        "the failed deployment is not the active one"
    );
}

/// What a version wrote to the extension's private state is what the next
/// version reads, and writes on top of.
#[tokio::test(flavor = "multi_thread")]
async fn private_state_written_by_one_version_is_read_by_the_next() {
    let fx = booted().await;
    let root = fx.svc.layout.project_dir.clone();
    Alpha::new(1).write(&root);
    deploy(&fx.svc, NAME).await.unwrap();
    let stamp = bus(
        &fx.svc,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"cursor": {"page": 4}})),
    )
    .await
    .unwrap();
    assert_eq!(stamp, json!({"schema_version": 1, "revision": 1}));

    Alpha::new(2).sized().write(&root);
    deploy(&fx.svc, NAME).await.unwrap();
    let read = bus(
        &fx.svc,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(read["values"], json!({"cursor": {"page": 4}}));
    let answer = peek(&fx).await.unwrap();
    assert_eq!(answer["v"], json!(2));
    assert_eq!(answer["state"], read, "a script of v2 reads it too");
    let next = bus(
        &fx.svc,
        "alpha.state.write",
        write_state(
            "project",
            json!({"schema_version": 1, "revision": 1}),
            json!({"cursor": {"page": 5}}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(next["revision"], json!(2));
}

/// A producer's exported dataset changes while two consumers import it. An
/// added column (a new contract version) leaves the consumer that binds
/// whatever is exported working; a consumer written against the first
/// version is held with a problem that names its import.
#[tokio::test(flavor = "multi_thread")]
async fn a_changed_export_keeps_the_open_consumer_and_holds_the_pinned_one() {
    let fx = booted().await;
    let svc = &fx.svc;
    let root = svc.layout.project_dir.clone();
    Alpha::new(1).exported().write(&root);
    write_consumer(&root, "pinned", Some(1));
    write_consumer(&root, "open", None);
    for name in [NAME, "pinned", "open"] {
        deploy(&fx.svc, name).await.unwrap();
    }
    for view in ["v_pinned_items", "v_open_items"] {
        assert!(readable(svc, view).await, "{view}");
    }

    // The compatible change: a column added, so its contract is at 2.
    Alpha::new(2).sized().exported().write(&root);
    deploy(&fx.svc, NAME).await.unwrap();
    assert_eq!(columns(&fx, "v_alpha_items").await, ["uri", "size"]);
    assert!(readable(svc, "v_open_items").await, "the open consumer");
    assert!(page(svc, "open/seen").await.is_ok());
    assert!(problem(svc, "open").is_empty(), "{}", problem(svc, "open"));

    // The pinned one asked for version 1: the activation says which import
    // no longer binds. It is held at its last working version, so what it
    // served before keeps serving (`extensions-activation.md` "One broken
    // extension doesn't hold back the rest").
    let why = problem(svc, "pinned");
    assert!(why.contains("`dep`"), "{why}");
    assert!(why.contains("alpha/items"), "{why}");
    assert!(readable(svc, "v_pinned_items").await, "{why}");

    // Written against version 2 and deployed, it is back.
    write_consumer(&root, "pinned", Some(2));
    deploy(&fx.svc, "pinned").await.unwrap();
    assert!(readable(svc, "v_pinned_items").await);
    assert!(problem(svc, "pinned").is_empty());
}

/// The titles `v_work_item` shows, in order: the active work list's.
async fn titles(fx: &EffortFixture) -> Vec<String> {
    rows(fx, "SELECT title FROM v_work_item ORDER BY title")
        .await
        .into_iter()
        .map(|r| r[0].as_str().unwrap().to_string())
        .collect()
}

/// The provider `v_capability_provider` says the work list is.
async fn active_work_list(fx: &EffortFixture) -> String {
    rows(
        fx,
        "SELECT provider FROM v_capability_provider WHERE capability = 'work_items' AND active = 1",
    )
    .await[0][0]
        .as_str()
        .unwrap()
        .to_string()
}

/// Choose `provider` as the work list, as a person's setting does.
async fn choose_work_list(fx: &EffortFixture, provider: &str) {
    bus(
        &fx.svc,
        "oxplow.capability.choose",
        json!({ "capability": "work_items", "provider": provider }),
    )
    .await
    .unwrap();
    // The setting asks for a generation; the activation owner's pass is
    // what the test runs.
    assert!(
        fx.svc.event_pump.settle_all(Duration::from_secs(10)).await,
        "the setting reached the registries"
    );
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
}

async fn file_item(fx: &EffortFixture, title: &str) {
    bus(
        &fx.svc,
        crate::commands::work_item::CREATE,
        json!({ "title": title }),
    )
    .await
    .unwrap();
}

/// Switching the work list from oxplow's own to a provider's and back
/// shows each list's items on its side, and the first list's items are
/// there again when it is chosen again.
#[tokio::test(flavor = "multi_thread")]
async fn switching_the_work_list_shows_each_providers_items_and_brings_the_other_back() {
    let fx = booted().await;
    write_extension(&fx.svc.layout.project_dir, "");
    deploy(&fx.svc, EXT).await.unwrap();
    bus(
        &fx.svc,
        "oxplow.provider.set_instance",
        json!({ "instance": "tracker/fake", "enabled": true, "config": { "team": "core" } }),
    )
    .await
    .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(active_work_list(&fx).await, "oxplow");
    file_item(&fx, "ours").await;
    assert_eq!(titles(&fx).await, ["ours"]);

    choose_work_list(&fx, "fake").await;
    assert_eq!(active_work_list(&fx).await, "fake");
    assert!(titles(&fx).await.is_empty(), "oxplow's items are not shown");
    file_item(&fx, "theirs").await;
    assert_eq!(titles(&fx).await, ["theirs"]);

    choose_work_list(&fx, "oxplow").await;
    assert_eq!(active_work_list(&fx).await, "oxplow");
    assert_eq!(
        titles(&fx).await,
        ["ours"],
        "the fake's items are not shown"
    );

    choose_work_list(&fx, "fake").await;
    assert_eq!(titles(&fx).await, ["theirs"], "its items come back");
}

/// A job queued before its extension is switched off is not cancelled
/// (nothing of it ran): it waits while the extension is off, and runs
/// when it is on again.
#[tokio::test(flavor = "multi_thread")]
async fn a_queued_job_waits_while_its_extension_is_off_and_runs_when_it_is_on_again() {
    let fx = fixture_idle(MANIFEST, vec![reply("back again")]).await;
    let id = ask(&fx, "off-and-on").await;

    disable(&fx.svc, "triage").await;
    fx.svc
        .jobs
        .spawn(fx.svc.extension_activation.clone(), fx.svc.commands.clone());
    // The scheduler polls each second: give it several passes.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let state = job_rows(
        &fx,
        "SELECT state FROM v_job_run WHERE id = ?1",
        vec![json!(id)],
    )
    .await;
    assert_eq!(state, [[json!("queued")]], "it waits, not cancelled");

    enable(&fx.svc, "triage").await;
    let done = until(&fx, &id, |s| s["state"] == json!("succeeded")).await;
    assert_eq!(done["text"], json!("back again"), "{done}");
    assert_eq!(completions(&fx, &id).await, ["succeeded"]);
}
