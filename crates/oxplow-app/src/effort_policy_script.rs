//! An effort policy written as a script (`implementations:` with a `.star`
//! entry, `.context/work-tracking.md`): [`ScriptedPolicy`] is the
//! [`EffortPolicy`] the dispatcher calls while it's the project's choice.
//! Its script is an effect's shape — `transform({event})` →
//! `{ commands: [{ name, input }] }` or `{ skip: "why" }` — run under the
//! effects' sandbox, reading oxplow through the scopes its entry `needs`.
//! It runs only while a person has approved it as it is
//! (`ProgramKind::EffortPolicy`); unapproved, each event it's offered fails
//! saying so, and the pump dead-letters it. Its reads aren't audited:
//! `react` is no command run; what it composes is, as
//! `effect:effort_policy:<id>`.

use std::sync::Arc;

use async_trait::async_trait;
use oxplow_domain::effort_policy::{EffortPolicy, PolicyEvent};
use oxplow_domain::{CommandCall, DomainError};

use crate::effects::Reaction;
use crate::exec_consent::Gate;
use crate::sql_gateway::SqlGateway;

pub struct ScriptedPolicy {
    pub id: String,
    pub script: Arc<str>,
    /// The scopes it may call.
    pub needs: Vec<String>,
    pub sql: SqlGateway,
    /// Whether a person approved it as it is now, asked before each event.
    pub gate: Gate,
}

#[async_trait]
impl EffortPolicy for ScriptedPolicy {
    fn id(&self) -> &str {
        &self.id
    }

    async fn react(&self, event: &PolicyEvent) -> Result<Vec<CommandCall>, DomainError> {
        let failed = |message: String| {
            DomainError::Invariant(format!("effort policy `{}`: {message}", self.id))
        };
        (self.gate)().map_err(&failed)?;
        let event = serde_json::to_value(event).map_err(|e| failed(e.to_string()))?;
        match crate::effects::run_over(&self.sql, &self.needs, &self.script, event, None)
            .await
            .map_err(&failed)?
        {
            Reaction::Skip(_) => Ok(Vec::new()),
            Reaction::Run { events, .. } if !events.is_empty() => Err(failed(
                "it composed events, and an effort policy emits none".into(),
            )),
            Reaction::Run { calls, .. } => Ok(calls),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::exec_consent::{approve_program, list, ProgramKind};
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_db::SqlCell;
    use oxplow_domain::Actor;
    use serde_json::json;

    /// Opens an effort for an item an agent starts on its thread, closes
    /// the item's open efforts when it's finished — reading them through
    /// `sql.read`.
    const TIDY: &str = r#"
def transform(x):
    e = x["event"]
    if e["type"] != "work_item.state_changed":
        return {"skip": "not an item's move"}
    item = e["payload"]["work_item"]
    to = e["payload"]["to"]
    thread = e["anchors"].get("thread_id")
    if to == "in_progress" and thread != None:
        return {"commands": [{"name": "oxplow.effort.open",
                              "input": {"thread": "thread:" + thread, "work_item": item}}]}
    if to == "done" or to == "canceled":
        rows = scope("sql.read", {"sql": "SELECT id FROM v_effort WHERE work_item = :w AND ended_at IS NULL",
                                  "params": {"w": item}})
        return {"commands": [{"name": "oxplow.effort.close",
                              "input": {"effort": "effort:eff" + str(r["id"]), "reason": "switch"}}
                             for r in rows]}
    return {"skip": "a move that changes no effort"}
"#;

    /// The fixture with a private extension declaring `script` as the
    /// effort policy `tidy`, chosen as the project's, the fixture's own
    /// effort closed so the policy's are the only ones.
    async fn scripted(script: &str) -> EffortFixture {
        let fx = services_with_effort().await;
        install(&fx, script).await;
        fx
    }

    /// `script` as the private extension `acme`'s effort policy `tidy`,
    /// chosen as the project's, the fixture's own effort closed.
    async fn install(fx: &EffortFixture, script: &str) {
        let dir = fx.svc.layout.project_dir.join("oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("policies")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: acme\nsharing: private\nintent:\n  purpose: a policy\n  examples: [{ name: a }]\nimplementations:\n  - { capability: effort_policy, id: tidy, title: Tidy, entry: policies/tidy.star, needs: [sql.read] }\n",
        )
        .unwrap();
        std::fs::write(dir.join("policies/tidy.star"), script).unwrap();
        crate::capabilities::refresh(&fx.svc).await.unwrap();
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert(crate::effort_policy::CAPABILITY.into(), "tidy".into());
        let config = crate::config_service::read_config(&fx.svc.config);
        fx.svc
            .capabilities
            .publish(&config, &fx.svc.db)
            .await
            .unwrap();
        fx.svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::effort::CLOSE,
                json!({ "effort": oxplow_domain::refs::build::effort_ref(fx.effort) }),
                false,
            )
            .await
            .unwrap();
    }

    fn approve(fx: &EffortFixture) {
        let project = fx.svc.layout.project_dir.clone();
        let config = fx.svc.config.read().unwrap().clone();
        let extensions = fx.svc.extension_catalog.get(&project);
        let program = list(&fx.svc.approvals, &project, &config, &extensions)
            .into_iter()
            .find(|p| p.kind == ProgramKind::EffortPolicy)
            .expect("the policy is a program");
        approve_program(
            &fx.svc.approvals,
            &project,
            &config,
            &extensions,
            program.kind,
            &program.name,
            program.version.as_deref().unwrap(),
        )
        .unwrap();
    }

    async fn task(fx: &EffortFixture) -> String {
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                crate::commands::work_item::CREATE,
                json!({ "title": "scripted", "thread": fx.thread.to_string() }),
                false,
            )
            .await
            .unwrap();
        out.result["ref"].as_str().unwrap().to_string()
    }

