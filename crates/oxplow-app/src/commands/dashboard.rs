//! Dashboard commands (P8.A5, `.context/dashboards.md`): a dashboard and
//! its tiles, written through the bus by the desktop, an agent or a lens.
//! Creating, renaming, deleting a dashboard and removing or reordering its
//! tiles are `Tx` over the dashboard store's `_tx` cores, and so are adding
//! a tile and editing one: a query tile's SQL is checked by the semantic
//! engine — the read contract a lens is held to, with `MEASURE()`
//! resolved — which is async, so it runs as the command's `Precheck`
//! before the transaction (`new_tile`), and the handler inside builds the
//! tile again without SQL (`build_tile`). A composite holding either stays
//! one transaction and undoes as one. Dashboards and tiles are named by id
//! (`dsh3`, `dti7`).

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_db::dashboard_store::{
    add_item_tx, create_tx, dashboard_tx, delete_tx, item_tx, items_tx, remove_item_tx, rename_tx,
    reorder_items_tx, update_item_tx,
};
use oxplow_db::Database;
use oxplow_domain::{CommandCall, CommandError, Confirm, DashboardId, DashboardItemId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::util::{invalid, parse, schema};
use super::{Handler, HandlerOutput, Precheck, TxCtx};
use crate::dashboard_tiles::{build_tile, new_tile, TileInput};
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
    /// like `oxplow_query_sql`.
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
    /// Its options JSON, checked as a new tile of its kind is (a query
    /// tile's `sql` and `display`, a lens tile's `lensId`).
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
    .confirm_at_least(Confirm::Destructive)
}

