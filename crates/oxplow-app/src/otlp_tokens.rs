//! OTLP exports → token readings (epic tsk22).
//!
//! Agent CLIs export token usage as OpenTelemetry to oxplow's control-plane
//! OTLP receiver. This module decodes the protobuf body into neutral
//! [`OtlpRecord`]s — metric data points and log records — and asks the
//! registered harnesses what token counts they hold
//! (`AgentHarness::token_readings`): each harness knows its own metric and
//! event names (Claude's token counter, Codex's `response.completed` log
//! event). The readings are the grain `agent.tokens.reported` carries
//! (`otlp_ingest.rs`), which the `token_usage.otlp` consumer writes as
//! facts. Every registered harness is asked, since the names are each
//! harness's own: reading the session's harness first would cost a database
//! read for every export, most of which (Codex's log events) carry no
//! counts. Each harness is asked once per export, with all its records — a
//! provider harness answers in one call.
//!
//! No IO of its own → fully unit-testable. [`summarize_metrics_request`] is the
//! opt-in wire-format diagnostic (tsk25), which also decodes logs.

use opentelemetry_proto::tonic::collector::logs::v1::ExportLogsServiceRequest;
use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::{metric, number_data_point, Gauge, Sum};
use prost::Message;

use oxplow_domain::agent::observe::{AttrValue, Attrs, OtlpRecord, TokenReading};
use oxplow_domain::agent::registry::HarnessRegistry;

/// What one OTLP export reported: its token counts, and the end of the
/// time window they cover — what places them in a turn, since an export
/// arrives after the turn it measured.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenExport {
    pub counts: Vec<TokenReading>,
    /// The earliest its counts cover, when they say (tsk900).
    pub window_start: Option<oxplow_domain::Timestamp>,
    pub window_end: Option<oxplow_domain::Timestamp>,
}

/// Decode an export body: as metrics (Claude), else as logs (Codex points
/// its one endpoint here, and its token counts ride log events), and read
/// its token counts through `harnesses`. `None` when it carries none —
/// most Codex log events don't.
pub async fn decode_token_export(body: &[u8], harnesses: &HarnessRegistry) -> Option<TokenExport> {
    let mut counts = match decode_metrics_request(body) {
        Ok(req) => read(harnesses, &metric_records(&req)).await,
        Err(_) => Vec::new(),
    };
    if counts.is_empty() {
        if let Ok(req) = decode_logs_request(body) {
            counts = read(harnesses, &log_records(&req)).await;
        }
    }
    if counts.is_empty() {
        return None;
    }
    let window_end = counts
        .iter()
        .map(|c| c.at_unix_nano)
        .max()
        .filter(|n| *n > 0)
        .and_then(|n| oxplow_domain::Timestamp::from_unix_nanos(n as i128));
    let window_start = counts
        .iter()
        .map(|c| c.from_unix_nano)
        .filter(|n| *n > 0)
        .min()
        .and_then(|n| oxplow_domain::Timestamp::from_unix_nanos(n as i128));
    Some(TokenExport {
        counts,
        window_start,
        window_end,
    })
}

