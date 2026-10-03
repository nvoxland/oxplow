//! Re-exports + helpers for the time crate.
//!
//! All timestamps in oxplow are UTC and RFC 3339 over the wire **and in a
//! fixed-width form**: `YYYY-MM-DDTHH:MM:SS.ffffffZ`, always six fractional
//! digits (27 chars). SQLite stores them as TEXT and compares them
//! lexicographically, and the `time` crate's default RFC 3339 formatter trims
//! trailing fractional zeros — so `…20.5Z` sorted *after* `…20.51Z`, and a
//! whole-second `…20Z` after everything in its second. That inverted
//! `ORDER BY`s and window comparisons on every store that didn't remember to
//! normalize (tsk243, tsk107, tsk387). Fixing the width **in the serializer**
//! makes the invariant hold by construction: there is no other way to turn a
//! `Timestamp` into text. Parsing accepts any RFC 3339 string (old rows, other
//! producers). Sub-microsecond precision is truncated, which preserves order.
//! Use `Timestamp` rather than reaching for `time::OffsetDateTime` directly.

use ::time::format_description::well_known::Rfc3339;
use ::time::macros::format_description;
use ::time::OffsetDateTime;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use specta::Type;

/// Wall-clock UTC timestamp serialized as a fixed-width RFC 3339 string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Type)]
#[specta(transparent)]
pub struct Timestamp(pub OffsetDateTime);

/// The one text form (`2026-09-29T02:53:51.920814Z`). Length is always
/// [`Timestamp::TEXT_LEN`] for years 0000–9999.
const FIXED: &[::time::format_description::BorrowedFormatItem<'static>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:6]Z");

impl Timestamp {
    /// Length of the fixed-width text form.
    pub const TEXT_LEN: usize = 27;

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

    /// At full precision: what an OTLP point's `time_unix_nano` says. A
    /// value out of range is `None`.
    pub fn from_unix_nanos(nanos: i128) -> Option<Self> {
        OffsetDateTime::from_unix_timestamp_nanos(nanos)
            .ok()
            .map(Self)
    }

    pub fn unix_nanos(&self) -> i128 {
        self.0.unix_timestamp_nanos()
    }

    /// The fixed-width text form: what goes over the wire and into SQLite.
    /// Lexicographic order of these strings is chronological order.
    pub fn to_text(&self) -> String {
        self.0
            .to_offset(::time::UtcOffset::UTC)
            .format(FIXED)
            .expect("fixed-width timestamp formats")
    }

    /// Parse any RFC 3339 string (fixed-width or not, any offset).
    pub fn parse(s: &str) -> Result<Self, ::time::error::Parse> {
        OffsetDateTime::parse(s, &Rfc3339).map(Self)
    }
}

impl std::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_text())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        Timestamp::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// The collectors' and models' duration grammar — `15m`, `2h` — as a
/// duration; anything else (zero included) is `None`. One parser for
/// `trigger: { every: … }` and `materialize: { every: … }`.
pub fn parse_every(text: &str) -> Option<std::time::Duration> {
    let t = text.trim();
    let (n, unit) = match t.strip_suffix('m') {
        Some(n) => (n, 60),
        None => (t.strip_suffix('h')?, 3600),
    };
    let n: u64 = n.trim().parse().ok().filter(|n| *n > 0)?;
    Some(std::time::Duration::from_secs(n * unit))
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

    #[test]
    fn text_is_fixed_width_and_sorts_chronologically() {
        // Whole second, trailing zeros, and a full six digits: all 27 chars.
        let whole = Timestamp::from_unix_ms(1_700_000_000_000);
        let half = Timestamp::from_unix_ms(1_700_000_000_500);
        let fine = Timestamp(
            OffsetDateTime::from_unix_timestamp_nanos(1_700_000_000_500_001_000).unwrap(),
        );
        for t in [whole, half, fine] {
            assert_eq!(t.to_text().len(), Timestamp::TEXT_LEN, "{}", t.to_text());
        }
        assert_eq!(whole.to_text(), "2023-11-14T22:13:20.000000Z");
        assert_eq!(half.to_text(), "2023-11-14T22:13:20.500000Z");
        assert_eq!(fine.to_text(), "2023-11-14T22:13:20.500001Z");
        // The trimmed forms `…20Z` / `…20.5Z` would have sorted after `…20.500001Z`.
        let mut texts = [fine.to_text(), whole.to_text(), half.to_text()];
        texts.sort();
        assert_eq!(texts, [whole.to_text(), half.to_text(), fine.to_text()]);
        // Sub-microsecond precision truncates rather than rounds (order-preserving).
        let almost = Timestamp(
            OffsetDateTime::from_unix_timestamp_nanos(1_700_000_000_500_000_999).unwrap(),
        );
        assert_eq!(almost.to_text(), half.to_text());
        assert_eq!(
            serde_json::to_string(&half).unwrap(),
            "\"2023-11-14T22:13:20.500000Z\""
        );
        assert_eq!(half.to_string(), half.to_text());
    }

    #[test]
    fn every_reads_minutes_and_hours_and_nothing_else() {
        use std::time::Duration;
        assert_eq!(parse_every("15m"), Some(Duration::from_secs(900)));
        assert_eq!(parse_every(" 2h "), Some(Duration::from_secs(7200)));
        for bad in ["0m", "1d", "h", "-1h", "90s", ""] {
            assert_eq!(parse_every(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_trimmed_offset_and_nanosecond_forms() {
        let half = Timestamp::from_unix_ms(1_700_000_000_500);
        for s in [
            "2023-11-14T22:13:20.5Z",
            "2023-11-14T22:13:20.500000Z",
            "2023-11-14T22:13:20.500000000Z",
            "2023-11-14T23:13:20.5+01:00",
        ] {
            assert_eq!(Timestamp::parse(s).unwrap(), half, "{s}");
            let back: Timestamp = serde_json::from_str(&format!("\"{s}\"")).unwrap();
            assert_eq!(back, half);
        }
        assert_eq!(
            Timestamp::parse("2023-11-14T22:13:20Z").unwrap(),
            Timestamp::from_unix_ms(1_700_000_000_000)
        );
        assert!(Timestamp::parse("yesterday").is_err());
        assert!(serde_json::from_str::<Timestamp>("42").is_err());
    }
}