    async fn agent_moves(fx: &EffortFixture, item: &str, to: &str) {
        let agent = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: Some(oxplow_domain::StreamId::new(1)),
        };
        fx.svc
            .commands
            .run(
                &agent,
                crate::commands::work_item::NAME,
                json!({ "ref": item, "to": to }),
                false,
            )
            .await
            .unwrap();
        fx.svc
            .event_pump
            .settle(
                &[crate::effort_policy::NAME],
                std::time::Duration::from_secs(10),
            )
            .await;
    }

    /// The thread's open efforts, linked to `item`.
    async fn open_for(fx: &EffortFixture, item: &str) -> i64 {
        let rows = fx
            .svc
            .sql
            .query_sql(
                "SELECT count(*) FROM v_effort WHERE thread_id = ?1 AND work_item = ?2 AND ended_at IS NULL",
                vec![SqlCell::Int(fx.thread.value()), SqlCell::Text(item.into())],
                None,
            )
            .await
            .unwrap()
            .rows;
        match rows[0][0] {
            SqlCell::Int(n) => n,
            _ => -1,
        }
    }

    async fn sources(fx: &EffortFixture, event_type: &str) -> Vec<String> {
        fx.svc
            .sql
            .query_sql(
                "SELECT source FROM v_event WHERE type = ?1 ORDER BY seq",
                vec![SqlCell::Text(event_type.into())],
                None,
            )
            .await
            .unwrap()
            .rows
            .into_iter()
            .filter_map(|r| match &r[0] {
                SqlCell::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    async fn dead_letters(fx: &EffortFixture) -> Vec<String> {
        fx.svc
            .sql
            .query_sql(
                "SELECT error FROM v_event_dead_letter WHERE consumer = ?1",
                vec![SqlCell::Text(crate::effort_policy::NAME.into())],
                None,
            )
            .await
            .unwrap()
            .rows
            .into_iter()
            .filter_map(|r| match &r[0] {
                SqlCell::Text(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// Approved and chosen, the script is the policy: an agent starting a
    /// task on its thread opens an effort linked to it, as the policy's own
    /// effect, and finishing it closes the effort its read found.
    #[tokio::test]
    async fn an_approved_policy_script_opens_and_closes_efforts() {
        let fx = scripted(TIDY).await;
        approve(&fx);
        let item = task(&fx).await;
        agent_moves(&fx, &item, "in_progress").await;
        assert_eq!(open_for(&fx, &item).await, 1);
        assert_eq!(
            sources(&fx, "effort.opened")
                .await
                .last()
                .map(String::as_str),
            Some("effect:effort_policy:tidy")
        );
        agent_moves(&fx, &item, "done").await;
        assert_eq!(open_for(&fx, &item).await, 0);
        assert_eq!(dead_letters(&fx).await, Vec::<String>::new());
    }

    /// Unapproved, it opens nothing and the event is dead-lettered saying
    /// why; approving it makes the next event run. An edit to any file of
    /// its extension stops it again.
    #[tokio::test]
    async fn an_unapproved_policy_script_runs_nothing_until_approved() {
        let fx = scripted(TIDY).await;
        let first = task(&fx).await;
        agent_moves(&fx, &first, "in_progress").await;
        assert_eq!(open_for(&fx, &first).await, 0);
        let letters = dead_letters(&fx).await;
        assert!(
            letters
                .iter()
                .any(|e| e.contains("Settings → Data → Programs")),
            "{letters:?}"
        );
        approve(&fx);
        let second = task(&fx).await;
        agent_moves(&fx, &second, "in_progress").await;
        assert_eq!(open_for(&fx, &second).await, 1);

        std::fs::write(
            fx.svc
                .layout
                .project_dir
                .join("oxplow/extensions/acme/README.md"),
            "# Acme\n",
        )
        .unwrap();
        crate::capabilities::refresh(&fx.svc).await.unwrap();
        let third = task(&fx).await;
        agent_moves(&fx, &third, "in_progress").await;
        assert_eq!(open_for(&fx, &third).await, 0);
    }

    /// A policy emits nothing of its own: a script composing events runs
    /// nothing.
    #[tokio::test]
    async fn a_policy_script_composing_events_runs_nothing() {
        let fx = scripted(
            "def transform(x):\n    return {\"commands\": [], \"events\": [{\"type\": \"acme.x\", \"payload\": {}}]}\n",
        )
        .await;
        approve(&fx);
        let item = task(&fx).await;
        agent_moves(&fx, &item, "in_progress").await;
        let letters = dead_letters(&fx).await;
        assert!(
            letters.iter().any(|e| e.contains("emits none")),
            "{letters:?}"
        );
    }

    /// A catalog reload restates the declared script, so it stays the
    /// policy.
    #[tokio::test]
    async fn a_reload_keeps_the_policy_script() {
        let fx = scripted(TIDY).await;
        approve(&fx);
        crate::capabilities::refresh(&fx.svc).await.unwrap();
        assert!(fx.svc.effort_policies.has("tidy"));
        let item = task(&fx).await;
        agent_moves(&fx, &item, "in_progress").await;
        assert_eq!(open_for(&fx, &item).await, 1);
    }

    /// The one-effort-per-prompt example (`examples/extensions/
    /// prompt-efforts`) over real turns: each prompt that changes files gets
    /// an effort of its own — the previous one closed as of the next turn's
    /// start — and a read-only prompt gets none.
    #[tokio::test]
    async fn the_prompt_efforts_example_gives_each_changing_prompt_its_own_effort() {
        const EXAMPLE: &str = include_str!(
            "../../../examples/extensions/prompt-efforts/policies/prompt_efforts.star"
        );
        let fx = crate::thread_checkpoint::tests::with_baseline().await;
        install(&fx, EXAMPLE).await;
        approve(&fx);
        let settle = || async {
            fx.svc
                .event_pump
                .settle(
                    &[crate::effort_policy::NAME],
                    std::time::Duration::from_secs(10),
                )
                .await;
        };
        let policy_efforts = || async {
            fx.svc
                .sql
                .query_sql(
                    "SELECT id, CASE WHEN ended_at IS NULL THEN 'open' ELSE 'closed' END
                       FROM v_effort WHERE thread_id = ?1 AND id <> ?2 ORDER BY id",
                    vec![
                        SqlCell::Int(fx.thread.value()),
                        SqlCell::Int(fx.effort.value()),
                    ],
                    None,
                )
                .await
                .unwrap()
                .rows
        };
        crate::thread_checkpoint::tests::turn(&fx, Some(("one.txt", "1")), &["edit"]).await;
        settle().await;
        assert_eq!(policy_efforts().await.len(), 1, "the first prompt's effort");
        crate::thread_checkpoint::tests::turn(&fx, None, &["read"]).await;
        settle().await;
        assert_eq!(
            policy_efforts().await.len(),
            1,
            "a read-only prompt gets none"
        );
        crate::thread_checkpoint::tests::turn(&fx, Some(("two.txt", "2")), &["edit"]).await;
        settle().await;
        let efforts = policy_efforts().await;
        assert_eq!(efforts.len(), 2, "{efforts:?}");
        assert_eq!(efforts[0][1], SqlCell::Text("closed".into()));
        assert_eq!(efforts[1][1], SqlCell::Text("open".into()));
        assert_eq!(dead_letters(&fx).await, Vec::<String>::new());
        assert_eq!(
            sources(&fx, "effort.opened")
                .await
                .last()
                .map(String::as_str),
            Some("effect:effort_policy:tidy")
        );
    }
}
