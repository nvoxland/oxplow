//! Map OTLP metric exports → token facts (epic tsk22).
//!
//! Agent CLIs (Claude Code, Codex) export token usage as OpenTelemetry metrics
//! to oxplow's control-plane OTLP receiver. This module decodes the protobuf
//! body and projects the token data points into the intermediate [`TokenFact`]
//! grain the ingest service writes onto the `oxplow.tokens` measure — the
//! successor to the transcript-parse producer (`token_usage.rs`), which
//! overcounted because Claude repeats a message's cumulative `usage` on every
//! content-block line.
//!
//! Pure + no IO → fully unit-testable. Two agents, two shapes:
//! - **Claude** — the `claude_code.token.usage` **metric** counter (tsk23).
//! - **Codex** — its `response.completed` **log event** (tsk27), the confirmed
//!   live source; Codex points its single OTLP endpoint at us and sends token
//!   counts as logs. `input_token_count` is the full context, so new input =
//!   `input − cached`; reasoning folds into output. (A `codex.turn.token_usage`
//!   metric mapper also exists but is speculative — unemitted by Codex 0.142.0.)
//!
//! Both keep the `input`/`output` token kinds; cache is dropped.
//! [`summarize_metrics_request`] is the opt-in wire-format diagnostic (tsk25),
//! which also decodes logs.

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::{metric, number_data_point, Metric};
use prost::Message;

/// Claude Code's per-model token counter (delta temporality). Its `type`
/// attribute carries the token kind; `model` the model id.
const CLAUDE_TOKEN_METRIC: &str = "claude_code.token.usage";

/// Codex's per-turn token histogram — SPECULATIVE. Codex 0.142.0 does NOT emit
/// this; its token counts ride the `response.completed` LOG event instead (see
/// [`otlp_logs_to_token_facts`]). Kept as a defensive path in case a future
/// Codex version adds the metric. `token_type` carries the kind; value is the
/// histogram data point's `sum`.
const CODEX_TOKEN_METRIC: &str = "codex.turn.token_usage";

/// Codex emits token counts on its `codex.sse_event` log record whose
/// `event.kind` attribute is `response.completed` (tsk27).
const CODEX_TOKEN_EVENT_KIND: &str = "response.completed";

/// The token kinds oxplow tracks. Cache tokens (Claude `cacheRead`/
/// `cacheCreation`, Codex `cached_input`) and the Codex `total` rollup are
/// dropped; Codex `reasoning_output` folds into `output` (matching Claude,
/// whose `output` already includes thinking). Cache kinds can be added later
/// without touching the `agent.tokens.*` specs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Input,
    Output,
}

impl TokenKind {
    /// The conformed `oxplow.token_kind` dimension value.
    pub fn as_str(self) -> &'static str {
        match self {
            TokenKind::Input => "input",
            TokenKind::Output => "output",
        }
    }
}

/// One token measurement projected out of an OTLP export: a `value`-token count
/// for a `(model, kind)` pair, ready to become a `NewFact` on `oxplow.tokens`.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenFact {
    pub model: String,
    pub kind: TokenKind,
    pub value: i64,
}

/// Decode an OTLP/HTTP protobuf metrics export body.
pub fn decode_metrics_request(
    body: &[u8],
) -> Result<ExportMetricsServiceRequest, prost::DecodeError> {
    ExportMetricsServiceRequest::decode(body)
}

/// Decode an OTLP/HTTP protobuf logs export body. Codex points its single OTLP
/// endpoint at us and sends its log events (which carry its token counts) here.
pub fn decode_logs_request(body: &[u8]) -> Result<ExportLogsServiceRequest, prost::DecodeError> {
    ExportLogsServiceRequest::decode(body)
}

