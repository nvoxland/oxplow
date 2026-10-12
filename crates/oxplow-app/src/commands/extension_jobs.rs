//! Audited intent/cancellation and private recorded result operations. Extension
//! declarations bind these to their own package; inputs contain no owner or grant.

use super::{ops::Op, util::schema, Handler, HandlerOutput};
use crate::extension_jobs::{EnqueueInput, ExtensionJobs, JobInput};
use oxplow_domain::CommandError;
use std::sync::Arc;

pub fn ops(jobs: Arc<ExtensionJobs>) -> Vec<Op> {
    let enqueue = jobs.clone();
    let snapshot = jobs.clone();
    let manage = jobs.clone();
    vec![
        Op::new(
            "jobs.write",
            "enqueue",
            schema::<EnqueueInput>(),
            false,
            Handler::Tx(Arc::new(move |ctx, input| {
                let input = serde_json::from_value(input).map_err(|_| invalid())?;
                Ok(HandlerOutput {
                    result: enqueue.enqueue(ctx, &input)?,
                    ..Default::default()
                })
            })),
        )
        .with_unrecorded(&["request"]),
        Op::new(
            "jobs.write",
            "cancel",
            schema::<JobInput>(),
            false,
            Handler::Tx(Arc::new(move |ctx, input| {
                let input = serde_json::from_value(input).map_err(|_| invalid())?;
                Ok(HandlerOutput {
                    result: jobs.cancel(ctx, &input)?,
                    ..Default::default()
                })
            })),
        ),
        Op::new(
            "jobs.manage",
            "cancel",
            schema::<JobInput>(),
            false,
            Handler::Tx(Arc::new(move |ctx, input| {
                let input = serde_json::from_value(input).map_err(|_| invalid())?;
                Ok(HandlerOutput {
                    result: manage.cancel_any(ctx, &input)?,
                    ..Default::default()
                })
            })),
        )
        .open_to(oxplow_domain::Invokers::NO_AGENT),
        Op::new(
            "jobs.read",
            "snapshot",
            schema::<JobInput>(),
            false,
            Handler::Tx(Arc::new(move |ctx, input| {
                let input = serde_json::from_value(input).map_err(|_| invalid())?;
                Ok(HandlerOutput {
                    result: snapshot.snapshot(ctx.conn, ctx.run, &input)?,
                    ..Default::default()
                })
            })),
        ),
    ]
}

fn invalid() -> CommandError {
    CommandError::Invalid {
        field: None,
        message: "invalid job service input".into(),
    }
}
