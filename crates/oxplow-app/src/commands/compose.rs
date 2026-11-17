//! Composition: a command made of other commands' calls (P6b.A1) — the
//! one mechanism `oxplow.command.sequence` and an extension's own commands
//! (P6b.B2) are built on. A composite is a [`Compose`] handler: given its
//! input it says which calls to run; the bus decides where they run.
//!
//! - Every call runs in the transaction (a `Tx` command, a composite whose
//!   calls all do): one run in one transaction (`steps::run_composed`) —
//!   all or nothing, one audit row and `command.executed` with the
//!   children's events caused by it, undone by the inverses of the
//!   children that changed something, reversed.
//! - Any call leaves it (an `External` command, `task.*` on another
//!   provider's item): its **steps** run in order (`steps.rs`), each
//!   landing as it runs, with no undo (P7 review).
//!
//! Either way it is composed once, before any transaction, its reads
//! through the SQL gateway (as an effect's are), and each child's own
//! needs, policy and confirmation — a composite child's
//! children's too — are checked before anything runs.

use std::sync::Arc;

use oxplow_domain::{
    Access, Atomicity, CommandCall, CommandError, CommandSpec, Confirm, Lifecycle,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{Command, Handler};

/// What a composite runs for one input: its calls, in order, the run's
/// own `result` (beside the children's), and events of its own, appended
/// after the children's and caused by the run's `command.executed` (an
/// extension command's declared types, P8.D4).
#[derive(Debug, Clone, Default)]
pub struct Composition {
    pub calls: Vec<CommandCall>,
    pub result: Option<Value>,
    pub events: Vec<oxplow_domain::Envelope>,
    /// The calls that reverse `calls`, when the composer says so (an
    /// extension script's `inverse`): the run's undo when its calls leave
    /// oxplow's records (steps, which have none of their own). Refused
    /// beside calls that all stay inside, which undo themselves.
    pub inverse: Option<Vec<CommandCall>>,
}

/// What a composer reads through: the SQL gateway (the published datasets,
/// the metric functions, extension state, configuration and jobs) and the
/// database, from the blocking thread the bus routes on — so a read may
/// block on `runtime`, and no connection is held while it does.
pub struct Composing<'a> {
    pub sql: &'a crate::sql_gateway::SqlGateway,
    pub db: &'a oxplow_db::Database,
    pub runtime: &'a tokio::runtime::Handle,
}

/// Say what a composite runs for `input`, reading through `composing`, as
/// part of `run` (a pure child of the run: the scopes it calls are counted
/// on its trace). The bus composes once per run, before the transaction:
/// what it routed and checked is what runs.
pub type Composer = dyn Fn(&Composing<'_>, &oxplow_domain::run::RunContext, &Value) -> Result<Composition, CommandError>
    + Send
    + Sync;

/// What one step of a run answered, as an extension script's `finish`
/// sees it: the call's name and input and the answer it gave (a store
/// write's carries `rows`, its `RETURNING` rows).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct StepAnswer {
    pub name: String,
    pub input: Value,
    pub result: Value,
}

/// What a composite's `finish` decided once all its steps landed: the
/// run's `result` and the `inverse` calls that undo it. A part it leaves
/// out keeps what the composer said.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Finished {
    pub result: Option<Value>,
    pub inverse: Option<Vec<CommandCall>>,
}

/// Say, after every step of a composite landed, what its run answers and
/// how it is undone: given the input the run was called with and each
/// step's answer, reading through `Composing` (what the composer reads
/// through) as part of `run`. Pure: it writes nothing and composes nothing.
/// `None` for the reads: the run is inside the bus's transaction, which
/// holds the database, so `finish` can't read (a read would wait for it).
pub type Finisher = dyn Fn(
        Option<&Composing<'_>>,
        &oxplow_domain::run::RunContext,
        &Value,
        &[StepAnswer],
    ) -> Result<Finished, CommandError>
    + Send
    + Sync;

