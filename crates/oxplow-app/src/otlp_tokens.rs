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
//! Pure + no IO → fully unit-testable. Phase 1 recognizes Claude Code's
//! `claude_code.token.usage` counter and keeps only the `input`/`output` token
//! kinds (cache/reasoning are dropped, matching today's substrate). Codex's
//! `codex.turn.token_usage` histogram lands in phase 2 ([`tsk24`]).

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::{metric, number_data_point};
use prost::Message;

/// Claude Code's per-model token counter (delta temporality). Its `type`
/// attribute carries the token kind; `model` the model id.
const CLAUDE_TOKEN_METRIC: &str = "claude_code.token.usage";

/// The token kinds oxplow tracks. Cache (`cacheRead`/`cacheCreation`) and
/// reasoning tokens are intentionally not modelled in phase 1 — reasoning
/// already lives inside Claude's `output`, and cache kinds can be added later
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

/// Project the token data points from a decoded OTLP metrics export into
/// [`TokenFact`]s. Non-token metrics and non-input/output token kinds are
/// ignored; zero-valued points are skipped.
pub fn otlp_metrics_to_token_facts(req: &ExportMetricsServiceRequest) -> Vec<TokenFact> {
    let mut out = Vec::new();
    for rm in &req.resource_metrics {
        for sm in &rm.scope_metrics {
            for m in &sm.metrics {
                if m.name != CLAUDE_TOKEN_METRIC {
                    continue;
                }
                // Claude's `token.usage` is a monotonic counter → OTLP Sum; be
                // lenient and also accept a Gauge shape.
                let points = match &m.data {
                    Some(metric::Data::Sum(sum)) => &sum.data_points,
                    Some(metric::Data::Gauge(gauge)) => &gauge.data_points,
                    _ => continue,
                };
                for dp in points {
                    let Some(kind) = token_kind_of(&dp.attributes) else {
                        continue;
                    };
                    let value = number_value(&dp.value);
                    if value == 0 {
                        continue;
                    }
                    let model = string_attr(&dp.attributes, "model")
                        .unwrap_or_else(|| "unknown".to_string());
                    out.push(TokenFact { model, kind, value });
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

/// Map the `type` attribute to a tracked [`TokenKind`] (drops cache/reasoning).
fn token_kind_of(attrs: &[KeyValue]) -> Option<TokenKind> {
    match string_attr(attrs, "type")?.as_str() {
        "input" => Some(TokenKind::Input),
        "output" => Some(TokenKind::Output),
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
        Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
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
    fn decode_round_trips_a_protobuf_body() {
        let bytes = claude_request().encode_to_vec();
        let decoded = decode_metrics_request(&bytes).expect("decode");
        let facts = otlp_metrics_to_token_facts(&decoded);
        assert_eq!(facts.len(), 2);
    }
}
