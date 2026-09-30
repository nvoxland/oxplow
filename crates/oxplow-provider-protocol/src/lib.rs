//! oxplow's external-provider protocol (P5.D1, `.context/providers.md`):
//! a provider is a process oxplow talks to over stdio, JSON-RPC 2.0 with
//! one message per line.
//!
//! - [`codec`] — the messages and their NDJSON form, and the
//!   notifications about an in-flight request (`$/cancel`, `$/progress`,
//!   `$/record`, `$/state`);
//! - [`peer`] — one side of a connection: requests with their replies,
//!   notifications, answering the other side;
//! - [`model`] — the meta-model: `initialize`, `check`, `discover`,
//!   `invoke`, `read`, as Rust types defined once;
//! - [`errors`] — JSON-RPC's codes plus `NotConfigured`, `Auth`,
//!   `RateLimited`, `InvalidInput`, `Cancelled`;
//! - [`schemas`] — every wire type's JSON Schema (the goldens under
//!   `schemas/`), and validation against them.

pub mod codec;
pub mod errors;
pub mod model;
pub mod peer;
pub mod schemas;

pub use codec::{Id, Message};
pub use errors::{ErrorObject, ProtocolError};
pub use peer::{Call, Incoming, Peer};