/// What a [`Composing`] borrows, owned: the bus keeps one with a routed
/// composition so a `finish` that runs later, in a transaction or after
/// the steps, can read as the composer did.
#[derive(Clone)]
pub struct Reads {
    pub sql: crate::sql_gateway::SqlGateway,
    pub db: oxplow_db::Database,
    pub runtime: tokio::runtime::Handle,
}

impl Reads {
    pub fn composing(&self) -> Composing<'_> {
        Composing {
            sql: &self.sql,
            db: &self.db,
            runtime: &self.runtime,
        }
    }
}

/// A composite's handler: its composer, and its `finish` when it has
/// one. The bus composes it, routes the calls and runs them (`steps.rs`).
pub struct Compose {
    pub compose: Arc<Composer>,
    /// Called after all its steps landed (an extension script's `finish`).
    pub finish: Option<Arc<Finisher>>,
    /// Native API result: keep child details in audit, return only the finished value.
    pub(crate) transparent_result: bool,
}

impl Compose {
    /// The composite over `compose`.
    pub fn handler(compose: Arc<Composer>) -> Handler {
        Handler::Compose(Arc::new(Compose {
            compose,
            finish: None,
            transparent_result: false,
        }))
    }

    /// The composite over `compose`, whose run `finish` answers.
    pub fn handler_finishing(compose: Arc<Composer>, finish: Option<Arc<Finisher>>) -> Handler {
        Handler::Compose(Arc::new(Compose {
            compose,
            finish,
            transparent_result: false,
        }))
    }
}

pub const SEQUENCE: &str = "oxplow.command.sequence";

/// `oxplow.command.sequence`: the commands to run, in order.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceInput {
    pub calls: Vec<SequenceCall>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SequenceCall {
    /// A registered command that writes.
    pub name: String,
    /// Its input, matching its schema.
    pub input: Value,
}

/// `SequenceInput`'s schema, with its example input: handing work to two
/// threads, one audited run.
fn sequence_schema() -> Value {
    let mut schema =
        serde_json::to_value(schemars::schema_for!(SequenceInput)).expect("schema serializes");
    if let Some(object) = schema.as_object_mut() {
        object.insert(
            "examples".into(),
            serde_json::json!([{
                "calls": [
                    {
                        "name": "oxplow.handoff.send",
                        "input": { "to_thread": "thr3", "body": "The login redirect loop is fixed; please re-run the e2e specs." }
                    },
                    {
                        "name": "oxplow.handoff.send",
                        "input": { "to_thread": "thr4", "body": "The login redirect loop is fixed; docs need the new flow." }
                    }
                ]
            }]),
        );
    }
    schema
}

/// `oxplow.command.sequence`'s spec: also what an extension effect's run is
/// audited as (`effect_triggers`).
pub fn sequence_spec() -> CommandSpec {
    CommandSpec {
        output_schema: None,
        id: SEQUENCE.into(),
        summary: "Run several commands as one, each one's own policy and confirmation \
                  checked before any runs; the run has one audit row. In oxplow's own \
                  records they run in one transaction and undo together; when one goes to \
                  an external system they run in order, each landing as it runs, with no undo."
            .into(),
        input_schema: sequence_schema(),
        job: false,
        // The children decide: one that asks makes the sequence ask.
        confirm: Confirm::Never,
        confirm_says: None,
        undoable: true,
        lifecycle: Lifecycle::Stable,
        // In the transaction, or as steps outside it: its calls decide.
        atomicity: Atomicity::Dispatch,
        access: Access::Write,
        needs: Vec::new(),
        ui: None,
        op: None,
        unrecorded: Vec::new(),
    }
}

