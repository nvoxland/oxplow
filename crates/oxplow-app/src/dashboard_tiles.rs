//! A new dashboard tile, built and checked the same way wherever it comes
//! from — the desktop's pin and picker, or an agent's `add_dashboard_item`
//! (`.context/dashboards.md`, P4.7). A tile is `query` (pinned SQL shown as
//! a lens viz or the metric card), `lens` (a lens by id) or `text`; what
//! identifies it rides in its options JSON (`sql` + `display`, `lensId`,
//! `text`) beside the size and filter overrides every tile has.

use oxplow_db::NewDashboardItem;
use oxplow_domain::DomainError;
use serde_json::{Map, Value};

use crate::sql_gateway::SqlGateway;

/// How a query tile shows its rows: a lens viz, or `metric` — the metric
/// card over a metric's captures (its key in the options' `metric`).
pub const DISPLAYS: &[&str] = &[
    "table", "list", "number", "markdown", "bar", "line", "treemap", "metric",
];

/// What a caller gives for a new tile.
#[derive(Debug, Clone, Default)]
pub struct TileInput {
    pub kind: String,
    /// A `query` tile's SQL (read-only, over published models).
    pub sql: Option<String>,
    /// A `query` tile's display; `table` when absent.
    pub display: Option<String>,
    /// A `lens` tile's lens (`<extension>/<slug>`).
    pub lens_id: Option<String>,
    /// Anything else the tile carries (size, title, a text tile's `text`,
    /// the metric card's `metric` and `viz`, …).
    pub options_json: Option<String>,
}

/// The chart columns a chart display needs (P6.F1): `bar` and `line` an
/// `x` and a `y` (a `series` optional), `treemap` a `label` and a `size`
/// (a `group` optional) — the same roles a lens's `chart:` block names.
fn chart_roles(display: &str) -> Option<(&'static [&'static str], &'static [&'static str])> {
    match display {
        "bar" | "line" => Some((&["x", "y"], &["series"])),
        "treemap" => Some((&["label", "size"], &["group"])),
        _ => None,
    }
}

/// A chart tile's `chart` names its required roles.
fn chart_names_roles<'a>(
    display: &str,
    required: &[&str],
    chart: Option<&'a Value>,
) -> Result<&'a Map<String, Value>, DomainError> {
    let need = || {
        invalid(format!(
            "a `{display}` tile needs `chart: {{ {} }}` in its options",
            required.join(", ")
        ))
    };
    let chart = chart.and_then(Value::as_object).ok_or_else(need)?;
    if required
        .iter()
        .any(|k| !chart.get(*k).is_some_and(Value::is_string))
    {
        return Err(need());
    }
    Ok(chart)
}

/// What a query tile's SQL has yet to be held to: the semantic engine's
/// read contract and, for a chart, the columns its roles name. Both run
/// SQL, which is async.
#[derive(Debug, Clone)]
pub struct QueryCheck {
    query: String,
    /// The chart's roles with the columns they name.
    chart_columns: Vec<(String, String)>,
}

impl QueryCheck {
    pub async fn run(&self, sql: &SqlGateway) -> Result<(), DomainError> {
        sql.check(&self.query).await?;
        if self.chart_columns.is_empty() {
            return Ok(());
        }
        let columns = sql
            .query_sql(&self.query, Vec::new(), Some(1))
            .await?
            .columns;
        for (key, col) in &self.chart_columns {
            if !columns.iter().any(|c| c == col) {
                return Err(invalid(format!(
                    "chart `{key}`: `{col}` isn't a column the query returns ({})",
                    columns.join(", ")
                )));
            }
        }
        Ok(())
    }
}

fn invalid(msg: impl Into<String>) -> DomainError {
    DomainError::Invalid(msg.into())
}

/// Build the tile and check a query tile's SQL through the gateway (the
/// read contract a lens is held to): [`build_tile`] and then its
/// [`QueryCheck`]. The bus runs the check as a command's precheck, before
/// the transaction, and builds again inside it.
pub async fn new_tile(sql: &SqlGateway, input: TileInput) -> Result<NewDashboardItem, DomainError> {
    let (tile, check) = build_tile(input)?;
    if let Some(check) = check {
        check.run(sql).await?;
    }
    Ok(tile)
}

