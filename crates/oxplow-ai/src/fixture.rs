//! A model that answers from a list, in-process: what a preview of an
//! extension (`oxplow extension test`, `preview_extension`) runs its model
//! calls against, so nothing is sent and nothing is spent. Registered as a
//! host kind (`ModelProviders::register_host`), never declared by an
//! extension.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::client::{AiError, CompleteRequest, Completion, ModelProvider, ProviderInstance};

/// Its kind, as `ai.yaml`'s `kind:` names it.
pub const KIND: &str = "fixture";

/// Answers with each reply in turn; the last repeats. Usage is unknown.
pub struct FixtureModel {
    replies: Mutex<VecDeque<String>>,
    last: Mutex<String>,
    asked: Mutex<Vec<String>>,
}

impl FixtureModel {
    pub fn new(replies: Vec<String>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            last: Mutex::new(String::new()),
            asked: Mutex::new(Vec::new()),
        }
    }

    /// The prompts it was asked, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait::async_trait]
impl ModelProvider for FixtureModel {
    fn kind(&self) -> &str {
        KIND
    }
    fn title(&self) -> &str {
        "Fixture replies"
    }
    fn default_base_url(&self) -> Option<&str> {
        Some("fixture:")
    }
    async fn complete(
        &self,
        _instance: &ProviderInstance<'_>,
        req: &CompleteRequest<'_>,
    ) -> Result<Completion, AiError> {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(req.prompt.to_string());
        let next = self
            .replies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(next) = next {
            *last = next;
        }
        Ok(Completion {
            text: last.clone(),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn it_answers_each_reply_in_turn_then_repeats_the_last() {
        let model = FixtureModel::new(vec!["one".into(), "two".into()]);
        let instance = ProviderInstance {
            id: "f",
            base_url: None,
            key: None,
        };
        let ask = |prompt: &'static str| CompleteRequest {
            model: "m",
            system: None,
            prompt,
            json: false,
        };
        let mut said = Vec::new();
        for prompt in ["a", "b", "c"] {
            said.push(model.complete(&instance, &ask(prompt)).await.unwrap().text);
        }
        assert_eq!(said, ["one", "two", "two"]);
        assert_eq!(model.asked(), ["a", "b", "c"]);
    }
}
