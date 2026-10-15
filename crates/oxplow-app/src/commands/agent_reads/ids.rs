//! The id checks the agent's reads share: a stream or thread id an agent
//! passes is checked against the id grammar, so a mix-up is corrected in
//! one round trip instead of surfacing as a foreign-key error.

use std::sync::Arc;

use oxplow_domain::{Actor, AnyId, CommandError, StreamId};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::super::ops::Op;
use super::super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};
use super::{stream_or_actors, Services};

/// One operation of `scope` that reads `T` and runs `run` over the
/// services as the calling actor.
pub fn read_op<T, F, Fut>(scope: &str, name: &str, services: ServicesSlot, run: F) -> Op
where
    T: DeserializeOwned + JsonSchema + 'static,
    F: Fn(Arc<Services>, Actor, T) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<HandlerOutput, CommandError>> + Send + 'static,
{
    let run = Arc::new(run);
    Op::new(
        scope,
        name,
        schema::<T>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let services = services.clone();
            let run = run.clone();
            Box::pin(async move {
                let input: T = parse(input)?;
                run(services.get()?, invocation.actor().clone(), input).await
            })
        })),
    )
}

/// A handler's output carrying `value` as its result.
pub fn result_of(value: impl serde::Serialize) -> Result<HandlerOutput, CommandError> {
    Ok(HandlerOutput {
        result: serde_json::to_value(value).map_err(failed)?,
        ..HandlerOutput::default()
    })
}

/// `value` is a `<prefix>…` id of the kind `prefix` (`str`, `thr`),
/// else a refusal at `field` naming what it looks like.
pub fn expect_id_kind(
    tool: &str,
    field: &str,
    value: &str,
    prefix: &str,
    label: &str,
) -> Result<(), CommandError> {
    match value.parse::<AnyId>() {
        Ok(any) if any.kind.prefix() == prefix => Ok(()),
        Ok(any) => Err(invalid(
            field,
            format!(
                "{tool}: `{}` expects a {label}, but got `{value}` which looks like a {}",
                field.trim_start_matches('/'),
                any.kind.label()
            ),
        )),
        Err(_) => Err(invalid(
            field,
            format!(
                "{tool}: `{}` expects a {label}, but got `{value}` which isn't a valid id",
                field.trim_start_matches('/')
            ),
        )),
    }
}

/// A stream id (`str3`), checked.
pub fn stream_id(tool: &str, field: &str, value: &str) -> Result<StreamId, CommandError> {
    expect_id_kind(tool, field, value, "str", "stream id (str…)")?;
    StreamId::try_from_str(value)
        .ok_or_else(|| invalid(field, format!("`{value}` is not a stream id")))
}

/// An optional stream id, checked when given.
pub fn check_optional_stream(
    tool: &str,
    field: &str,
    value: Option<&str>,
) -> Result<(), CommandError> {
    value.map_or(Ok(()), |v| {
        expect_id_kind(tool, field, v, "str", "stream id (str…)")
    })
}

/// The stream the call names, else the actor's, else the primary (for a
/// read that needs one).
pub async fn stream_or_primary(
    svc: &Services,
    actor: &oxplow_domain::Actor,
    given: Option<String>,
) -> Result<String, CommandError> {
    if let Some(stream) = stream_or_actors(svc, actor, given).await? {
        return Ok(stream);
    }
    svc.streams
        .list_streams()
        .await
        .map_err(failed)?
        .into_iter()
        .find(|s| s.kind == oxplow_domain::StreamKind::Primary)
        .map(|s| s.id.to_string())
        .ok_or_else(|| invalid("/stream_id", "this project has no stream"))
}
