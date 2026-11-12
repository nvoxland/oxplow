//! What a capability's adapter calls (`.context/providers.md` "What a
//! provider may implement"): an [`Endpoint`] answers the capability's verbs.
//! A provider instance's process is one ([`super::Instance`]); a script an
//! extension declares as an implementation is another
//! ([`super::script::ScriptEndpoint`]). Each capability's host turns an
//! endpoint into the capability's implementation the same way whichever it
//! is, so a capability whose interface is a set of verbs takes both.

use async_trait::async_trait;
use oxplow_domain::events::Envelope;
use oxplow_domain::run::RunContext;
use oxplow_domain::tasks::StageDecl;
use oxplow_domain::{Actor, CommandError};
use oxplow_provider_protocol::model::{EventDraft, InitializeResult, InvokeResult};
use serde_json::Value;

#[async_trait]
pub trait Endpoint: Send + Sync {
    /// Its id: the capability provider's id, its refs' segment
    /// (`oxplow:oxplow/task?id=…&provider=<id>`).
    fn id(&self) -> &str;
    /// How messages name it: `<namespace>/<id>`.
    fn name(&self) -> &str;
    /// The extension that declares it.
    fn extension(&self) -> &str;
    /// What it declares of itself: the capability with its features and
    /// `data`, the verbs it answers and their input schemas.
    fn declared(&self) -> &InitializeResult;
    /// An item-keeping store's stages, in order; empty for anything else.
    fn stages(&self) -> &[StageDecl];
    /// What an item-keeping store's own ids look like (a regex matched whole).
    fn id_pattern(&self) -> Option<&str>;
    /// Call `verb` with `input` for `run`, under `idempotency_key` (the
    /// write's, when the caller has one).
    async fn invoke(
        &self,
        run: &RunContext,
        verb: &str,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<InvokeResult, CommandError>;
    /// The envelope core logs for an event a verb answered with, by
    /// `actor` — refused unless the capability's contract lets it emit
    /// that type, and its record and subjects are its own.
    fn envelope(&self, actor: &Actor, draft: EventDraft) -> Result<Envelope, CommandError>;
    /// Start over: a process ends (the next call starts another); what
    /// keeps no process has nothing to end.
    async fn restart(&self);
}

/// The envelope of `draft` from the implementation `id` (`name` in
/// messages) of `capability`, declared by `extension`, by `actor`: refused
/// unless `emits` it (its declarations, within its contract's events), its
/// record is of its own items and every subject is one of its refs. One
/// rule for an instance and a script.
pub(crate) fn envelope_of(
    name: &str,
    id: &str,
    extension: &str,
    capability: &str,
    emits: impl Fn(&str, u32) -> bool,
    actor: &Actor,
    draft: EventDraft,
) -> Result<Envelope, CommandError> {
    let failed = |message: String| CommandError::Failed { message };
    if !emits(&draft.event_type, draft.v) {
        return Err(failed(format!(
            "provider `{name}` returned `{}@{}`, which it doesn't declare",
            draft.event_type, draft.v
        )));
    }
    let Some(contract) = oxplow_domain::capability::contract(capability) else {
        return Err(failed(format!(
            "provider `{name}`: `{capability}` has no provider contract"
        )));
    };
    if let Some(record) = contract
        .records
        .filter(|r| r.event == (draft.event_type.as_str(), draft.v))
    {
        let row = draft.payload[record.key]["uri"]
            .as_str()
            .unwrap_or_default();
        if !super::registry::owns(contract, id, row) {
            return Err(failed(format!(
                "provider `{name}` recorded `{row}`, which isn't one of its {}s",
                record.entity
            )));
        }
    }
    for subject in &draft.subject {
        super::registry::check_subject(capability, id, extension, subject).map_err(failed)?;
    }
    // An agent's thread anchors what it did.
    let anchors = match actor {
        Actor::Agent { .. } => actor.anchors(),
        _ => oxplow_domain::Anchors::default(),
    };
    Envelope::new(draft.event_type, draft.v, actor.source(), draft.payload)
        .map(|e| e.with_subject(draft.subject).with_anchors(anchors))
        .map_err(|e| failed(e.to_string()))
}
