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

/// The sizes a tile can take (its options' `size`).
pub const SIZES: &[&str] = &["small", "wide", "tall", "full"];

/// How the metric card draws (its options' `viz`).
pub const METRIC_VIZ: &[&str] = &["line", "number"];

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
    /// The tile's title override. Beats the same key in `options_json`.
    pub title: Option<String>,
    /// One of [`SIZES`].
    pub size: Option<String>,
    /// A `text` tile's heading (plain text).
    pub text: Option<String>,
    /// The metric card's metric key.
    pub metric: Option<String>,
    /// One of [`METRIC_VIZ`].
    pub viz: Option<String>,
    /// A chart tile's columns (a JSON object).
    pub chart: Option<Value>,
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

/// One row per capture of metric `key`, newest first: what a metric card
/// reads (the UI's `metricSeriesSql`, ungrouped and unranged).
fn metric_series_sql(key: &str) -> String {
    let measure = format!("MEASURE('{}')", key.replace('\'', "''"));
    format!(
        "SELECT g.capture_id, g.bucket AS captured_at, {measure} AS value, NULL AS \"group\",\n       c.branch, c.provenance, c.closest_vcs_rev AS vcs_rev, c.source\nFROM metric_grid('capture') g LEFT JOIN v_capture c ON c.id = g.capture_id\nWHERE {measure} IS NOT NULL\nORDER BY g.bucket DESC"
    )
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
    for (key, flat) in [
        ("title", &input.title),
        ("size", &input.size),
        ("text", &input.text),
        ("metric", &input.metric),
        ("viz", &input.viz),
    ] {
        if let Some(v) = flat {
            opts.insert(key.into(), Value::String(v.clone()));
        }
    }
    if let Some(chart) = &input.chart {
        opts.insert("chart".into(), chart.clone());
    }
    for (key, allowed) in [("size", SIZES), ("viz", METRIC_VIZ)] {
        if let Some(v) = opts.get(key) {
            if !v.as_str().is_some_and(|s| allowed.contains(&s)) {
                let shown = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                return Err(invalid(format!(
                    "unknown {key} `{shown}` ({})",
                    allowed.join(" | ")
                )));
            }
        }
    }
    if opts.get("chart").is_some_and(|c| !c.is_object()) {
        return Err(invalid("`chart` must be an object of column names"));
    }
    match input.kind.as_str() {
        "query" => {
            let given_sql = input
                .sql
                .or_else(|| opts.get("sql").and_then(Value::as_str).map(str::to_string));
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
            // A metric card's SQL is its metric's captures: the key is
            // enough, and the tile pins what the key means today.
            let query = match given_sql {
                Some(query) => query,
                None if display == "metric" => metric_series_sql(
                    opts.get("metric")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                ),
                None => return Err(invalid("a `query` tile needs its `sql`")),
            };
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

    /// A metric card needs the metric key and no SQL: the tile pins the
    /// metric's captures.
    #[test]
    fn a_metric_tile_pins_its_metrics_captures_from_the_key_alone() {
        let (tile, check) = build_tile(TileInput {
            kind: "query".into(),
            display: Some("metric".into()),
            metric: Some("loc".into()),
            viz: Some("number".into()),
            ..TileInput::default()
        })
        .unwrap();
        assert!(check.is_some());
        let opts: Value = serde_json::from_str(tile.options_json.as_deref().unwrap()).unwrap();
        assert!(opts["sql"].as_str().unwrap().contains("MEASURE('loc')"));
        assert_eq!(opts["metric"], "loc");
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

    #[tokio::test]
    async fn a_tile_takes_flat_fields_over_its_options() {
        let gw = SqlGateway::new(oxplow_db::Database::in_memory());
        let tile = new_tile(
            &gw,
            TileInput {
                kind: "text".into(),
                title: Some("Flat".into()),
                size: Some("full".into()),
                text: Some("hi".into()),
                options_json: Some(r#"{"title":"Blob","size":"wide","range":"7d"}"#.into()),
                ..TileInput::default()
            },
        )
        .await
        .unwrap();
        let opts: Value = serde_json::from_str(tile.options_json.as_deref().unwrap()).unwrap();
        assert_eq!(opts["title"], "Flat");
        assert_eq!(opts["size"], "full");
        assert_eq!(opts["text"], "hi");
        assert_eq!(opts["range"], "7d");
    }

    #[tokio::test]
    async fn size_viz_and_chart_are_checked() {
        let gw = SqlGateway::new(oxplow_db::Database::in_memory());
        for (input, says) in [
            (
                TileInput {
                    kind: "text".into(),
                    size: Some("huge".into()),
                    ..TileInput::default()
                },
                "unknown size `huge` (small | wide | tall | full)",
            ),
            (
                TileInput {
                    kind: "text".into(),
                    options_json: Some(r#"{"viz":"pie"}"#.into()),
                    ..TileInput::default()
                },
                "unknown viz `pie` (line | number)",
            ),
            (
                TileInput {
                    kind: "text".into(),
                    chart: Some(Value::String("x".into())),
                    ..TileInput::default()
                },
                "`chart` must be an object",
            ),
        ] {
            let err = new_tile(&gw, input).await.unwrap_err().to_string();
            assert!(err.contains(says), "{err}");
        }
        let tile = new_tile(
            &gw,
            TileInput {
                kind: "query".into(),
                sql: Some("SELECT 'a' AS day, 3 AS n".into()),
                display: Some("bar".into()),
                chart: Some(serde_json::json!({"x":"day","y":"n"})),
                viz: Some("number".into()),
                ..TileInput::default()
            },
        )
        .await
        .unwrap();
        assert!(tile.options_json.unwrap().contains(r#""viz":"number""#));
    }
}