/// `dashboard.add_item { dashboard, kind, sql?, display?, lens_id?,
/// options_json?, position? }`: the tile checked (a query tile's SQL by the
/// semantic engine, before the transaction), then written. Undone by
/// removing it.
pub fn add_item_op(sql: SqlGateway) -> Op {
    let precheck: Arc<Precheck> = Arc::new(move |input: Value| {
        let sql = sql.clone();
        Box::pin(async move {
            let input: AddItemInput = parse(input)?;
            new_tile(&sql, add_tile_input(input))
                .await
                .map(|_| ())
                .map_err(|e| tile_error(e, ""))
        })
    });
    Op::new(
        "dashboards.write",
        "add_item",
        schema::<AddItemInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: AddItemInput = parse(input)?;
            let dashboard = dashboard_id(&input.dashboard, "/dashboard")?;
            let position = input.position;
            // Its query was checked before the transaction.
            let (tile, _) = build_tile(add_tile_input(input)).map_err(|e| tile_error(e, ""))?;
            exists(ctx.conn, dashboard, "/dashboard")?;
            let id = add_item_tx(ctx.conn, dashboard, &tile, position)?;
            Ok(HandlerOutput {
                result: json!({ "id": id }),
                inverse: Some(CommandCall {
                    name: REMOVE_ITEM.into(),
                    input: json!({ "item": id }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
    .with_precheck(precheck)
}

fn add_tile_input(input: AddItemInput) -> TileInput {
    TileInput {
        kind: input.kind,
        sql: input.sql,
        display: input.display,
        lens_id: input.lens_id,
        options_json: input.options_json,
    }
}

/// `dashboard.update_item { item, options_json? }`: an edited tile is
/// checked as a new one of its kind is — its options parse, a query tile
/// keeps its SQL (held to the read contract, before the transaction) and a
/// display it can show. Undone by its previous options.
pub fn update_item_op(db: Database, sql: SqlGateway) -> Op {
    let precheck: Arc<Precheck> = Arc::new(move |input: Value| {
        let (db, sql) = (db.clone(), sql.clone());
        Box::pin(async move {
            let input: UpdateItemInput = parse(input)?;
            let id = item_id(&input.item, "/item")?;
            let kind = db
                .read(move |tx| {
                    item_tx(tx, id)?.map(|t| t.kind).ok_or_else(|| {
                        oxplow_domain::DomainError::Invalid(format!("no tile `{id}`"))
                    })
                })
                .await
                .map_err(|e| tile_error(e, "/item"))?;
            new_tile(&sql, update_tile_input(kind, input.options_json))
                .await
                .map(|_| ())
                .map_err(|e| tile_error(e, "/options_json"))
        })
    });
    Op::new(
        "dashboards.write",
        "update_item",
        schema::<UpdateItemInput>(),
        true,
        Handler::Tx(Arc::new(|ctx: &TxCtx<'_>, input| {
            let input: UpdateItemInput = parse(input)?;
            let id = item_id(&input.item, "/item")?;
            let before = item_tx(ctx.conn, id)?
                .ok_or_else(|| invalid("/item", format!("no tile `{id}`")))?;
            let (tile, _) = build_tile(update_tile_input(before.kind, input.options_json))
                .map_err(|e| tile_error(e, "/options_json"))?;
            update_item_tx(ctx.conn, id, tile.options_json.as_deref())?;
            Ok(HandlerOutput {
                result: json!({ "item": input.item }),
                inverse: Some(CommandCall {
                    name: UPDATE_ITEM.into(),
                    input: json!({ "item": input.item, "options_json": before.options_json }),
                }),
                ..HandlerOutput::default()
            })
        })),
    )
    .with_precheck(precheck)
}

fn update_tile_input(kind: String, options_json: Option<String>) -> TileInput {
    TileInput {
        kind,
        options_json,
        ..TileInput::default()
    }
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
        add_item_op(sql.clone()),
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
            session_id: None,
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

    /// Editing a tile is held to what adding one is: options that aren't
    /// JSON, a query tile left without its SQL, or an unknown display are
    /// refused, and nothing changes.
    #[tokio::test]
    async fn an_edited_tile_is_checked_like_a_new_one() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
        let out = run(
            &fx,
            &Actor::Human,
            ADD_ITEM,
            json!({ "dashboard": d, "kind": "query", "sql": "SELECT count(*) AS n FROM v_work_item",
                    "display": "number" }),
        )
        .await
        .unwrap();
        let item = out.result["id"].as_str().unwrap().to_string();
        let before = tiles(&fx, &d).await;
        for options in [
            "{not json".to_string(),
            json!({ "display": "number" }).to_string(),
            json!({ "sql": "SELECT 1 AS n", "display": "pie" }).to_string(),
        ] {
            let err = run(
                &fx,
                &Actor::Human,
                UPDATE_ITEM,
                json!({ "item": item, "options_json": options }),
            )
            .await
            .unwrap_err();
            assert!(
                matches!(err, CommandError::Invalid { .. }),
                "{options}: {err:?}"
            );
        }
        assert_eq!(tiles(&fx, &d).await, before);
        run(
            &fx,
            &Actor::Human,
            UPDATE_ITEM,
            json!({ "item": item, "options_json": json!({ "sql": "SELECT 2 AS n", "display": "number", "title": "Two" }).to_string() }),
        )
        .await
        .unwrap();
        assert!(tiles(&fx, &d).await[0].contains("SELECT 2 AS n"));
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

    /// A composite holding tile writes stays one transaction and undoes as
    /// one; a refused query in it writes nothing.
    #[tokio::test]
    async fn tile_writes_compose_into_one_undoable_transaction() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
        let first = run(
            &fx,
            &Actor::Human,
            ADD_ITEM,
            json!({ "dashboard": d, "kind": "text",
                    "options_json": json!({ "text": "a" }).to_string() }),
        )
        .await
        .unwrap();
        let item = first.result["id"].as_str().unwrap().to_string();
        let before = tiles(&fx, &d).await;
        let text = |t: &str| json!({ "text": t }).to_string();
        let calls = |sql: &str| {
            json!({ "calls": [
                { "name": ADD_ITEM, "input": { "dashboard": d, "kind": "text", "options_json": text("b") } },
                { "name": UPDATE_ITEM, "input": { "item": item, "options_json": text("a2") } },
                { "name": ADD_ITEM, "input": { "dashboard": d, "kind": "query", "sql": sql, "display": "number" } },
            ] })
        };
        let err = run(
            &fx,
            &Actor::Human,
            "oxplow.command.sequence",
            calls("SELECT * FROM task"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
        assert_eq!(tiles(&fx, &d).await, before);
        let out = run(
            &fx,
            &Actor::Human,
            "oxplow.command.sequence",
            calls("SELECT count(*) AS n FROM v_work_item"),
        )
        .await
        .unwrap();
        assert_eq!(tiles(&fx, &d).await.len(), 3);
        fx.svc
            .commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .unwrap();
        assert_eq!(tiles(&fx, &d).await, before);
    }

    /// A refused tile is audited invalid at its field.
    #[tokio::test]
    async fn a_refused_tile_names_its_field() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
        let field = |e: CommandError| match e {
            CommandError::Invalid { field, .. } => field,
            other => panic!("{other:?}"),
        };
        let err = run(
            &fx,
            &Actor::Human,
            ADD_ITEM,
            json!({ "dashboard": d, "kind": "query", "sql": "SELECT * FROM task" }),
        )
        .await
        .unwrap_err();
        assert_eq!(field(err).unwrap_or_default(), "");
        let err = run(
            &fx,
            &Actor::Human,
            ADD_ITEM,
            json!({ "dashboard": "dsh99999", "kind": "text" }),
        )
        .await
        .unwrap_err();
        assert_eq!(field(err).as_deref(), Some("/dashboard"));
        let err = run(
            &fx,
            &Actor::Human,
            UPDATE_ITEM,
            json!({ "item": "dti99999" }),
        )
        .await
        .unwrap_err();
        assert_eq!(field(err).as_deref(), Some("/item"));
    }

    /// Deleting a dashboard asks a person first; an agent's delete runs.
    #[tokio::test]
    async fn deleting_a_dashboard_asks_a_person_first() {
        let fx = services_with_effort().await;
        let d = dashboard(&fx).await;
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

        let d = dashboard(&fx).await;
        run(&fx, &agent(&fx), DELETE, json!({ "dashboard": d }))
            .await
            .unwrap();
        assert!(fx.svc.dashboard_store.list().await.unwrap().is_empty());
    }
}
