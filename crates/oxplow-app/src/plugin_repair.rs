//! The repair work item (P7.C2): when a plugin contribution is disabled,
//! a work item asks for it to be fixed.
//!
//! The `plugin.repair` pump consumer handles each `plugin.disabled@1`. The
//! first disable files an item on the active work-items provider, as the
//! system: title `Repair <plugin> <contribution>: <reason>`, body the repair
//! prompt ([`render`]). A later disable while that item is open comments
//! the new failure on it; once the item is done or canceled the next
//! disable files a new one. The item is the contribution's
//! `plugin_health.repair_item` (`v_plugin_health` shows it while open).
//! oxplow never sends the prompt to an agent: a person does, from the item
//! or Settings → Extensions' Repair with the Agent.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use oxplow_db::plugin_health_store::{self as store, PluginKey};
use oxplow_domain::{Actor, DomainError, StoredEvent};

use crate::event_pump::AsyncEventConsumer;
use crate::Services;

/// The consumer's name: its checkpoint and dead letters.
pub const NAME: &str = "plugin.repair";
/// The errors a prompt lists.
const ERRORS_SHOWN: usize = 5;
/// The longest reason a title carries.
const TITLE_REASON_MAX: usize = 80;

/// Everything a repair prompt says.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RepairContext {
    pub plugin: String,
    pub contribution: String,
    /// `provider` or `collector`.
    pub kind: String,
    /// What disabled it.
    pub reason: String,
    /// The extension's `intent.purpose`.
    pub purpose: Option<String>,
    /// The thread or effort that created it.
    pub origin: Option<String>,
    /// Where it's declared (`oxplow/extensions/<name>/extension.yaml`,
    /// `.oxplow/project.yaml`).
    pub manifest: Option<String>,
    /// Its declaration, as YAML.
    pub declaration: Option<String>,
    /// Its recent failures, newest first.
    pub errors: Vec<String>,
    /// What loading its extension reports.
    pub check: Vec<String>,
    /// The intent examples it should still satisfy (their names).
    pub examples: Vec<String>,
    /// The manifest's `engine:` and the running oxplow's version.
    pub engine: Option<String>,
    pub current_engine: String,
}

/// The repair item's title.
pub fn title(c: &RepairContext) -> String {
    let first = c.reason.lines().next().unwrap_or_default();
    let reason: String = if first.chars().count() > TITLE_REASON_MAX {
        let cut: String = first.chars().take(TITLE_REASON_MAX).collect();
        format!("{}…", cut.trim_end())
    } else {
        first.to_string()
    };
    format!("Repair {} {}: {reason}", c.plugin, c.contribution)
}

