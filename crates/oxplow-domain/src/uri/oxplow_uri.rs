//! oxplow's URIs (`.context/resources.md`): one scheme, the resource as
//! `<extension>/<name>`, its identity as named params, and the
//! renderer's view as named params after `#`:
//!
//! ```text
//! uri  := "oxplow:" extension "/" name [ "?" params ] [ "#" view ]
//! params, view := key "=" value ( "&" key "=" value )*
//! ```
//!
//! `oxplow:oxplow/file?path=src/a.rs&rev=git:HEAD#lines=10-20`. Keys are
//! unique and their order doesn't matter: the canonical form sorts them,
//! so `?id=4&group=5` and `?group=5&id=4` are one resource. The view is
//! never part of the identity ([`OxplowUri::identity`]).
//!
//! The string is split by hand rather than by a URL library: the URL
//! standard reads `+` as a space in a query and normalizes paths, which
//! would make the Rust and TS parsers disagree. The golden fixture
//! `tests/fixtures/uri.json` pins the grammar for both.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

/// The scheme every oxplow URI has.
pub const SCHEME: &str = "oxplow";

/// What a URI names: `<extension>/<name>` (`oxplow/file`, `acme/pr`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceName {
    extension: String,
    name: String,
}

impl ResourceName {
    pub fn new(extension: &str, name: &str) -> Result<Self, UriParseError> {
        if !is_extension(extension) || !is_name(name) {
            return Err(UriParseError::BadResource(format!("{extension}/{name}")));
        }
        Ok(Self {
            extension: extension.to_string(),
            name: name.to_string(),
        })
    }

    pub fn extension(&self) -> &str {
        &self.extension
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Display for ResourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.extension, self.name)
    }
}

impl FromStr for ResourceName {
    type Err = UriParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (extension, name) = s
            .split_once('/')
            .ok_or_else(|| UriParseError::BadResource(s.to_string()))?;
        Self::new(extension, name)
    }
}

/// An extension's name: lowercase letters, digits and `-`, starting with a
/// letter (`oxplow`, `acme-tools`).
fn is_extension(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A resource's name within its extension: lowercase letters, digits and
/// `_`, starting with a letter (`file`, `work_item`).
fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// A param's key: a letter, then letters, digits, `_`, `.` and `-`
/// (`id`, `stream_id`, `f.state`, `oxplow.renderer`, `newThreadSession`).
/// Never escaped.
fn is_key(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// A byte a canonical key or value carries as itself.
fn is_raw(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b':'
                | b'?'
                | b','
                | b'@'
                | b'!'
                | b'$'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b';'
        )
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UriParseError {
    #[error("empty URI")]
    Empty,
    #[error("`{0}` isn't an oxplow URI: it starts `oxplow:<extension>/<resource>`")]
    NotOxplow(String),
    #[error(
        "`{0}` has an authority (`oxplow://`): an oxplow URI is `oxplow:<extension>/<resource>`"
    )]
    Authority(String),
    #[error("bad resource `{0}`: `<extension>/<resource>`, an extension's name (`a-z0-9-`) and the resource's (`a-z0-9_`)")]
    BadResource(String),
    #[error("bad param `{0}`: each is `<key>=<value>`, the key a letter then letters, digits, `_`, `.` or `-`")]
    BadParam(String),
    #[error("param `{0}` is given twice")]
    DuplicateParam(String),
    #[error("bad percent escape in `{0}`: use two hex digits, e.g. `%26`")]
    BadEscape(String),
    #[error("`{0}` decodes to bytes that aren't UTF-8")]
    NotUtf8(String),
    #[error("`{0}` holds a character that must be percent-encoded (`%20` for a space)")]
    Unescaped(String),
}

impl UriParseError {
    /// A stable snake_case name for the fixture to assert on.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::NotOxplow(_) => "not_oxplow",
            Self::Authority(_) => "authority",
            Self::BadResource(_) => "bad_resource",
            Self::BadParam(_) => "bad_param",
            Self::DuplicateParam(_) => "duplicate_param",
            Self::BadEscape(_) => "bad_escape",
            Self::NotUtf8(_) => "not_utf8",
            Self::Unescaped(_) => "unescaped",
        }
    }
}

/// One of oxplow's URIs, parsed: always valid and formatted canonically.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OxplowUri {
    resource: ResourceName,
    params: BTreeMap<String, String>,
    view: BTreeMap<String, String>,
}

