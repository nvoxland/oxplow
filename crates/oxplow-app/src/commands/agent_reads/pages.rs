//! `pages.read/open { thread_id }`: what the person is looking at in a
//! thread — the page its window last reported open and, for a lens, that
//! lens re-run with the person's params, as text.

use std::sync::Arc;

use oxplow_domain::{CommandError, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};

/// The command its operation is declared as.
pub const PAGE_OPEN: &str = "oxplow.page.open";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenPageInput {
    /// Your thread (`thr…`).
    pub thread_id: String,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![Op::new(
        "pages.read",
        "open",
        schema::<OpenPageInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let OpenPageInput { thread_id } = parse(input)?;
                let thread = ThreadId::try_from_str(&thread_id).ok_or_else(|| {
                    invalid(
                        "/thread_id",
                        format!("expected a thread id (thr…), got `{thread_id}`"),
                    )
                })?;
                let svc = services.get()?;
                let Some(page) = svc.thread_runtime.open_page(&thread) else {
                    return Ok(HandlerOutput {
                        result: json!({ "open": null }),
                        ..HandlerOutput::default()
                    });
                };
                let detail: Value = page
                    .detail_json
                    .as_deref()
                    .and_then(|d| serde_json::from_str(d).ok())
                    .unwrap_or(Value::Null);
                let mut lens_text = Value::Null;
                if page.kind == "lens" {
                    if let Ok(lens_id) = oxplow_domain::refs::build::id_of(
                        &page.page_id,
                        oxplow_domain::refs::kind::LENS,
                    ) {
                        // The app shows the main worktree's lenses, whatever
                        // the thread's stream.
                        let root = svc.worktrees.project_dir();
                        let lens_params = detail
                            .get("params")
                            .and_then(|p| p.as_object())
                            .map(|m| {
                                m.iter()
                                    .map(|(k, v)| (k.clone(), oxplow_db::SqlCell::from(v.clone())))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let ctx = crate::extensions::lens_context(&svc, None, Some(thread)).await;
                        let read = async {
                            let run = crate::extensions::run_lens(
                                &svc.sql,
                                &svc.extension_catalog,
                                root,
                                &lens_id,
                                lens_params,
                                &ctx,
                            )
                            .await?;
                            crate::lens_text::text_run(&svc, root, &run, &ctx).await
                        };
                        lens_text = match read.await {
                            Ok(text) => serde_json::to_value(text).map_err(failed)?,
                            Err(e) => json!({ "error": e.to_string() }),
                        };
                    }
                }
                Ok::<_, CommandError>(HandlerOutput {
                    result: json!({
                        "open": {
                            "pageId": page.page_id,
                            "kind": page.kind,
                            "detail": detail,
                            "reportedAt": page.reported_at,
                        },
                        "lens": lens_text,
                    }),
                    ..HandlerOutput::default()
                })
            })
        })),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::Actor;

    /// What the window reported open in a thread reads back through the
    /// bus (null while nothing has been reported); a non-thread id is
    /// refused at its field.
    #[tokio::test]
    async fn the_open_page_reads_through_the_bus() {
        let fx = services_with_effort().await;
        let actor = Actor::Agent {
            session_id: None,
            thread_id: Some(fx.thread),
            stream_id: None,
        };
        let tid = fx.thread.to_string();
        let none = fx
            .svc
            .commands
            .run(&actor, PAGE_OPEN, json!({ "thread_id": tid }), false)
            .await
            .unwrap();
        assert_eq!(none.result["open"], Value::Null);
        assert!(none.audit_id.is_none(), "a read leaves no record");

        fx.svc.thread_runtime.set_open_page(
            &fx.thread,
            Some(crate::thread_runtime::OpenPage {
                page_id: "file:src/a.rs".into(),
                kind: "file".into(),
                detail_json: Some(r#"{"line":3}"#.into()),
                reported_at: oxplow_domain::Timestamp::now(),
            }),
        );
        let open = fx
            .svc
            .commands
            .run(&actor, PAGE_OPEN, json!({ "thread_id": tid }), false)
            .await
            .unwrap();
        assert_eq!(open.result["open"]["pageId"], "file:src/a.rs");
        assert_eq!(open.result["open"]["detail"]["line"], 3);
        assert_eq!(open.result["lens"], Value::Null);

        let err = fx
            .svc
            .commands
            .run(&actor, PAGE_OPEN, json!({ "thread_id": "str1" }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/thread_id"),
            "{err:?}"
        );
    }
}
