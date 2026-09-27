//! Oxplow's own model access: providers configured once (keys in the OS
//! keychain), used everywhere through roles. See `.context/ai-providers.md`.

pub mod client;
pub mod config;
pub mod secrets;

#[cfg(any(test, feature = "test-support"))]
pub mod testing;