impl OxplowUri {
    /// `resource` with `params` and `view` (decoded values; their keys
    /// must be keys — a builder passes literals).
    pub fn new(
        resource: ResourceName,
        params: BTreeMap<String, String>,
        view: BTreeMap<String, String>,
    ) -> Self {
        for key in params.keys().chain(view.keys()) {
            assert!(is_key(key), "`{key}` isn't a URI param key");
        }
        Self {
            resource,
            params,
            view,
        }
    }

    pub fn parse(text: &str) -> Result<Self, UriParseError> {
        if text.is_empty() {
            return Err(UriParseError::Empty);
        }
        let rest = text
            .strip_prefix("oxplow:")
            .ok_or_else(|| UriParseError::NotOxplow(text.to_string()))?;
        if rest.starts_with("//") {
            return Err(UriParseError::Authority(text.to_string()));
        }
        let (before_view, view) = match rest.split_once('#') {
            Some((head, view)) => (head, view),
            None => (rest, ""),
        };
        let (path, query) = match before_view.split_once('?') {
            Some((path, query)) => (path, query),
            None => (before_view, ""),
        };
        let resource: ResourceName = path.parse()?;
        Ok(Self {
            resource,
            params: pairs(query)?,
            view: pairs(view)?,
        })
    }

    pub fn resource(&self) -> &ResourceName {
        &self.resource
    }

    /// The params that say which resource it is (decoded).
    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }

    /// How a renderer shows it (decoded): never part of its identity.
    pub fn view(&self) -> &BTreeMap<String, String> {
        &self.view
    }

    /// The same URI showing `view`.
    pub fn with_view(&self, view: BTreeMap<String, String>) -> Self {
        Self::new(self.resource.clone(), self.params.clone(), view)
    }

    /// The canonical URI without its view: two URIs name the same
    /// resource exactly when their identities are equal.
    pub fn identity(&self) -> String {
        let mut out = format!("{SCHEME}:{}", self.resource);
        write_pairs(&mut out, '?', &self.params);
        out
    }
}

impl fmt::Display for OxplowUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = self.identity();
        write_pairs(&mut out, '#', &self.view);
        f.write_str(&out)
    }
}

impl FromStr for OxplowUri {
    type Err = UriParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl serde::Serialize for OxplowUri {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for OxplowUri {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// `k=v&k=v` decoded, refusing a duplicate key; empty is none.
fn pairs(text: &str) -> Result<BTreeMap<String, String>, UriParseError> {
    let mut out = BTreeMap::new();
    if text.is_empty() {
        return Ok(out);
    }
    for pair in text.split('&') {
        let (key, value) = pair
            .split_once('=')
            .filter(|(key, _)| is_key(key))
            .ok_or_else(|| UriParseError::BadParam(pair.to_string()))?;
        let value = decode(value)?;
        if out.insert(key.to_string(), value).is_some() {
            return Err(UriParseError::DuplicateParam(key.to_string()));
        }
    }
    Ok(out)
}

/// A value's `%XX` escapes decoded; a raw byte must be one a canonical
/// value carries, or `+` (a plus, never a space).
fn decode(value: &str) -> Result<String, UriParseError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .ok_or_else(|| UriParseError::BadEscape(value.to_string()))?;
                out.push(hex);
                i += 3;
            }
            b if is_raw(b) || b == b'+' => {
                out.push(b);
                i += 1;
            }
            _ => return Err(UriParseError::Unescaped(value.to_string())),
        }
    }
    String::from_utf8(out).map_err(|_| UriParseError::NotUtf8(value.to_string()))
}

fn encode(value: &str, out: &mut String) {
    for &b in value.as_bytes() {
        if is_raw(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
}

fn write_pairs(out: &mut String, lead: char, pairs: &BTreeMap<String, String>) {
    for (i, (key, value)) in pairs.iter().enumerate() {
        out.push(if i == 0 { lead } else { '&' });
        out.push_str(key);
        out.push('=');
        encode(value, out);
    }
}

/// Any URI a link may name: one of oxplow's, or anything else a URL
/// names (`https:`, `mailto:`, `file:`), which oxplow hands to its
/// renderer for that scheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnyUri {
    Oxplow(OxplowUri),
    External(url::Url),
}

impl AnyUri {
    /// An `oxplow:` string must be a valid oxplow URI; anything else must
    /// be an absolute URL.
    pub fn parse(text: &str) -> Result<Self, UriParseError> {
        if text.starts_with("oxplow:") {
            return OxplowUri::parse(text).map(AnyUri::Oxplow);
        }
        url::Url::parse(text)
            .map(AnyUri::External)
            .map_err(|_| UriParseError::NotOxplow(text.to_string()))
    }
}

impl fmt::Display for AnyUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AnyUri::Oxplow(u) => u.fmt(f),
            AnyUri::External(u) => u.fmt(f),
        }
    }
}