/// Build the tile from what the caller gave, with everything that needs no
/// SQL checked; a query tile also returns its [`QueryCheck`].
pub fn build_tile(input: TileInput) -> Result<(NewDashboardItem, Option<QueryCheck>), DomainError> {
    let mut pending = None;
    let mut opts: Map<String, Value> = match input.options_json.as_deref() {
        Some(raw) if !raw.trim().is_empty() => {
            serde_json::from_str(raw).map_err(|e| invalid(format!("options_json: {e}")))?
        }
        _ => Map::new(),
    };
    match input.kind.as_str() {
        "query" => {
            let query = input
                .sql
                .or_else(|| opts.get("sql").and_then(Value::as_str).map(str::to_string))
                .ok_or_else(|| invalid("a `query` tile needs its `sql`"))?;
            let display = input
                .display
                .or_else(|| {
                    opts.get("display")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "table".into());
            if !DISPLAYS.contains(&display.as_str()) {
                return Err(invalid(format!(
                    "unknown display `{display}` ({})",
                    DISPLAYS.join(" | ")
                )));
            }
            if display == "metric" && !opts.get("metric").is_some_and(Value::is_string) {
                return Err(invalid(
                    "a `metric` display needs the metric key in options `metric`",
                ));
            }
            let mut chart_columns = Vec::new();
            if let Some((required, optional)) = chart_roles(&display) {
                let chart = chart_names_roles(&display, required, opts.get("chart"))?;
                for key in required.iter().chain(optional) {
                    if let Some(col) = chart.get(*key).and_then(Value::as_str) {
                        chart_columns.push((key.to_string(), col.to_string()));
                    }
                }
            }
            pending = Some(QueryCheck {
                query: query.clone(),
                chart_columns,
            });
            opts.insert("sql".into(), Value::String(query));
            opts.insert("display".into(), Value::String(display));
        }
        "lens" => {
            let lens_id = input
                .lens_id
                .or_else(|| {
                    opts.get("lensId")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .ok_or_else(|| invalid("a `lens` tile needs its lens id (`<extension>/<slug>`)"))?;
            opts.insert("lensId".into(), Value::String(lens_id));
        }
        "text" => {}
        other => {
            return Err(invalid(format!(
                "unknown tile kind `{other}` ({})",
                oxplow_db::dashboard_store::TILE_KINDS.join(" | ")
            )))
        }
    }
    Ok((
        NewDashboardItem {
            kind: input.kind,
            options_json: Some(Value::Object(opts).to_string()),
        },
        pending,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_query_tile_is_checked_and_its_sql_kept() {
        let gw = SqlGateway::new(oxplow_db::Database::in_memory());
        let tile = new_tile(
            &gw,
            TileInput {
                kind: "query".into(),
                sql: Some("SELECT count(*) FROM v_work_item".into()),
                display: Some("number".into()),
                options_json: Some(r#"{"size":"wide"}"#.into()),
                ..TileInput::default()
            },
        )
        .await
        .unwrap();
        let opts: Value = serde_json::from_str(tile.options_json.as_deref().unwrap()).unwrap();
        assert_eq!(opts["sql"], "SELECT count(*) FROM v_work_item");
        assert_eq!(opts["display"], "number");
        assert_eq!(opts["size"], "wide");
        for (input, says) in [
            (
                TileInput {
                    kind: "query".into(),
                    sql: Some("SELECT * FROM task".into()),
                    ..TileInput::default()
                },
                "physical table",
            ),
            (
                TileInput {
                    kind: "query".into(),
                    sql: Some("SELECT 1".into()),
                    display: Some("metric".into()),
                    ..TileInput::default()
                },
                "needs the metric key",
            ),
            (
                TileInput {
                    kind: "metric".into(),
                    ..TileInput::default()
                },
                "unknown tile kind",
            ),
            (
                TileInput {
                    kind: "lens".into(),
                    ..TileInput::default()
                },
                "needs its lens id",
            ),
        ] {
            let err = new_tile(&gw, input).await.unwrap_err().to_string();
            assert!(err.contains(says), "{err}");
        }
    }

    /// P6.F1: a pinned chart keeps its chart, which must name the
    /// columns its viz needs, from what the query returns.
    #[tokio::test]
    async fn a_chart_tile_keeps_a_chart_over_its_columns() {
        let gw = SqlGateway::new(oxplow_db::Database::in_memory());
        let sql = "SELECT 'mon' AS day, 3 AS n, 'a' AS s";
        let tile = new_tile(
            &gw,
            TileInput {
                kind: "query".into(),
                sql: Some(sql.into()),
                display: Some("line".into()),
                options_json: Some(r#"{"chart":{"x":"day","y":"n","series":"s"}}"#.into()),
                ..TileInput::default()
            },
        )
        .await
        .unwrap();
        let opts: Value = serde_json::from_str(tile.options_json.as_deref().unwrap()).unwrap();
        assert_eq!(opts["chart"]["series"], "s");
        for (display, chart, says) in [
            ("bar", None, "a `bar` tile needs `chart: { x, y }`"),
            (
                "bar",
                Some(r#"{"x":"day"}"#),
                "a `bar` tile needs `chart: { x, y }`",
            ),
            (
                "treemap",
                Some(r#"{"label":"day"}"#),
                "a `treemap` tile needs `chart: { label, size }`",
            ),
            (
                "line",
                Some(r#"{"x":"day","y":"gone"}"#),
                "`gone` isn't a column",
            ),
        ] {
            let options = chart.map(|c| format!(r#"{{"chart":{c}}}"#));
            let err = new_tile(
                &gw,
                TileInput {
                    kind: "query".into(),
                    sql: Some(sql.into()),
                    display: Some(display.into()),
                    options_json: options,
                    ..TileInput::default()
                },
            )
            .await
            .unwrap_err()
            .to_string();
            assert!(err.contains(says), "{display}: {err}");
        }
    }
}