/// Project the token data points from a decoded OTLP metrics export into
/// [`TokenFact`]s. Recognizes Claude Code's `claude_code.token.usage` counter
/// and Codex's `codex.turn.token_usage` histogram; other metrics and non-input/
/// output token kinds are ignored, zero-valued points skipped. `model` is read
/// from the data point, falling back to the resource attributes.
pub fn otlp_metrics_to_token_facts(req: &ExportMetricsServiceRequest) -> Vec<TokenFact> {
    let mut out = Vec::new();
    for rm in &req.resource_metrics {
        let resource_attrs = rm
            .resource
            .as_ref()
            .map(|r| r.attributes.as_slice())
            .unwrap_or(&[]);
        for sm in &rm.scope_metrics {
            for m in &sm.metrics {
                match m.name.as_str() {
                    CLAUDE_TOKEN_METRIC => collect_claude(m, resource_attrs, &mut out),
                    CODEX_TOKEN_METRIC => collect_codex(m, resource_attrs, &mut out),
                    _ => {}
                }
            }
        }
    }
    out
}

/// Claude: a counter (OTLP Sum; Gauge tolerated) with a `type` attribute per
/// number data point.
fn collect_claude(m: &Metric, resource_attrs: &[KeyValue], out: &mut Vec<TokenFact>) {
    let points = match &m.data {
        Some(metric::Data::Sum(sum)) => &sum.data_points,
        Some(metric::Data::Gauge(gauge)) => &gauge.data_points,
        _ => return,
    };
    for dp in points {
        let Some(kind) = claude_token_kind(&dp.attributes) else {
            continue;
        };
        let value = number_value(&dp.value);
        if value == 0 {
            continue;
        }
        out.push(TokenFact {
            model: model_attr(&dp.attributes, resource_attrs),
            kind,
            value,
        });
    }
}

/// Codex: a per-turn histogram with a `token_type` attribute; the token count
/// is the data point's `sum`.
fn collect_codex(m: &Metric, resource_attrs: &[KeyValue], out: &mut Vec<TokenFact>) {
    let Some(metric::Data::Histogram(hist)) = &m.data else {
        return;
    };
    for dp in &hist.data_points {
        let Some(kind) = codex_token_kind(&dp.attributes) else {
            continue;
        };
        let value = dp.sum.unwrap_or(0.0) as i64;
        if value == 0 {
            continue;
        }
        out.push(TokenFact {
            model: model_attr(&dp.attributes, resource_attrs),
            kind,
            value,
        });
    }
}

/// Project token facts from a decoded OTLP **logs** export (tsk27) — Codex's
/// real token source. Each `response.completed` log record carries per-request
/// counts (`input_token_count` is the FULL context, so new input =
/// `input_token_count − cached_token_count`; reasoning folds into output to
/// match Claude). `model` reads the record, falling back to resource attributes.
pub fn otlp_logs_to_token_facts(req: &ExportLogsServiceRequest) -> Vec<TokenFact> {
    let mut out = Vec::new();
    for rl in &req.resource_logs {
        let resource_attrs = rl
            .resource
            .as_ref()
            .map(|r| r.attributes.as_slice())
            .unwrap_or(&[]);
        for sl in &rl.scope_logs {
            for lr in &sl.log_records {
                if string_attr(&lr.attributes, "event.kind").as_deref()
                    != Some(CODEX_TOKEN_EVENT_KIND)
                {
                    continue;
                }
                let a = &lr.attributes;
                let input = int_attr(a, "input_token_count").unwrap_or(0);
                let cached = int_attr(a, "cached_token_count").unwrap_or(0);
                let output = int_attr(a, "output_token_count").unwrap_or(0);
                let reasoning = int_attr(a, "reasoning_token_count").unwrap_or(0);
                let model = model_attr(a, resource_attrs);
                // new (uncached) input this request; reasoning folded into output.
                let new_input = (input - cached).max(0);
                let out_total = output + reasoning;
                if new_input > 0 {
                    out.push(TokenFact {
                        model: model.clone(),
                        kind: TokenKind::Input,
                        value: new_input,
                    });
                }
                if out_total > 0 {
                    out.push(TokenFact {
                        model,
                        kind: TokenKind::Output,
                        value: out_total,
                    });
                }
            }
        }
    }
    out
}

/// Read a string-valued OTLP attribute by key.
fn string_attr(attrs: &[KeyValue], key: &str) -> Option<String> {
    attrs.iter().find(|kv| kv.key == key).and_then(|kv| {
        match kv.value.as_ref()?.value.as_ref()? {
            any_value::Value::StringValue(s) => Some(s.clone()),
            _ => None,
        }
    })
}