/// The repair prompt: what failed, what it's for, how it's declared, what
/// went wrong and what to do — the work item's body.
pub fn render(c: &RepairContext) -> String {
    let mut out = format!(
        "The {} `{}` of the extension `{}` was disabled on this machine after it kept failing:\n\n> {}\n",
        c.kind,
        c.contribution,
        c.plugin,
        c.reason.replace('\n', "\n> ")
    );
    if let Some(purpose) = &c.purpose {
        out.push_str(&format!("\n## What it's for\n\n{purpose}\n"));
        if let Some(origin) = &c.origin {
            out.push_str(&format!("\nIt was made in [[{origin}]].\n"));
        }
    }
    if let Some(decl) = &c.declaration {
        out.push_str(&format!(
            "\n## Its declaration\n\nIn `{}`:\n\n```yaml\n{}```\n",
            c.manifest.as_deref().unwrap_or("its manifest"),
            decl
        ));
    }
    if !c.errors.is_empty() {
        out.push_str("\n## Recent failures (newest first)\n\n");
        for e in c.errors.iter().take(ERRORS_SHOWN) {
            out.push_str(&format!("- {}\n", e.replace('\n', " ")));
        }
    }
    out.push_str("\n## What `oxplow plugin check` reports\n\n");
    if c.check.is_empty() {
        out.push_str("Nothing: it loads cleanly.\n");
    } else {
        for line in &c.check {
            out.push_str(&format!("- {line}\n"));
        }
    }
    if !c.examples.is_empty() {
        out.push_str(&format!(
            "\nIts intent examples, which it must still satisfy: {}.\n",
            c.examples
                .iter()
                .map(|e| format!("`{e}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(engine) = &c.engine {
        out.push_str(&format!(
            "\nIt targets oxplow `{engine}`; this is oxplow {}.\n",
            c.current_engine
        ));
    }
    out.push_str(&format!(
        "\n## What to do\n\n\
         1. Find why it fails: read the failures and its source, and reproduce one \
         (`oxplow plugin test {plugin}`).\n\
         2. Fix it, keeping its purpose and examples.\n\
         3. Check it: `oxplow plugin check {plugin}` and `oxplow plugin test {plugin}` must be clean.\n\
         4. Say what you changed on this item. A person enables it again \
         (`plugin.enable`, Settings → Extensions); you can't.\n",
        plugin = c.plugin
    ));
    out
}

/// What's known about `key`'s contribution, for its prompt.
pub async fn gather(svc: &Services, key: &PluginKey, reason: &str) -> RepairContext {
    let mut c = RepairContext {
        plugin: key.plugin.clone(),
        contribution: key.contribution.clone(),
        kind: key.kind.to_string(),
        reason: reason.to_string(),
        current_engine: crate::extensions::manifest_v2::current_engine().to_string(),
        ..RepairContext::default()
    };
    let list = match key.kind {
        "provider" => "providers",
        "effect" => "effects",
        _ => "collectors",
    };
    if key.plugin == oxplow_config::collectors::PROJECT {
        c.manifest = Some(".oxplow/project.yaml".into());
        let config = svc.config.read().map(|c| c.clone()).ok();
        c.declaration = config
            .and_then(|cfg| cfg.collectors_yaml)
            .and_then(|raw| serde_json::to_string(&raw).ok())
            .and_then(|text| serde_yaml::from_str::<serde_yaml::Value>(&text).ok())
            .and_then(|list| item_yaml(&list, &key.contribution));
    } else if let Some(ext) = svc
        .extension_catalog
        .get(&svc.layout.project_dir)
        .iter()
        .find(|e| e.name == key.plugin)
    {
        let manifest = format!("{}/extension.yaml", ext.path);
        c.purpose = ext.intent.as_ref().map(|i| i.purpose.clone());
        c.origin = ext.intent.as_ref().and_then(|i| i.origin.clone());
        c.examples = ext
            .intent
            .as_ref()
            .map(|i| i.examples.iter().map(|e| e.name.clone()).collect())
            .unwrap_or_default();
        c.check = ext.errors.iter().chain(&ext.warnings).cloned().collect();
        if let Some(doc) = crate::extensions::read_extension_file(
            &svc.layout.project_dir,
            &ext.name,
            "extension.yaml",
        )
        .and_then(|text| serde_yaml::from_str::<serde_yaml::Value>(&text).ok())
        {
            c.engine = doc["engine"].as_str().map(str::to_string);
            c.declaration = item_yaml(&doc[list], &key.contribution);
        }
        c.manifest = Some(manifest);
    }
    c.errors = recent_errors(svc, key, reason).await;
    c
}

/// The item of a declaration list whose `id` is `id`, as YAML.
fn item_yaml(list: &serde_yaml::Value, id: &str) -> Option<String> {
    list.as_sequence()?
        .iter()
        .find(|item| item["id"].as_str() == Some(id))
        .and_then(|item| serde_yaml::to_string(item).ok())
}

/// Its recent failures, newest first: a collector's failed runs (its
/// `collector.synced@1` errors), else what disabled it and its last error.
async fn recent_errors(svc: &Services, key: &PluginKey, reason: &str) -> Vec<String> {
    let mut errors = Vec::new();
    if key.kind == "collector" {
        let subject = oxplow_domain::refs::build::collector_ref(&key.plugin, &key.contribution);
        if let Ok(found) = svc
            .db
            .read(move |c| {
                let mut st = c
                    .prepare(
                        "SELECT json_extract(e.payload, '$.error') FROM event_log e
                          WHERE e.type = 'collector.synced'
                            AND json_extract(e.payload, '$.collector') = ?1
                            AND json_extract(e.payload, '$.status') = 'error'
                          ORDER BY e.seq DESC LIMIT ?2",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map(rusqlite::params![subject, ERRORS_SHOWN as i64], |r| {
                        r.get::<_, Option<String>>(0)
                    })
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows.filter_map(|r| r.ok().flatten()).collect::<Vec<_>>())
            })
            .await
        {
            errors = found;
        }
    }
    if key.kind == "effect" {
        // Its failed reactions (`v_effect_run`), newest first.
        let effect = format!("{}/{}", key.plugin, key.contribution);
        if let Ok(found) = svc
            .db
            .read(move |c| {
                let mut st = c
                    .prepare(
                        // Where each reaction stands: a failure a
                        // person's retry got past isn't one any more.
                        "SELECT reason FROM effect_run r
                          WHERE effect = ?1 AND state = 'failed' AND reason IS NOT NULL
                            AND attempt = (SELECT max(attempt) FROM effect_run l
                                            WHERE l.effect = r.effect AND l.event_id = r.event_id)
                          ORDER BY id DESC LIMIT ?2",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map(rusqlite::params![effect, ERRORS_SHOWN as i64], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows.filter_map(Result::ok).collect::<Vec<_>>())
            })
            .await
        {
            errors = found;
        }
    }
    if errors.is_empty() {
        errors.push(reason.to_string());
        let health = crate::plugin_health::PluginHealth::new(
            svc.db.clone(),
            svc.event_log_store.vocabulary().clone(),
        );
        if let Ok(Some(last)) = health.get(key).await.map(|r| r.and_then(|r| r.last_error)) {
            if !reason.contains(&last) {
                errors.push(last);
            }
        }
    }
    errors
}

pub struct PluginRepair {
    services: Weak<Services>,
}

impl PluginRepair {
    pub fn new(services: Weak<Services>) -> Self {
        Self { services }
    }
}

/// Register the consumer on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(PluginRepair::new(Arc::downgrade(svc))));
}

/// Whether `item` is still open (not done or canceled).
async fn is_open(svc: &Services, item: &str) -> Result<bool, DomainError> {
    let item = item.to_string();
    svc.db
        .read(move |c| {
            use rusqlite::OptionalExtension;
            c.query_row(
                "SELECT state FROM v_work_item WHERE ref = ?1",
                [item],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .map(|s| s.is_some_and(|s| s != "done" && s != "canceled"))
}

#[async_trait]
impl AsyncEventConsumer for PluginRepair {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "plugin.disabled"
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        let p = &event.envelope.payload;
        let text = |f: &str| p[f].as_str().unwrap_or_default().to_string();
        let key = PluginKey {
            plugin: text("plugin")
                .strip_prefix("plugin:")
                .unwrap_or_default()
                .to_string(),
            contribution: text("contribution"),
            kind: match text("kind").as_str() {
                "collector" => "collector",
                "effect" => "effect",
                _ => "provider",
            },
        };
        let reason = text("reason");
        let row = svc
            .db
            .read({
                let key = key.clone();
                move |c| store::get_tx(c, &key)
            })
            .await?;
        // A redelivery: this disable was handled.
        if row
            .as_ref()
            .and_then(|r| r.repair_seq)
            .is_some_and(|seq| seq >= event.seq)
        {
            return Ok(());
        }
        let open = match row.as_ref().and_then(|r| r.repair_item.clone()) {
            Some(item) if is_open(&svc, &item).await? => Some(item),
            _ => None,
        };
        let client = svc.work_items_client();
        let item = match open {
            Some(item) => {
                let errors = recent_errors(&svc, &key, &reason).await;
                let mut body = format!("Disabled again:\n\n> {}\n", reason.replace('\n', "\n> "));
                if errors.len() > 1 {
                    body.push_str("\nRecent failures (newest first):\n\n");
                    for e in errors.iter().take(ERRORS_SHOWN) {
                        body.push_str(&format!("- {}\n", e.replace('\n', " ")));
                    }
                }
                client
                    .comment(&Actor::System, &item, &body)
                    .await
                    .map_err(|e| DomainError::Invalid(format!("commenting on {item}: {e}")))?;
                item
            }
            None => {
                let context = gather(&svc, &key, &reason).await;
                let item = crate::work_items::NewItem {
                    title: title(&context),
                    body: render(&context),
                    ..Default::default()
                };
                // The active provider may be down — even the contribution
                // just disabled (tsk714): then oxplow's own tasks hold the
                // item, which says why.
                let active = svc.work_items.active();
                match client.create(&Actor::System, item.clone()).await {
                    Ok(item) => item,
                    Err(e) if active != crate::work_items::PROVIDER => {
                        let fallback = crate::work_items::NewItem {
                            provider: Some(crate::work_items::PROVIDER.into()),
                            body: format!(
                                "{}\n\n> Filed on oxplow: the active work-items provider `{active}` \
                                 couldn't take it ({e}).\n",
                                item.body
                            ),
                            ..item
                        };
                        client.create(&Actor::System, fallback).await.map_err(|e| {
                            DomainError::Invalid(format!("filing the repair item: {e}"))
                        })?
                    }
                    Err(e) => {
                        return Err(DomainError::Invalid(format!("filing the repair item: {e}")))
                    }
                }
            }
        };
        let seq = event.seq;
        svc.db
            .transaction(move |tx| store::set_repair_tx(tx, &key, &item, seq))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> RepairContext {
        RepairContext {
            plugin: "work".into(),
            contribution: "hot".into(),
            kind: "collector".into(),
            reason: "3 failures in a row; the last: collector `hot`: division by zero".into(),
            purpose: Some("Lists the high-priority tasks.".into()),
            origin: Some("effort:eff12".into()),
            manifest: Some("oxplow/extensions/work/extension.yaml".into()),
            declaration: Some("id: hot\nruntime: starlark\nentry: hot.star\n".into()),
            errors: vec![
                "collector `hot`: division by zero".into(),
                "collector `hot`: division by zero".into(),
            ],
            check: vec!["oxplow/extensions/work/extension.yaml:4: a warning".into()],
            examples: vec!["lists one".into()],
            engine: Some(">=0.7".into()),
            current_engine: "0.7.0".into(),
        }
    }

    /// The prompt's shape, pinned (`OXPLOW_BLESS=1` rewrites it).
    #[test]
    fn the_repair_prompt_golden() {
        let got = render(&context());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/repair-prompt.md");
        if std::env::var("OXPLOW_BLESS").is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &got).unwrap();
        }
        let want = std::fs::read_to_string(&path).expect("the golden exists (OXPLOW_BLESS=1)");
        assert_eq!(got, want);
        assert_eq!(
            title(&context()),
            "Repair work hot: 3 failures in a row; the last: collector `hot`: division by zero"
        );
    }

    async fn disable(svc: &Arc<Services>, reason: &str) -> StoredEvent {
        let health = crate::plugin_health::PluginHealth::new(
            svc.db.clone(),
            svc.event_log_store.vocabulary().clone(),
        );
        let key = crate::collector_runner::plugin_key("work", "hot");
        health.enable(&key, "human").await.unwrap();
        health.disable(&key, reason).await.unwrap();
        let events = svc.event_log_store.read_after(0, 500).await.unwrap();
        events
            .into_iter()
            .rev()
            .find(|e| e.envelope.event_type == "plugin.disabled")
            .unwrap()
    }

    async fn repair_items(svc: &Services) -> Vec<(String, String)> {
        let out = svc
            .sql
            .query_sql(
                "SELECT ref, title FROM v_work_item WHERE title LIKE 'Repair work hot:%' ORDER BY ref",
                vec![],
                None,
            )
            .await
            .unwrap();
        out.rows
            .into_iter()
            .map(|r| {
                let text = |c: &oxplow_db::SqlCell| match c {
                    oxplow_db::SqlCell::Text(t) => t.clone(),
                    other => format!("{other:?}"),
                };
                (text(&r[0]), text(&r[1]))
            })
            .collect()
    }

    /// P7 review (tsk714): when the active work-items provider isn't
    /// running — it may be the very contribution that was disabled — the
    /// repair item is filed on oxplow's own tasks, saying why, rather than
    /// lost.
    #[tokio::test]
    async fn a_repair_item_is_filed_on_oxplow_when_the_active_provider_is_down() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let ext = root.join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nsharing: private\nintent: { purpose: Lists hot tasks., origin: null, examples: [] }\ncollectors:\n  - { id: hot, runtime: starlark, entry: hot.star, entities: [{ name: hot, key: id, columns: { id: int } }] }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("hot.star"),
            "def transform(input):\n    return 1 // 0\n",
        )
        .unwrap();
        fx.svc.work_items.set_active("linear");
        let consumer = PluginRepair::new(Arc::downgrade(&fx.svc));
        let event = disable(&fx.svc, "3 failures in a row; the last: boom").await;
        consumer.handle(&event).await.unwrap();
        let items = repair_items(&fx.svc).await;
        assert_eq!(items.len(), 1, "{items:?}");
        assert!(items[0].0.starts_with("work_item:oxplow:"), "{items:?}");
        let body = fx
            .svc
            .sql
            .query_sql(
                "SELECT body FROM v_work_item WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(items[0].0.clone())],
                None,
            )
            .await
            .unwrap();
        let oxplow_db::SqlCell::Text(body) = &body.rows[0][0] else {
            panic!("a body");
        };
        assert!(
            body.contains("`linear`"),
            "says why it isn't on the active provider: {body}"
        );
    }

    /// A disable files a repair item whose body is the prompt; a repeat
    /// disable comments on the open item; a closed item lets the next
    /// disable file a new one.
    #[tokio::test]
    async fn a_disable_files_a_repair_item_and_a_repeat_comments_on_it() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let root = fx.svc.layout.project_dir.clone();
        let ext = root.join("oxplow/extensions/work");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "manifest: 2\nname: work\nsharing: private\nintent: { purpose: Lists hot tasks., origin: null, examples: [] }\ncollectors:\n  - { id: hot, runtime: starlark, entry: hot.star, entities: [{ name: hot, key: id, columns: { id: int } }] }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("hot.star"),
            "def transform(input):\n    return 1 // 0\n",
        )
        .unwrap();
        let consumer = PluginRepair::new(Arc::downgrade(&fx.svc));

        let first = disable(&fx.svc, "3 failures in a row; the last: boom").await;
        consumer.handle(&first).await.unwrap();
        consumer.handle(&first).await.unwrap(); // a redelivery files nothing
        let items = repair_items(&fx.svc).await;
        assert_eq!(items.len(), 1, "{items:?}");
        let (item, title) = items[0].clone();
        assert_eq!(
            title,
            "Repair work hot: 3 failures in a row; the last: boom"
        );
        let body = fx
            .svc
            .sql
            .query_sql(
                "SELECT body FROM v_work_item WHERE ref = ?1",
                vec![oxplow_db::SqlCell::Text(item.clone())],
                None,
            )
            .await
            .unwrap();
        let oxplow_db::SqlCell::Text(body) = &body.rows[0][0] else {
            panic!("a body");
        };
        assert!(
            body.contains("Lists hot tasks.") && body.contains("id: hot"),
            "{body}"
        );
        let shown = fx
            .svc
            .sql
            .query_sql(
                "SELECT repair_item FROM v_plugin_health WHERE plugin = 'work'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(shown.rows[0][0], oxplow_db::SqlCell::Text(item.clone()));

        let again = disable(&fx.svc, "3 failures in a row; the last: boom again").await;
        consumer.handle(&again).await.unwrap();
        assert_eq!(
            repair_items(&fx.svc).await.len(),
            1,
            "the open item is reused"
        );
        let comments = fx
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_event WHERE type = 'work_item.commented'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(comments.rows[0][0], oxplow_db::SqlCell::Int(1));

        // Closed: the next disable files a new one.
        fx.svc
            .work_items_client()
            .transition(
                &Actor::Human,
                &item,
                oxplow_domain::work_items::CanonicalState::Done,
                None,
            )
            .await
            .unwrap();
        let third = disable(&fx.svc, "3 failures in a row; the last: boom thrice").await;
        consumer.handle(&third).await.unwrap();
        assert_eq!(repair_items(&fx.svc).await.len(), 2);
    }
}
