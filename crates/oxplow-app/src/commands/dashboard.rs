//! Dashboard commands (P8.A5, `.context/dashboards.md`): a dashboard and
//! its tiles, written through the bus by the desktop, an agent or a lens.
//! Creating, renaming, deleting a dashboard and removing or reordering its
//! tiles are `Tx` over the dashboard store's `_tx` cores. Adding a tile and
//! editing one are `External`: a query tile's SQL is checked by the
//! semantic engine first — the read contract a lens is held to, with
//! `MEASURE()` resolved — which is async, so it can't run inside the
//! bus's transaction; the write that follows is one statement. Dashboards
//! and tiles are named by id (`dsh3`, `dti7`).

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::dashboard_store::{
    add_item_tx, create_tx, dashboard_tx, delete_tx, item_tx, items_tx, remove_item_tx, rename_tx,
    reorder_items_tx, update_item_tx,
};
use oxplow_db::Database;
use oxplow_domain::{CommandCall, CommandError, Confirm, DashboardId, DashboardItemId, Invokers};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::util::{invalid, parse, schema};
use super::{Handler, HandlerOutput, Invocation, TxCtx};
use crate::dashboard_tiles::{new_tile, TileInput};
use crate::sql_gateway::SqlGateway;

pub const CREATE: &str = "oxplow.dashboard.create";
pub const RENAME: &str = "oxplow.dashboard.rename";
pub const DELETE: &str = "oxplow.dashboard.delete";
pub const ADD_ITEM: &str = "oxplow.dashboard.add_item";
pub const UPDATE_ITEM: &str = "oxplow.dashboard.update_item";
pub const REMOVE_ITEM: &str = "oxplow.dashboard.remove_item";
pub const REORDER_ITEMS: &str = "oxplow.dashboard.reorder_items";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateInput {
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The dashboard (`dsh3`).
    pub dashboard: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DashboardInput {
    /// The dashboard (`dsh3`).
    pub dashboard: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddItemInput {
    /// The dashboard (`dsh3`).
    pub dashboard: String,
    /// `query` (pinned SQL), `lens` or `text`.
    pub kind: String,
    /// A `query` tile's SQL — read-only, over published models, checked
    /// like `query_sql`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    /// A `query` tile's display: a lens viz, or `metric` (the metric card,
    /// with the metric key as `metric` in `options_json`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// A `lens` tile's lens (`<extension>/<slug>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens_id: Option<String>,
    /// Anything else the tile carries (size, title, a text tile's `text`,
    /// a chart's `chart`), as JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_json: Option<String>,
    /// Where it goes (0 = first); the end when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateItemInput {
    /// The tile (`dti7`).
    pub item: String,
    /// Its options JSON (a query tile's `sql` is checked again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemInput {
    /// The tile (`dti7`).
    pub item: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReorderItemsInput {
    /// The dashboard (`dsh3`).
    pub dashboard: String,
    /// Its tiles in their new order (`dti7`, …).
    pub order: Vec<String>,
}

fn dashboard_id(value: &str, field: &str) -> Result<DashboardId, CommandError> {
    DashboardId::try_from_str(value)
        .ok_or_else(|| invalid(field, format!("`{value}` isn't a dashboard id (dsh…)")))
}

fn item_id(value: &str, field: &str) -> Result<DashboardItemId, CommandError> {
    DashboardItemId::try_from_str(value)
        .ok_or_else(|| invalid(field, format!("`{value}` isn't a tile id (dti…)")))
}

/// A tile's options as a check sees them: invalid input is the caller's.
fn tile_error(e: oxplow_domain::DomainError, field: &str) -> CommandError {
    match e {
        oxplow_domain::DomainError::Invalid(m) => invalid(field, m),
        other => CommandError::from(other),
    }
}

fn exists(conn: &rusqlite::Connection, id: DashboardId, field: &str) -> Result<(), CommandError> {
    dashboard_tx(conn, id)?
        .map(|_| ())
        .ok_or_else(|| invalid(field, format!("no dashboard `{id}`")))
}

/// `dashboard.create { title }`: an empty dashboard at the end of the list.
pub fn create_op() -> Op {
    Op::new(
        "dashboards.write",
        "create",
        schema::<CreateInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: CreateInput = parse(input)?;
            let id = create_tx(ctx.conn, &input.title)?;
            let created = dashboard_tx(ctx.conn, id)?.expect("just created");
            Ok(HandlerOutput {
                result: serde_json::to_value(created).expect("a dashboard serializes"),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `dashboard.rename { dashboard, title }`; undone by renaming it back.
pub fn rename_op() -> Op {
    Op::new(
        "dashboards.write",
        "rename",
        schema::<RenameInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: RenameInput = parse(input)?;
            let id = dashboard_id(&input.dashboard, "/dashboard")?;
            let before = dashboard_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/dashboard", format!("no dashboard `{id}`")))?
                .title;
            rename_tx(ctx.conn, id, &input.title)?;
            Ok(HandlerOutput {
                result: json!({ "dashboard": input.dashboard, "title": input.title }),
                inverse: Some(CommandCall {
                    name: RENAME.into(),
                    input: json!({ "dashboard": input.dashboard, "title": before }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `dashboard.delete { dashboard }`: it and its tiles. A person's,
/// confirmed; not undoable.
pub fn delete_op() -> Op {
    Op::new(
        "dashboards.write",
        "delete",
        schema::<DashboardInput>(),
        false,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: DashboardInput = parse(input)?;
            let id = dashboard_id(&input.dashboard, "/dashboard")?;
            exists(ctx.conn, id, "/dashboard")?;
            delete_tx(ctx.conn, id)?;
            Ok(HandlerOutput {
                result: json!({ "dashboard": input.dashboard, "deleted": true }),
                ..HandlerOutput::default()
            })
        })),
    )
    .open_to(Invokers::NO_AGENT)
    .confirm_at_least(Confirm::Destructive)
}

/// `dashboard.add_item { dashboard, kind, sql?, display?, lens_id?,
/// options_json?, position? }`: the tile checked (a query tile's SQL by the
/// semantic engine), then written. Undone by removing it.
pub fn add_item_op(db: Database, sql: SqlGateway) -> Op {
    Op::new(
        "dashboards.write",
        "add_item",
        schema::<AddItemInput>(),
        true,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let (db, sql) = (db.clone(), sql.clone());
            Box::pin(async move {
                let input: AddItemInput = parse(input)?;
                let dashboard = dashboard_id(&input.dashboard, "/dashboard")?;
                let tile = new_tile(
                    &sql,
                    TileInput {
                        kind: input.kind,
                        sql: input.sql,
                        display: input.display,
                        lens_id: input.lens_id,
                        options_json: input.options_json,
                    },
                )
                .await
                .map_err(|e| tile_error(e, ""))?;
                let position = input.position;
                let id = db
                    .transaction(move |tx| {
                        if dashboard_tx(tx, dashboard)?.is_none() {
                            return Err(oxplow_domain::DomainError::Invalid(format!(
                                "no dashboard `{dashboard}`"
                            )));
                        }
                        add_item_tx(tx, dashboard, &tile, position)
                    })
                    .await
                    .map_err(|e| tile_error(e, "/dashboard"))?;
                Ok(HandlerOutput {
                    result: json!({ "id": id }),
                    inverse: Some(CommandCall {
                        name: REMOVE_ITEM.into(),
                        input: json!({ "item": id }),
                    }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// `dashboard.update_item { item, options_json? }`: a query tile's SQL is
/// checked again. Undone by its previous options.
pub fn update_item_op(db: Database, sql: SqlGateway) -> Op {
    Op::new(
        "dashboards.write",
        "update_item",
        schema::<UpdateItemInput>(),
        true,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let (db, sql) = (db.clone(), sql.clone());
            Box::pin(async move {
                let input: UpdateItemInput = parse(input)?;
                let id = item_id(&input.item, "/item")?;
                // A query tile's SQL is held to the read contract on every
                // edit too.
                if let Some(query) = input
                    .options_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .and_then(|v| v.get("sql").and_then(Value::as_str).map(str::to_string))
                {
                    sql.check(&query)
                        .await
                        .map_err(|e| tile_error(e, "/options_json"))?;
                }
                let options = input.options_json.clone();
                let before = db
                    .transaction(move |tx| {
                        let before = item_tx(tx, id)?.ok_or_else(|| {
                            oxplow_domain::DomainError::Invalid(format!("no tile `{id}`"))
                        })?;
                        update_item_tx(tx, id, options.as_deref())?;
                        Ok(before.options_json)
                    })
                    .await
                    .map_err(|e| tile_error(e, "/item"))?;
                Ok(HandlerOutput {
                    result: json!({ "item": input.item }),
                    inverse: Some(CommandCall {
                        name: UPDATE_ITEM.into(),
                        input: json!({ "item": input.item, "options_json": before }),
                    }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )
}

/// `dashboard.remove_item { item }`; undone by adding it back where it was.
pub fn remove_item_op() -> Op {
    Op::new(
        "dashboards.write",
        "remove_item",
        schema::<ItemInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ItemInput = parse(input)?;
            let id = item_id(&input.item, "/item")?;
            let item = item_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/item", format!("no tile `{id}`")))?;
            let position = items_tx(ctx.conn, item.dashboard_id)?
                .iter()
                .position(|i| i.id == id)
                .unwrap_or_default() as i64;
            remove_item_tx(ctx.conn, id)?;
            Ok(HandlerOutput {
                result: json!({ "item": input.item, "removed": true }),
                inverse: Some(CommandCall {
                    name: ADD_ITEM.into(),
                    input: json!({
                        "dashboard": item.dashboard_id,
                        "kind": item.kind,
                        "options_json": item.options_json,
                        "position": position,
                    }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// `dashboard.reorder_items { dashboard, order }`; undone by the previous
/// order.
pub fn reorder_items_op() -> Op {
    Op::new(
        "dashboards.write",
        "reorder_items",
        schema::<ReorderItemsInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: ReorderItemsInput = parse(input)?;
            let dashboard = dashboard_id(&input.dashboard, "/dashboard")?;
            exists(ctx.conn, dashboard, "/dashboard")?;
            let current = items_tx(ctx.conn, dashboard)?;
            let mut order = Vec::with_capacity(input.order.len());
            for (i, raw) in input.order.iter().enumerate() {
                let field = format!("/order/{i}");
                let id = item_id(raw, &field)?;
                if !current.iter().any(|t| t.id == id) {
                    return Err(invalid(&field, format!("`{raw}` isn't on `{dashboard}`")));
                }
                order.push(id);
            }
            let previous: Vec<String> = current.iter().map(|t| t.id.to_string()).collect();
            reorder_items_tx(ctx.conn, dashboard, &order)?;
            Ok(HandlerOutput {
                result: json!({ "dashboard": input.dashboard, "order": input.order }),
                inverse: Some(CommandCall {
                    name: REORDER_ITEMS.into(),
                    input: json!({ "dashboard": input.dashboard, "order": previous }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
}

/// The dashboard commands, for the bus.
pub fn ops(db: Database, sql: SqlGateway) -> Vec<Op> {
    vec![
        create_op(),
        rename_op(),
        delete_op(),
        add_item_op(db.clone(), sql.clone()),
        update_item_op(db, sql),
        remove_item_op(),
        reorder_items_op(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{services_with_effort, EffortFixture};
    use oxplow_domain::Actor;

    fn agent(fx: &EffortFixture) -> Actor {
        Actor::Agent {
            thread_id: Some(fx.thread),
            stream_id: None,
        }
    }

    async fn run(
        fx: &EffortFixture,
        actor: &Actor,
        name: &str,
        input: Value,
    ) -> Result<oxplow_domain::CommandOutcome, CommandError> {
        fx.svc.commands.run(actor, name, input, false).await
    }

    async fn tiles(fx: &EffortFixture, d: &str) -> Vec<String> {
        fx.svc
            .dashboard_store
            .get(DashboardId::try_from_str(d).unwrap())
            .await
            .unwrap()
            .unwrap()
            .items
            .into_iter()
            .map(|i| i.options_json.unwrap_or_default())
            .collect()
    }

    async fn dashboard(fx: &EffortFixture) -> String {
        let out = run(fx, &agent(fx), CREATE, json!({ "title": "Coverage" }))
            .await
            .unwrap();
        out.result["id"].as_str().unwrap().to_string()
    }

    /// An agent builds a dashboard; a query tile whose SQL writes (or reads
    /// a physical table) is refused at its input.
    #[tokio::test]
    async fn a_tile_is_checked_before_it_is_added() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
        run(
            &fx,
            &agent(&fx),
            ADD_ITEM,
            json!({ "dashboard": d, "kind": "query", "sql": "SELECT count(*) AS n FROM v_work_item",
                    "display": "number" }),
        )
        .await
        .unwrap();
        for sql in ["DELETE FROM task", "SELECT * FROM task"] {
            let err = run(
                &fx,
                &agent(&fx),
                ADD_ITEM,
                json!({ "dashboard": d, "kind": "query", "sql": sql }),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{sql}: {err:?}"
            );
        }
        assert_eq!(tiles(&fx, &d).await.len(), 1);
    }

    /// Removing a tile undoes by putting it back where it was.
    #[tokio::test]
    async fn removing_a_tile_undoes_to_its_place() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
        let mut ids = Vec::new();
        for text in ["a", "b", "c"] {
            let out = run(
                &fx,
                &Actor::Human,
                ADD_ITEM,
                json!({ "dashboard": d, "kind": "text",
                        "options_json": json!({ "text": text }).to_string() }),
            )
            .await
            .unwrap();
            ids.push(out.result["id"].as_str().unwrap().to_string());
        }
        let before = tiles(&fx, &d).await;
        let out = run(&fx, &Actor::Human, REMOVE_ITEM, json!({ "item": ids[1] }))
            .await
            .unwrap();
        assert_eq!(tiles(&fx, &d).await.len(), 2);
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(tiles(&fx, &d).await, before);
    }

    /// Deleting a dashboard is a person's, asked first.
    #[tokio::test]
    async fn deleting_a_dashboard_is_a_persons_and_asks_first() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
        let err = run(&fx, &agent(&fx), DELETE, json!({ "dashboard": d }))
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Denied { .. }), "{err:?}");
        let err = run(&fx, &Actor::Human, DELETE, json!({ "dashboard": d }))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::NeedsConfirmation { preview } if preview.destructive),
            "{err:?}"
        );
        fx.svc
            .commands
            .run(&Actor::Human, DELETE, json!({ "dashboard": d }), true)
            .await
            .unwrap();
        assert!(fx.svc.dashboard_store.list().await.unwrap().is_empty());
    }
}