pub fn sequence_command() -> Command {
    let spec = sequence_spec();
    let compose: Arc<Composer> = Arc::new(|_conn, _run, input: &Value| {
        let input: SequenceInput =
            serde_json::from_value(input.clone()).map_err(|e| CommandError::Invalid {
                field: None,
                message: e.to_string(),
            })?;
        Ok(Composition {
            calls: input
                .calls
                .into_iter()
                .map(|c| CommandCall {
                    name: c.name,
                    input: c.input,
                })
                .collect(),
            result: None,
            events: Vec::new(),
            inverse: None,
        })
    });
    Command::new(spec, Compose::handler(compose)).expect("command.sequence registers")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::stores::ThreadStore as _;
    use oxplow_domain::Actor;
    use serde_json::json;

    /// Two thread commands as one run by a person: the sequence runs them
    /// in one transaction — one audit row naming the sequence with the
    /// children in its result, one `command.executed`, and one undo that
    /// reverses both.
    #[tokio::test]
    async fn a_sequence_of_commands_is_one_audited_run() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let thread = fx.thread.to_string();
        let before = fx.svc.thread_store.get(&fx.thread).await.unwrap().unwrap();
        let out = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                SEQUENCE,
                json!({ "calls": [
                    { "name": "oxplow.thread.rename", "input": { "thread": thread, "title": "First" } },
                    { "name": "oxplow.thread.rename", "input": { "thread": thread, "title": "Second" } },
                ] }),
                false,
            )
            .await
            .unwrap();
        let after = fx.svc.thread_store.get(&fx.thread).await.unwrap().unwrap();
        assert_eq!(after.title, "Second");
        assert_eq!(out.result["children"].as_array().unwrap().len(), 2);
        assert_eq!(out.result["children"][1]["name"], "oxplow.thread.rename");
        assert!(
            out.inverse.is_some(),
            "a sequence in one transaction undoes"
        );

        let audits = oxplow_db::SqliteCommandAuditStore::new(fx.svc.db.clone())
            .list_recent(20)
            .await
            .unwrap();
        let ok: Vec<_> = audits
            .iter()
            .filter(|a| a.outcome == oxplow_domain::events::schema::CommandOutcome::Ok)
            .collect();
        assert_eq!(ok.len(), 1, "{audits:?}");
        assert_eq!(ok[0].command, SEQUENCE);

        let events = fx.svc.event_log_store.read_after(0, 1000).await.unwrap();
        let executed = events
            .iter()
            .filter(|e| e.envelope.event_type == "command.executed")
            .count();
        assert_eq!(executed, 1);

        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        let undone = fx.svc.thread_store.get(&fx.thread).await.unwrap().unwrap();
        assert_eq!(undone.title, before.title);
    }
}

#[cfg(test)]
mod secret_recording_tests {
    use super::*;
    use crate::commands::{HandlerOutput, Invocation, TxCtx};
    use oxplow_config::settings::Level;
    use oxplow_domain::{Actor, Envelope};
    use serde_json::json;

    const MARKER: &str = "secret-audit-marker-do-not-record";

    fn spec(id: &str, atomicity: Atomicity) -> CommandSpec {
        let mut spec = sequence_spec();
        spec.id = id.into();
        spec.input_schema = json!({"type":"object"});
        spec.atomicity = atomicity;
        spec
    }

    fn answer(input: Value) -> HandlerOutput {
        HandlerOutput {
            result: input.clone(),
            inverse: Some(CommandCall {
                name: "oxplow.secret_test.leaf".into(),
                input: input.clone(),
            }),
            events: vec![Envelope::typed::<
                oxplow_domain::events::schema::ConfigChanged,
            >(
                "test",
                &oxplow_domain::events::schema::ConfigChangedV3 {
                    key: "test.secret-derived".into(),
                    before: Value::Null,
                    after: input,
                    layer: oxplow_domain::settings::Level::Project,
                },
            )],
            ..Default::default()
        }
    }

    async fn assert_no_secret_history(svc: &crate::Services) {
        let audits = oxplow_db::SqliteCommandAuditStore::new(svc.db.clone())
            .list_recent(100)
            .await
            .unwrap();
        assert!(!format!("{audits:?}").contains(MARKER), "{audits:?}");
        let events = svc.event_log_store.read_after(0, 1000).await.unwrap();
        assert!(!format!("{events:?}").contains(MARKER));
        assert!(!events
            .iter()
            .any(|event| event.envelope.event_type == "config.changed"
                && event.envelope.payload["key"] == "test.secret-derived"));
    }

