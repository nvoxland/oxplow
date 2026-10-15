//! `agent_text.read/skill { name, reference? }`: one of oxplow's skills
//! (its `SKILL.md`, or a `references/<name>.md` of it), as the active
//! implementations offer it — for agents that don't load skill files
//! themselves.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::super::ops::Op;
use super::super::util::{invalid, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};

/// The command its operation is declared as.
pub const SKILL_READ: &str = "oxplow.skill.read";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillInput {
    /// The skill's name (`oxplow-extension`); your instructions list them.
    pub name: String,
    /// A `references/<name>.md` of the skill, by that `<name>`.
    #[serde(default)]
    pub reference: Option<String>,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![Op::new(
        "agent_text.read",
        "skill",
        schema::<SkillInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let SkillInput { name, reference } = parse(input)?;
                let svc = services.get()?;
                // What's offered now: core's and the active implementations'.
                let text = crate::capabilities::agent_text(&svc);
                let found = match &reference {
                    None => text.skill_body(&name),
                    Some(reference) => text.skill_reference(&name, reference),
                };
                let body = match found {
                    Some(body) => body.to_string(),
                    None if text.skill_body(&name).is_some() => {
                        return Err(invalid(
                            "/reference",
                            format!(
                                "skill `{name}` has no reference `{}` (references: {})",
                                reference.unwrap_or_default(),
                                text.skill_reference_names(&name).join(", ")
                            ),
                        ))
                    }
                    None => {
                        let names: Vec<&str> =
                            text.skill_index().into_iter().map(|(n, _)| n).collect();
                        return Err(invalid(
                            "/name",
                            format!("no skill `{name}` (skills: {})", names.join(", ")),
                        ));
                    }
                };
                Ok(HandlerOutput {
                    result: Value::String(body),
                    ..HandlerOutput::default()
                })
            })
        })),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    fn anonymous() -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        }
    }

    /// A skill's body, and one of its references, read through the bus
    /// with no thread behind the call; an unknown name or reference says
    /// what there is.
    #[tokio::test]
    async fn a_skill_and_its_references_read_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let body = svc
            .commands
            .run(
                &anonymous(),
                SKILL_READ,
                json!({ "name": "oxplow-extension" }),
                false,
            )
            .await
            .unwrap();
        assert!(body.result.as_str().unwrap().contains("oxplow-extension"));
        assert!(body.audit_id.is_none(), "a read leaves no record");

        let err = svc
            .commands
            .run(
                &anonymous(),
                SKILL_READ,
                json!({ "name": "oxplow-extension", "reference": "nope" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. }
                if message.contains("has no reference `nope`") && message.contains("references:")),
            "{err:?}"
        );
        let err = svc
            .commands
            .run(&anonymous(), SKILL_READ, json!({ "name": "nope" }), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("skills:")),
            "{err:?}"
        );
    }
}
