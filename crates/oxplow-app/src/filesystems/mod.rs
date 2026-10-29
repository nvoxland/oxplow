//! The filesystems core ships (`.context/filesystems.md`), registered in
//! `Services.filesystems`: `oxplow:local` now. Each one implements
//! `oxplow_domain::filesystem::FileSystem`.

pub mod local;

use serde_json::{json, Value};

use crate::capabilities::{Implementation, Source};

/// The filesystem capability's implementations that are core's own: every
/// filesystem core registers, as the rows of `v_capability_provider`.
pub fn implementations() -> Vec<Implementation> {
    vec![Implementation {
        capability: "filesystem".into(),
        id: "local".into(),
        title: "The stream's files".into(),
        extension: None,
        source: Source::Core,
        features: json!({ "writable": true }),
        fields: Value::Array(Vec::new()),
        id_pattern: None,
        icon: None,
        config: json!({}),
    }]
}
