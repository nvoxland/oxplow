//! The filesystems core ships (`.context/filesystems.md`), registered in
//! `Services.filesystems`: `oxplow:local`, `oxplow:extensions` and `oxplow:temp`. Each one implements
//! `oxplow_domain::filesystem::FileSystem`.

pub mod extensions;
pub mod local;
pub mod temp;

use serde_json::{json, Value};

use crate::capabilities::{Implementation, Source};

/// The filesystem capability's implementations that are core's own: every
/// filesystem core registers, as the rows of `v_capability_provider`.
pub fn implementations() -> Vec<Implementation> {
    let row = |id: &str, title: &str, features: Value| Implementation {
        capability: "filesystem".into(),
        id: id.into(),
        title: title.into(),
        extension: None,
        source: Source::Core,
        features,
        fields: Value::Array(Vec::new()),
        id_pattern: None,
        icon: None,
        config: json!({}),
        description: None,
    };
    vec![
        row("local", "The stream's files", json!({ "writable": true })),
        row("extensions", "The running copies of extensions", json!({})),
        row("temp", "Scratch files", json!({})),
    ]
}
