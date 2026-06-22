//! Re-exports + helpers for the time crate.
//!
//! All timestamps in oxplow are UTC and RFC 3339 over the wire. In SQLite they
//! are stored as RFC 3339 TEXT; stores that order or range-compare on a
//! timestamp column write it in a **fixed-width canonical form**
//! (`…SS.ffffffZ`) via `oxplow_db::database::canonical_ts`, because the `time`
//! crate trims trailing fractional-second zeros and lexicographic comparison
//! would otherwise misorder same-second values (see `.context/data-model.md`).
//! Use `Timestamp` rather than reaching for `time::OffsetDateTime` directly so
//! swapping representations later stays cheap.

use ::time::OffsetDateTime;
use serde::{Deserialize, Serialize};
use specta::Type;

/// Wall-clock UTC timestamp serialized as RFC 3339 strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct Timestamp(#[serde(with = "::time::serde::rfc3339")] pub OffsetDateTime);

impl Timestamp {
    pub fn now() -> Self {
        Self(OffsetDateTime::now_utc())
    }

    pub fn from_unix_ms(ms: i64) -> Self {
        let secs = ms / 1000;
        let ns = ((ms % 1000) * 1_000_000) as i32;
        let nanos = secs * 1_000_000_000 + ns as i64;
        Self(OffsetDateTime::from_unix_timestamp_nanos(nanos as i128).expect("valid timestamp"))
    }

    pub fn unix_ms(&self) -> i64 {
        (self.0.unix_timestamp_nanos() / 1_000_000) as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_unix_ms() {
        let ms = 1_700_000_000_123_i64;
        let ts = Timestamp::from_unix_ms(ms);
        assert_eq!(ts.unix_ms(), ms);
    }

    #[test]
    fn round_trip_rfc3339() {
        let ts = Timestamp::from_unix_ms(1_700_000_000_000);
        let json = serde_json::to_string(&ts).unwrap();
        let back: Timestamp = serde_json::from_str(&json).unwrap();
        assert_eq!(ts, back);
    }
}