/// Read an integer-valued OTLP attribute (int, double, or numeric string).
fn int_attr(attrs: &[KeyValue], key: &str) -> Option<i64> {
    match attrs
        .iter()
        .find(|kv| kv.key == key)?
        .value
        .as_ref()?
        .value
        .as_ref()?
    {
        any_value::Value::IntValue(i) => Some(*i),
        any_value::Value::DoubleValue(d) => Some(*d as i64),
        any_value::Value::StringValue(s) => s.parse().ok(),
        _ => None,
    }
}

/// The model id: from the data-point attributes, else the resource attributes,
/// else `"unknown"`.
fn model_attr(dp_attrs: &[KeyValue], resource_attrs: &[KeyValue]) -> String {
    string_attr(dp_attrs, "model")
        .or_else(|| string_attr(resource_attrs, "model"))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Claude's `type` attribute → a tracked kind (drops cacheRead/cacheCreation).
fn claude_token_kind(attrs: &[KeyValue]) -> Option<TokenKind> {
    match string_attr(attrs, "type")?.as_str() {
        "input" => Some(TokenKind::Input),
        "output" => Some(TokenKind::Output),
        _ => None,
    }
}

/// Codex's `token_type` attribute → a tracked kind. `reasoning_output` folds
/// into `output`; `cached_input` and the `total` rollup are dropped (dropping
/// `total` is what prevents double-counting).
fn codex_token_kind(attrs: &[KeyValue]) -> Option<TokenKind> {
    match string_attr(attrs, "token_type")?.as_str() {
        "input" => Some(TokenKind::Input),
        "output" | "reasoning_output" => Some(TokenKind::Output),
        _ => None,
    }
}

/// The scalar value of a number data point (counter/gauge), truncated to i64.
fn number_value(v: &Option<number_data_point::Value>) -> i64 {
    match v {
        Some(number_data_point::Value::AsInt(i)) => *i,
        Some(number_data_point::Value::AsDouble(d)) => *d as i64,
        None => 0,
    }
}

/// Diagnostic (tsk25): a human-readable dump of an OTLP metrics export — the
/// resource attributes plus every metric's name, data type, and each data
/// point's attributes + value/sum. Used behind the `OXPLOW_OTLP_DEBUG` flag to
/// discover an agent's real wire format (e.g. Codex's token metric
/// name/attributes) from a live run, without guessing.
pub fn summarize_metrics_request(body: &[u8]) -> String {
    let req = match decode_metrics_request(body) {
        Ok(r) => r,
        // Codex points its single OTLP endpoint here and sends its LOG events
        // too (its token counts may ride the `codex.response.completed` event),
        // so on a metrics-decode miss fall back to dumping the logs payload.
        Err(metrics_err) => return summarize_logs_request(body, &metrics_err),
    };
    let mut lines = Vec::new();
    for rm in &req.resource_metrics {
        if let Some(res) = &rm.resource {
            let a = fmt_attrs(&res.attributes);
            if !a.is_empty() {
                lines.push(format!("resource: {a}"));
            }
        }
        for sm in &rm.scope_metrics {
            for m in &sm.metrics {
                match &m.data {
                    Some(metric::Data::Sum(x)) => {
                        summarize_number_points(&mut lines, &m.name, "sum", &x.data_points)
                    }
                    Some(metric::Data::Gauge(x)) => {
                        summarize_number_points(&mut lines, &m.name, "gauge", &x.data_points)
                    }
                    Some(metric::Data::Histogram(x)) => {
                        for dp in &x.data_points {
                            lines.push(format!(
                                "metric {} [histogram] {{{}}} sum={:?} count={}",
                                m.name,
                                fmt_attrs(&dp.attributes),
                                dp.sum,
                                dp.count
                            ));
                        }
                    }
                    Some(_) => lines.push(format!("metric {} [other data type]", m.name)),
                    None => lines.push(format!("metric {} [no data]", m.name)),
                }
            }
        }
    }
    if lines.is_empty() {
        "<no metrics in export>".to_string()
    } else {
        lines.join("\n")
    }
}

/// Fallback dump when a body isn't OTLP MetricsData: try OTLP LogsData and list
/// each log record's event name + attributes + body (tsk26). Codex's token
/// counts are suspected to ride the `codex.response.completed` log event.
fn summarize_logs_request(body: &[u8], metrics_err: &prost::DecodeError) -> String {
    use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
    let req = match ExportLogsServiceRequest::decode(body) {
        Ok(r) => r,
        Err(logs_err) => {
            return format!(
                "<undecodable as metrics ({metrics_err}) or logs ({logs_err}); {} bytes>",
                body.len()
            )
        }
    };
    let mut lines = vec!["[OTLP LOGS payload — not metrics]".to_string()];
    for rl in &req.resource_logs {
        if let Some(res) = &rl.resource {
            let a = fmt_attrs(&res.attributes);
            if !a.is_empty() {
                lines.push(format!("resource: {a}"));
            }
        }
        for sl in &rl.scope_logs {
            for lr in &sl.log_records {
                let name = if !lr.event_name.is_empty() {
                    lr.event_name.as_str()
                } else if !lr.severity_text.is_empty() {
                    lr.severity_text.as_str()
                } else {
                    "<log>"
                };
                let body_str = lr
                    .body
                    .as_ref()
                    .and_then(|b| b.value.as_ref())
                    .map(fmt_attr_value)
                    .unwrap_or_default();
                lines.push(format!(
                    "log {name} {{{}}} body={body_str}",
                    fmt_attrs(&lr.attributes)
                ));
            }
        }
    }
    if lines.len() == 1 {
        lines.push("<no log records>".to_string());
    }
    lines.join("\n")
}

fn summarize_number_points(
    lines: &mut Vec<String>,
    name: &str,
    kind: &str,
    pts: &[opentelemetry_proto::tonic::metrics::v1::NumberDataPoint],
) {
    for dp in pts {
        lines.push(format!(
            "metric {name} [{kind}] {{{}}} value={}",
            fmt_attrs(&dp.attributes),
            number_value(&dp.value)
        ));
    }
}

/// Render OTLP attributes as `key=value, key2=value2` (scalar values only).
fn fmt_attrs(attrs: &[KeyValue]) -> String {
    attrs
        .iter()
        .map(|kv| {
            let val = kv
                .value
                .as_ref()
                .and_then(|a| a.value.as_ref())
                .map(fmt_attr_value)
                .unwrap_or_default();
            format!("{}={}", kv.key, val)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn fmt_attr_value(v: &any_value::Value) -> String {
    match v {
        any_value::Value::StringValue(s) => s.clone(),
        any_value::Value::IntValue(i) => i.to_string(),
        any_value::Value::DoubleValue(d) => d.to_string(),
        any_value::Value::BoolValue(b) => b.to_string(),
        _ => "<complex>".to_string(),
    }
}

/// Test-only: build an encoded (protobuf) Claude-shaped OTLP metrics export
/// body with one `input` + one `output` `claude_code.token.usage` data point
/// for `model`. Shared with the ingest-service tests in `token_usage.rs`.
#[cfg(test)]
pub(crate) fn encoded_claude_export(model: &str, input: i64, output: i64) -> Vec<u8> {
    use opentelemetry_proto::tonic::common::v1::AnyValue;
    use opentelemetry_proto::tonic::metrics::v1::{
        Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
    };
    let kv = |k: &str, v: &str| KeyValue {
        key: k.into(),
        value: Some(AnyValue {
            value: Some(any_value::Value::StringValue(v.into())),
        }),
        ..Default::default()
    };
    let point = |ty: &str, val: i64| NumberDataPoint {
        attributes: vec![kv("type", ty), kv("model", model)],
        value: Some(number_data_point::Value::AsInt(val)),
        ..Default::default()
    };
    ExportMetricsServiceRequest {
        resource_metrics: vec![ResourceMetrics {
            scope_metrics: vec![ScopeMetrics {
                metrics: vec![Metric {
                    name: CLAUDE_TOKEN_METRIC.into(),
                    data: Some(metric::Data::Sum(Sum {
                        data_points: vec![point("input", input), point("output", output)],
                        ..Default::default()
                    })),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }],
    }
    .encode_to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry_proto::tonic::common::v1::AnyValue;
    use opentelemetry_proto::tonic::metrics::v1::{
        Histogram, HistogramDataPoint, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
    };

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(v.into())),
            }),
            ..Default::default()
        }
    }

    fn point(token_type: &str, model: &str, value: i64) -> NumberDataPoint {
        NumberDataPoint {
            attributes: vec![kv("type", token_type), kv("model", model)],
            value: Some(number_data_point::Value::AsInt(value)),
            ..Default::default()
        }
    }

    /// A Claude-shaped export: one `claude_code.token.usage` Sum with input,
    /// output, and a cacheRead point (which must be dropped).
    fn claude_request() -> ExportMetricsServiceRequest {
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: CLAUDE_TOKEN_METRIC.into(),
                        data: Some(metric::Data::Sum(Sum {
                            data_points: vec![
                                point("input", "claude-opus-4-8", 100),
                                point("output", "claude-opus-4-8", 20),
                                point("cacheRead", "claude-opus-4-8", 5000),
                            ],
                            ..Default::default()
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    #[test]
    fn claude_counter_maps_to_input_output_facts_dropping_cache() {
        let facts = otlp_metrics_to_token_facts(&claude_request());
        assert_eq!(facts.len(), 2, "cacheRead dropped, input+output kept");
        assert!(facts.contains(&TokenFact {
            model: "claude-opus-4-8".into(),
            kind: TokenKind::Input,
            value: 100,
        }));
        assert!(facts.contains(&TokenFact {
            model: "claude-opus-4-8".into(),
            kind: TokenKind::Output,
            value: 20,
        }));
    }

    #[test]
    fn codex_histogram_maps_token_types_folding_reasoning_into_output() {
        // tsk24: Codex emits a per-turn histogram with a `token_type` attribute;
        // the count is the data point's `sum`. reasoning_output folds into
        // output; cached_input + the `total` rollup are dropped.
        let hp = |token_type: &str, sum: f64| HistogramDataPoint {
            attributes: vec![kv("token_type", token_type), kv("model", "gpt-5-codex")],
            sum: Some(sum),
            ..Default::default()
        };
        let req = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: CODEX_TOKEN_METRIC.into(),
                        data: Some(metric::Data::Histogram(Histogram {
                            data_points: vec![
                                hp("input", 100.0),
                                hp("output", 20.0),
                                hp("reasoning_output", 30.0),
                                hp("cached_input", 5000.0),
                                hp("total", 5150.0),
                            ],
                            ..Default::default()
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let facts = otlp_metrics_to_token_facts(&req);
        let input: i64 = facts
            .iter()
            .filter(|f| f.kind == TokenKind::Input)
            .map(|f| f.value)
            .sum();
        let output: i64 = facts
            .iter()
            .filter(|f| f.kind == TokenKind::Output)
            .map(|f| f.value)
            .sum();
        assert_eq!(input, 100, "input kept");
        assert_eq!(output, 50, "output(20) + reasoning_output(30) folded");
        assert!(
            facts.iter().all(|f| f.model == "gpt-5-codex"),
            "model read from the data point"
        );
        // cached_input + total contributed nothing.
        assert_eq!(facts.iter().map(|f| f.value).sum::<i64>(), 150);
    }

    fn kv_int(k: &str, v: i64) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue {
                value: Some(any_value::Value::IntValue(v)),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn codex_response_completed_log_maps_new_input_and_folded_output() {
        // tsk27: real Codex token source. input=full context, so new input =
        // input − cached; reasoning folds into output. Mix int + string attrs
        // to exercise int_attr's coercion.
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        let rec = LogRecord {
            attributes: vec![
                kv("event.name", "codex.sse_event"),
                kv("event.kind", "response.completed"),
                kv_int("input_token_count", 113690),
                kv_int("cached_token_count", 2432),
                kv("output_token_count", "254"),
                kv("reasoning_token_count", "42"),
                kv("model", "gpt-5.5"),
            ],
            ..Default::default()
        };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![rec],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let facts = otlp_logs_to_token_facts(&req);
        let input: i64 = facts
            .iter()
            .filter(|f| f.kind == TokenKind::Input)
            .map(|f| f.value)
            .sum();
        let output: i64 = facts
            .iter()
            .filter(|f| f.kind == TokenKind::Output)
            .map(|f| f.value)
            .sum();
        assert_eq!(input, 111258, "113690 input − 2432 cached");
        assert_eq!(output, 296, "254 output + 42 reasoning");
        assert!(facts.iter().all(|f| f.model == "gpt-5.5"));
    }

    #[test]
    fn non_response_completed_logs_produce_no_token_facts() {
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        // A codex.api_request event (no response.completed kind) → nothing.
        let rec = LogRecord {
            attributes: vec![
                kv("event.name", "codex.api_request"),
                kv("duration_ms", "427"),
            ],
            ..Default::default()
        };
        let req = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![rec],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        assert!(otlp_logs_to_token_facts(&req).is_empty());
    }

    #[test]
    fn model_falls_back_to_resource_attribute() {
        use opentelemetry_proto::tonic::resource::v1::Resource;
        // A data point with no `model` attribute; the model rides the resource.
        let dp = NumberDataPoint {
            attributes: vec![kv("type", "input")],
            value: Some(number_data_point::Value::AsInt(42)),
            ..Default::default()
        };
        let req = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![kv("model", "claude-sonnet-5")],
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: CLAUDE_TOKEN_METRIC.into(),
                        data: Some(metric::Data::Sum(Sum {
                            data_points: vec![dp],
                            ..Default::default()
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        let facts = otlp_metrics_to_token_facts(&req);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].model, "claude-sonnet-5");
    }

    #[test]
    fn ignores_unrelated_metrics_and_zero_points() {
        let req = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![
                        Metric {
                            name: "claude_code.cost.usage".into(),
                            data: Some(metric::Data::Sum(Sum {
                                data_points: vec![point("input", "m", 999)],
                                ..Default::default()
                            })),
                            ..Default::default()
                        },
                        Metric {
                            name: CLAUDE_TOKEN_METRIC.into(),
                            data: Some(metric::Data::Sum(Sum {
                                data_points: vec![point("output", "m", 0)],
                                ..Default::default()
                            })),
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        };
        assert!(
            otlp_metrics_to_token_facts(&req).is_empty(),
            "wrong metric name + zero-valued point both ignored"
        );
    }

    #[test]
    fn summary_dumps_metric_names_and_attributes() {
        let body = encoded_claude_export("claude-opus-4-8", 100, 20);
        let s = summarize_metrics_request(&body);
        assert!(
            s.contains("claude_code.token.usage"),
            "names the metric: {s}"
        );
        assert!(s.contains("type=input"));
        assert!(s.contains("model=claude-opus-4-8"));
        assert!(s.contains("value=100"));
        // A garbage body degrades gracefully rather than panicking.
        assert!(summarize_metrics_request(b"not protobuf").contains("undecodable"));
    }

    #[test]
    fn logs_payload_falls_back_to_a_logs_dump() {
        // tsk26: a body that isn't MetricsData is retried as LogsData so the
        // dump reveals Codex's log events (where its token counts may live).
        use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
        use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
        let body = ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: vec![LogRecord {
                        event_name: "codex.response.completed".into(),
                        attributes: vec![kv("input_tokens", "100"), kv("output_tokens", "20")],
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec();
        let s = summarize_metrics_request(&body);
        assert!(s.contains("[OTLP LOGS payload"), "{s}");
        assert!(s.contains("codex.response.completed"));
        assert!(s.contains("input_tokens=100"));
    }

    #[test]
    fn decode_round_trips_a_protobuf_body() {
        let bytes = claude_request().encode_to_vec();
        let decoded = decode_metrics_request(&bytes).expect("decode");
        let facts = otlp_metrics_to_token_facts(&decoded);
        assert_eq!(facts.len(), 2);
    }
}
