//! The fake harness's agent: one scripted session posted to oxplow's hook
//! route (`$OXPLOW_HOOK_BASE_URL/<event>`) and OTLP receiver
//! (`$OXPLOW_FAKE_OTLP_URL`) with the identity its launch gave it. Every
//! hook answer must be the fake's own shape (`{"fake": …}`): anything else
//! means core rendered it, and the process fails.

use opentelemetry_proto::tonic::collector::metrics::v1::ExportMetricsServiceRequest;
use opentelemetry_proto::tonic::common::v1::{any_value, AnyValue, KeyValue};
use opentelemetry_proto::tonic::metrics::v1::{
    metric, number_data_point, Metric, NumberDataPoint, ResourceMetrics, ScopeMetrics, Sum,
};
use prost::Message;
use serde_json::{json, Value};

use oxplow_harness_fake::{EDITED, TOKENS, TOKEN_METRIC};

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

struct Agent {
    http: reqwest::Client,
    session: &'static str,
}

impl Agent {
    async fn hook(&self, event: &str, mut body: Value) -> Result<Value, String> {
        body["session_id"] = json!(self.session);
        let url = format!("{}/{event}", env("OXPLOW_HOOK_BASE_URL"));
        let answer: Value = self
            .http
            .post(url)
            .bearer_auth(env("OXPLOW_HOOK_TOKEN"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("{event}: {e}"))?
            .json()
            .await
            .map_err(|e| format!("{event}: {e}"))?;
        if !answer["fake"].is_string() {
            return Err(format!("{event}: not the fake's answer: {answer}"));
        }
        Ok(answer)
    }

    async fn export_tokens(&self) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos() as u64;
        let point = |kind: &str, value: i64| NumberDataPoint {
            attributes: vec![
                KeyValue {
                    key: "kind".into(),
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue(kind.into())),
                    }),
                    ..Default::default()
                },
                KeyValue {
                    key: "model".into(),
                    value: Some(AnyValue {
                        value: Some(any_value::Value::StringValue("fake-model".into())),
                    }),
                    ..Default::default()
                },
            ],
            time_unix_nano: now,
            start_time_unix_nano: now.saturating_sub(1_000_000),
            value: Some(number_data_point::Value::AsInt(value)),
            ..Default::default()
        };
        let body = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                scope_metrics: vec![ScopeMetrics {
                    metrics: vec![Metric {
                        name: TOKEN_METRIC.into(),
                        data: Some(metric::Data::Sum(Sum {
                            data_points: vec![point("input", TOKENS.0), point("output", TOKENS.1)],
                            ..Default::default()
                        })),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
        .encode_to_vec();
        let status = self
            .http
            .post(env("OXPLOW_FAKE_OTLP_URL"))
            .bearer_auth(env("OXPLOW_HOOK_TOKEN"))
            .header("Content-Type", "application/x-protobuf")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("otlp: {e}"))?
            .status();
        if !status.is_success() {
            return Err(format!("otlp: {status}"));
        }
        Ok(())
    }

    async fn run(&self) -> Result<(), String> {
        let edited = std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(EDITED);
        let edit = json!({ "tool_name": "Edit", "tool_input": { "file_path": edited } });
        self.hook("SessionStart", json!({ "source": "startup" }))
            .await?;
        self.hook("UserPromptSubmit", json!({ "prompt": "change the file" }))
            .await?;
        let allowed = self.hook("PreToolUse", edit.clone()).await?;
        if allowed["fake"] != "ack" {
            return Err(format!("the edit was refused: {allowed}"));
        }
        std::fs::write(&edited, "changed\n").map_err(|e| e.to_string())?;
        let mut done = edit;
        done["tool_response"] = json!({ "success": true });
        self.hook("PostToolUse", done).await?;
        self.export_tokens().await?;
        self.hook("Stop", json!({ "last_assistant_message": "Done." }))
            .await?;
        Ok(())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let agent = Agent {
        http: reqwest::Client::new(),
        session: "fake-session-1",
    };
    if let Err(e) = agent.run().await {
        eprintln!("oxplow-harness-fake: {e}");
        std::process::exit(2);
    }
}
