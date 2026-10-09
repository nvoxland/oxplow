//! Model providers written as scripts (`ai_provider` implementations,
//! `.context/ai-providers.md` "Scripted providers"): an extension's
//! Starlark script shapes each call and reads its reply, and the host
//! makes the one HTTP call. oxplow's own (Anthropic, OpenAI and any
//! OpenAI-compatible API, OpenRouter, TypeSafe) ship as scripts in
//! `oxplow-foundation`, approved like any other.

mod scripted;

pub use scripted::{scripted, Gate, FUNCTIONS};