/// The token counts every registered harness reads in `records`, one
/// call each.
async fn read(harnesses: &HarnessRegistry, records: &[OtlpRecord]) -> Vec<TokenReading> {
    if records.is_empty() {
        return Vec::new();
    }
    let mut counts = Vec::new();
    for h in harnesses.all() {
        counts.extend(h.token_readings(records).await);
    }
    counts
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

/// Every data point of a metrics export: a counter's or gauge's value, a
/// histogram's sum.
fn metric_records(req: &ExportMetricsServiceRequest) -> Vec<OtlpRecord> {
    let mut out = Vec::new();
    for rm in &req.resource_metrics {
        let resource = attrs(
            rm.resource
                .as_ref()
                .map(|r| r.attributes.as_slice())
                .unwrap_or(&[]),
        );
        for m in rm.scope_metrics.iter().flat_map(|sm| &sm.metrics) {
            let mut point = |value: i64, a: &[KeyValue], time: u64, start: u64| {
                out.push(OtlpRecord::Point {
                    metric: m.name.clone(),
                    value,
                    attributes: attrs(a),
                    resource: resource.clone(),
                    time_unix_nano: time,
                    start_time_unix_nano: start,
                })
            };
            match &m.data {
                Some(metric::Data::Sum(Sum { data_points, .. }))
                | Some(metric::Data::Gauge(Gauge { data_points, .. })) => {
                    for dp in data_points {
                        point(
                            number_value(&dp.value),
                            &dp.attributes,
                            dp.time_unix_nano,
                            dp.start_time_unix_nano,
                        );
                    }
                }
                Some(metric::Data::Histogram(hist)) => {
                    for dp in &hist.data_points {
                        point(
                            dp.sum.unwrap_or(0.0) as i64,
                            &dp.attributes,
                            dp.time_unix_nano,
                            dp.start_time_unix_nano,
                        );
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Every log record of a logs export, timed when it happened, else when it
/// was observed.
fn log_records(req: &ExportLogsServiceRequest) -> Vec<OtlpRecord> {
    let mut out = Vec::new();
    for rl in &req.resource_logs {
        let resource = attrs(
            rl.resource
                .as_ref()
                .map(|r| r.attributes.as_slice())
                .unwrap_or(&[]),
        );
        for lr in rl.scope_logs.iter().flat_map(|sl| &sl.log_records) {
            out.push(OtlpRecord::Log {
                attributes: attrs(&lr.attributes),
                resource: resource.clone(),
                time_unix_nano: if lr.time_unix_nano > 0 {
                    lr.time_unix_nano
                } else {
                    lr.observed_time_unix_nano
                },
            });
        }
    }
    out
}

/// OTLP attributes with a scalar value; arrays, maps and bytes are left
/// out.
fn attrs(kvs: &[KeyValue]) -> Attrs {
    Attrs(
        kvs.iter()
            .filter_map(|kv| {
                let value = match kv.value.as_ref()?.value.as_ref()? {
                    any_value::Value::StringValue(s) => AttrValue::Str(s.clone()),
                    any_value::Value::IntValue(i) => AttrValue::Int(*i),
                    any_value::Value::DoubleValue(d) => AttrValue::Double(*d),
                    any_value::Value::BoolValue(b) => AttrValue::Bool(*b),
                    _ => return None,
                };
                Some((kv.key.clone(), value))
            })
            .collect(),
    )
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
pub(crate) mod tests {
    use super::*;

    pub(crate) fn encoded_claude_export(model: &str, input: i64, output: i64) -> Vec<u8> {
        encoded_claude_export_at(model, input, output, None)
    }

    /// [`encoded_claude_export`] whose points cover the window `start..end`
    /// (delta temporality: since the last export, collected at `end`).
    pub(crate) fn encoded_claude_export_over(
        model: &str,
        input: i64,
        output: i64,
        start: oxplow_domain::Timestamp,
        end: oxplow_domain::Timestamp,
    ) -> Vec<u8> {
        let mut req = ExportMetricsServiceRequest::decode(
            encoded_claude_export_at(model, input, output, Some(end)).as_slice(),
        )
        .expect("our own export decodes");
        for rm in &mut req.resource_metrics {
            for sm in &mut rm.scope_metrics {
                for m in &mut sm.metrics {
                    if let Some(metric::Data::Sum(sum)) = &mut m.data {
                        for dp in &mut sum.data_points {
                            dp.start_time_unix_nano = start.unix_nanos() as u64;
                        }
                    }
                }
            }
        }
        req.encode_to_vec()
    }

    /// [`encoded_claude_export`] whose points say they were measured `at`.
    pub(crate) fn encoded_claude_export_at(
        model: &str,
        input: i64,
        output: i64,
        at: Option<oxplow_domain::Timestamp>,
    ) -> Vec<u8> {
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
        let time_unix_nano = at.map_or(0, |t| t.unix_nanos() as u64);
        let point = |ty: &str, val: i64| NumberDataPoint {
            attributes: vec![kv("type", ty), kv("model", model)],
            value: Some(number_data_point::Value::AsInt(val)),
            time_unix_nano,
            ..Default::default()
        };
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: "claude_code.token.usage".into(),
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

    /// [`encoded_claude_export`] with cache points — the tsk73 ingest tests' body.
    pub(crate) fn encoded_claude_export_with_cache(
        model: &str,
        input: i64,
        output: i64,
        cache_read: i64,
        cache_creation: i64,
    ) -> Vec<u8> {
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
                        name: "claude_code.token.usage".into(),
                        data: Some(metric::Data::Sum(Sum {
                            data_points: vec![
                                point("input", input),
                                point("output", output),
                                point("cacheRead", cache_read),
                                point("cacheCreation", cache_creation),
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
        .encode_to_vec()
    }
    use opentelemetry_proto::tonic::common::v1::AnyValue;
    use opentelemetry_proto::tonic::logs::v1::{LogRecord, ResourceLogs, ScopeLogs};
    use opentelemetry_proto::tonic::metrics::v1::{
        Histogram, HistogramDataPoint, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics,
    };
    use oxplow_domain::events::schema::TokenKind;

    /// Claude and Codex registered, as foundation declares them.
    fn harnesses() -> HarnessRegistry {
        let r = HarnessRegistry::new(std::sync::Arc::new(|| "claude".into()));
        for (entry, id) in [
            ("oxplow:claude-code", "claude"),
            ("oxplow:codex-cli", "codex"),
        ] {
            r.register(oxplow_harnesses::built_in(entry, id, id).unwrap());
        }
        r
    }

    /// [`decode_token_export`], run to its end.
    fn decode(body: &[u8], harnesses: &HarnessRegistry) -> Option<TokenExport> {
        futures::executor::block_on(decode_token_export(body, harnesses))
    }

    /// A harness counting its calls, reading every record as one input
    /// token.
    struct Counting(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    #[async_trait::async_trait]
    impl oxplow_domain::agent::harness::AgentHarness for Counting {
        fn id(&self) -> &str {
            "counting"
        }
        fn title(&self) -> &str {
            "Counting"
        }
        fn interact(&self) -> oxplow_domain::agent::harness::Interact {
            oxplow_domain::agent::harness::Interact {
                transcript: oxplow_domain::agent::harness::Transcript::Terminal,
            }
        }
        async fn launch(
            &self,
            _: &oxplow_domain::agent::harness::LaunchInput,
        ) -> Result<
            oxplow_domain::agent::harness::Launch,
            oxplow_domain::agent::harness::HarnessError,
        > {
            Err(oxplow_domain::agent::harness::HarnessError::Config(
                "none".into(),
            ))
        }
        async fn tool_use(
            &self,
            _: &serde_json::Value,
        ) -> Option<oxplow_domain::agent::tool::ToolUse> {
            None
        }
        async fn render(&self, _: &oxplow_domain::agent::observe::HookAnswer) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn token_readings(&self, records: &[OtlpRecord]) -> Vec<TokenReading> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            records
                .iter()
                .map(|r| TokenReading {
                    model: r.model(),
                    kind: TokenKind::Input,
                    value: 1,
                    at_unix_nano: 0,
                    from_unix_nano: 0,
                })
                .collect()
        }
    }

    /// Each harness is asked once per export, with all its records: a
    /// provider harness answers an export in one call.
    #[test]
    fn each_harness_reads_an_export_in_one_call() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let r = HarnessRegistry::new(std::sync::Arc::new(String::new));
        r.register(std::sync::Arc::new(Counting(calls.clone())));
        let export = decode(&encoded_claude_export("m", 100, 20), &r).unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(export.counts.len(), 2, "both points, in one call");
    }

    fn readings(body: &[u8]) -> Vec<TokenReading> {
        decode(body, &harnesses())
            .map(|e| e.counts)
            .unwrap_or_default()
    }

    fn sum_of(readings: &[TokenReading], kind: TokenKind) -> i64 {
        readings
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.value)
            .sum()
    }

    fn kv(k: &str, v: &str) -> KeyValue {
        KeyValue {
            key: k.into(),
            value: Some(AnyValue {
                value: Some(any_value::Value::StringValue(v.into())),
            }),
            ..Default::default()
        }
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

    fn metrics(resource: Vec<KeyValue>, metrics: Vec<Metric>) -> Vec<u8> {
        use opentelemetry_proto::tonic::resource::v1::Resource;
        ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: resource,
                    ..Default::default()
                }),
                scope_metrics: vec![ScopeMetrics {
                    metrics,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec()
    }

    fn sum_metric(name: &str, points: Vec<NumberDataPoint>) -> Metric {
        Metric {
            name: name.into(),
            data: Some(metric::Data::Sum(Sum {
                data_points: points,
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    fn point(attributes: Vec<KeyValue>, value: i64) -> NumberDataPoint {
        NumberDataPoint {
            attributes,
            value: Some(number_data_point::Value::AsInt(value)),
            ..Default::default()
        }
    }

    fn logs(records: Vec<LogRecord>) -> Vec<u8> {
        ExportLogsServiceRequest {
            resource_logs: vec![ResourceLogs {
                scope_logs: vec![ScopeLogs {
                    log_records: records,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec()
    }

    /// tsk860: an export says when it was measured — the latest of its
    /// points' times — which is what places it in a turn; one that doesn't
    /// say has no window end, and a body with no token counts is none. It
    /// keeps the export's precision: a turn stored to the microsecond that
    /// began earlier in the same millisecond is still before it.
    #[test]
    fn an_export_reports_its_window_end() {
        let at = oxplow_domain::Timestamp::from_unix_nanos(1_790_000_000_123_456_000).unwrap();
        let export = decode(
            &encoded_claude_export_at("m", 100, 20, Some(at)),
            &harnesses(),
        )
        .expect("token counts");
        assert_eq!(export.window_end, Some(at));
        assert_eq!(export.counts.len(), 2);
        let undated = decode(&encoded_claude_export("m", 100, 20), &harnesses());
        assert_eq!(undated.unwrap().window_end, None);
        assert_eq!(decode(b"not otlp", &harnesses()), None);
    }

    /// A counter's points reach the harness with their attributes and the
    /// resource's: all four of Claude's kinds come back, the model read
    /// from the resource when a point doesn't name one; another metric and
    /// a zero point read as nothing.
    #[test]
    fn a_counter_export_reads_through_the_harnesses() {
        let body = metrics(
            vec![kv("model", "claude-sonnet-5")],
            vec![
                sum_metric(
                    "claude_code.token.usage",
                    vec![
                        point(vec![kv("type", "input")], 100),
                        point(
                            vec![kv("type", "output"), kv("model", "claude-opus-4-8")],
                            20,
                        ),
                        point(vec![kv("type", "cacheRead")], 5000),
                        point(vec![kv("type", "cacheCreation")], 700),
                        point(vec![kv("type", "output")], 0),
                    ],
                ),
                sum_metric(
                    "claude_code.cost.usage",
                    vec![point(vec![kv("type", "input")], 9)],
                ),
            ],
        );
        let r = readings(&body);
        assert_eq!(r.len(), 4);
        assert_eq!(sum_of(&r, TokenKind::Input), 100);
        assert_eq!(sum_of(&r, TokenKind::CacheRead), 5000);
        assert_eq!(sum_of(&r, TokenKind::CacheCreation), 700);
        let models: Vec<&str> = r.iter().map(|r| r.model.as_str()).collect();
        assert_eq!(
            models,
            [
                "claude-sonnet-5",
                "claude-opus-4-8",
                "claude-sonnet-5",
                "claude-sonnet-5"
            ]
        );
    }

    /// A histogram's sum is its value.
    #[test]
    fn a_histogram_export_reads_its_sums() {
        let hp = |token_type: &str, sum: f64| HistogramDataPoint {
            attributes: vec![kv("token_type", token_type)],
            sum: Some(sum),
            ..Default::default()
        };
        let body = metrics(
            vec![],
            vec![Metric {
                name: "codex.turn.token_usage".into(),
                data: Some(metric::Data::Histogram(Histogram {
                    data_points: vec![hp("input", 100.0), hp("reasoning_output", 30.0)],
                    ..Default::default()
                })),
                ..Default::default()
            }],
        );
        let r = readings(&body);
        assert_eq!(sum_of(&r, TokenKind::Input), 100);
        assert_eq!(sum_of(&r, TokenKind::Output), 30);
    }

    /// A body that isn't metrics is read as logs, ints and numeric strings
    /// alike, timed by the record; a log event with no counts is none.
    #[test]
    fn a_logs_export_reads_through_the_harnesses() {
        let rec = LogRecord {
            time_unix_nano: 5,
            attributes: vec![
                kv("event.kind", "response.completed"),
                kv_int("input_token_count", 113690),
                kv_int("cached_token_count", 2432),
                kv("output_token_count", "254"),
                kv("model", "gpt-5.5"),
            ],
            ..Default::default()
        };
        let r = readings(&logs(vec![rec]));
        assert_eq!(sum_of(&r, TokenKind::Input), 111258);
        assert_eq!(sum_of(&r, TokenKind::Output), 254);
        assert!(r
            .iter()
            .all(|r| r.at_unix_nano == 5 && r.model == "gpt-5.5"));
        let other = LogRecord {
            attributes: vec![kv("event.name", "codex.api_request")],
            ..Default::default()
        };
        assert_eq!(decode(&logs(vec![other]), &harnesses()), None);
    }

    /// No harness registered reads no counts.
    #[test]
    fn with_no_harness_an_export_reads_as_none() {
        let none = HarnessRegistry::new(std::sync::Arc::new(String::new));
        assert_eq!(decode(&encoded_claude_export("m", 100, 20), &none), None);
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
        let body = logs(vec![LogRecord {
            event_name: "codex.response.completed".into(),
            attributes: vec![kv("input_tokens", "100"), kv("output_tokens", "20")],
            ..Default::default()
        }]);
        let s = summarize_metrics_request(&body);
        assert!(s.contains("[OTLP LOGS payload"), "{s}");
        assert!(s.contains("codex.response.completed"));
        assert!(s.contains("input_tokens=100"));
    }
}