    /// The authorized read stays useful, while arbitrary transformations of
    /// its value cannot enter child audit inputs, results, events or undo.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_secret_read_derived_composite_keeps_values_out_of_history() {
        for external in [false, true] {
            let fx = crate::test_fixtures::services_with_thread().await;
            let global = fx.svc.layout.state_dir.join("global-config");
            let store = crate::settings::store(
                &fx.svc.layout.project_dir,
                &global,
                fx.svc.secrets.clone(),
                &[],
            )
            .unwrap();
            store
                .registry
                .write()
                .unwrap()
                .register_many(crate::settings::ai_provider_definitions("openai"))
                .unwrap();
            let key = crate::ai_service::provider_key_setting("openai");
            store.set(&key, json!(MARKER), Level::Local).unwrap();
            let bus = fx.svc.commands.clone();
            let read = bus
                .run(
                    &Actor::Human,
                    "oxplow.config.get",
                    json!({"key":key}),
                    false,
                )
                .await
                .unwrap();
            assert_eq!(read.result["value"], MARKER);
            let handler = if external {
                Handler::External(Arc::new(|_: Invocation, input| {
                    Box::pin(async move { Ok(answer(input)) })
                }))
            } else {
                Handler::Tx(Arc::new(|_: &TxCtx<'_>, input| Ok(answer(input))))
            };
            bus.register(
                Command::new(
                    spec(
                        "oxplow.secret_test.leaf",
                        if external {
                            Atomicity::External
                        } else {
                            Atomicity::Tx
                        },
                    ),
                    handler,
                )
                .unwrap(),
            )
            .unwrap();
            let compose = Arc::new(
                move |_: &Composing<'_>, run: &oxplow_domain::run::RunContext, _: &Value| {
                    let setting = store.get(&key, None).unwrap();
                    assert!(setting.secret);
                    run.trace().count("config.secret");
                    let derived = json!({"arbitrary":{"nested":[format!("prefix:{}", setting.value.unwrap().as_str().unwrap())]}});
                    Ok(Composition {
                        calls: vec![CommandCall {
                            name: "oxplow.secret_test.leaf".into(),
                            input: derived.clone(),
                        }],
                        result: Some(derived.clone()),
                        inverse: external.then(|| {
                            vec![CommandCall {
                                name: "oxplow.secret_test.leaf".into(),
                                input: derived,
                            }]
                        }),
                        ..Default::default()
                    })
                },
            );
            bus.register(
                Command::new(
                    spec("oxplow.secret_test.compose", Atomicity::Dispatch),
                    Compose::handler(compose),
                )
                .unwrap(),
            )
            .unwrap();
            let result = bus
                .run(&Actor::Human, "oxplow.secret_test.compose", json!({}), true)
                .await
                .unwrap();
            assert!(
                result.result.to_string().contains(MARKER),
                "authorized callers receive the result"
            );
            assert!(result.inverse.is_none());
            assert_no_secret_history(&fx.svc).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_secret_read_failed_composer_redacts_its_error() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let compose = Arc::new(
            |_: &Composing<'_>, run: &oxplow_domain::run::RunContext, _: &Value| {
                run.trace().count("config.secret");
                Err(CommandError::Failed {
                    message: MARKER.into(),
                })
            },
        );
        fx.svc
            .commands
            .register(
                Command::new(
                    spec("oxplow.secret_test.fail", Atomicity::Dispatch),
                    Compose::handler(compose),
                )
                .unwrap(),
            )
            .unwrap();
        let error = fx
            .svc
            .commands
            .run(&Actor::Human, "oxplow.secret_test.fail", json!({}), true)
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(MARKER));
        assert!(error
            .to_string()
            .contains("secret configuration consumer failed"));
        assert_no_secret_history(&fx.svc).await;
    }
}
