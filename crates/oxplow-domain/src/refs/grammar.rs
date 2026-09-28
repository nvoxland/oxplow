//! The canonical ref grammar (target-architecture §4.1):
//!
//! ```text
//! ref  := kind ":" id [ "@" rev ] [ "#" frag ]
//! kind := [a-z][a-z0-9_]*
//! id   := 1+ chars; `@` `#` `%` percent-encoded; `:` `/` `?` `=` `&` legal raw
//! rev  := rev_kind ":" value          e.g. snap:01J9…, git:HEAD
//! frag := kind-defined                 e.g. L10-20
//! ```
//!
//! A ref names one thing (`effort:eff362`, `work_item:linear:ENG-12`,
//! `file:src/a.rs@git:HEAD#L10-20`). Tabs, links, backlinks and agent
//! context all use this one string identity. Only `@`, `#` and `%` are
//! reserved in an id, so `work_item:oxplow:tsk42` and
//! `lens:acme/blocked?stream_id=2` stay readable; the first `:` ends the
//! kind, the first unescaped `#` ends the id (or rev), and the first
//! unescaped `@` before that starts the rev.
//!
//! Pure: no IO. The golden fixture `tests/fixtures/ref_grammar.json` pins
//! this grammar for both this parser and the TS one.

use std::fmt;

/// A parsed canonical ref. `id` is decoded (an `@` in a file name is a
/// real `@` here); `Display` re-encodes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CanonicalRef {
    pub kind: String,
    pub id: String,
    /// `<rev_kind>:<value>`, e.g. `git:HEAD`; `None` means the working tree.
    pub rev: Option<String>,
    pub frag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefParseError {
    #[error("empty ref")]
    Empty,
    #[error("ref `{0}` has no `:` between kind and id")]
    MissingColon(String),
    #[error("ref has an empty kind")]
    EmptyKind,
    #[error("ref has an empty id")]
    EmptyId,
    #[error("bad kind `{0}`: a kind is lowercase letters, digits and `_`, starting with a letter")]
    BadKind(String),
    #[error("ref has an empty revision after `@`")]
    EmptyRev,
    #[error("bad revision `{0}`: a revision is `<kind>:<value>`, e.g. `git:HEAD` or `snap:01J9…`")]
    BadRev(String),
    #[error("ref has an empty fragment after `#`")]
    EmptyFrag,
    #[error("unescaped `{0}` in ref: encode it as `%{1:02X}` inside the id or revision")]
    Unescaped(char, u8),
    #[error("bad percent escape `{0}`: use two hex digits, e.g. `%40`")]
    BadEscape(String),
}

impl RefParseError {
    /// A stable snake_case name for the fixture to assert on.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Empty => "empty",
            Self::MissingColon(_) => "missing_colon",
            Self::EmptyKind => "empty_kind",
            Self::EmptyId => "empty_id",
            Self::BadKind(_) => "bad_kind",
            Self::EmptyRev => "empty_rev",
            Self::BadRev(_) => "bad_rev",
            Self::EmptyFrag => "empty_frag",
            Self::Unescaped(..) => "unescaped",
            Self::BadEscape(_) => "bad_escape",
        }
    }
}

/// The characters an id or revision must percent-encode.
const RESERVED: [char; 3] = ['@', '#', '%'];

pub fn is_valid_kind(kind: &str) -> bool {
    let mut chars = kind.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl CanonicalRef {
    /// Build from decoded parts, validating the kind and the revision shape.
    pub fn new(
        kind: &str,
        id: &str,
        rev: Option<&str>,
        frag: Option<&str>,
    ) -> Result<Self, RefParseError> {
        if !is_valid_kind(kind) {
            return Err(if kind.is_empty() {
                RefParseError::EmptyKind
            } else {
                RefParseError::BadKind(kind.to_string())
            });
        }
        if id.is_empty() {
            return Err(RefParseError::EmptyId);
        }
        if let Some(r) = rev {
            check_rev(r)?;
        }
        if frag.is_some_and(str::is_empty) {
            return Err(RefParseError::EmptyFrag);
        }
        Ok(Self {
            kind: kind.to_string(),
            id: id.to_string(),
            rev: rev.map(str::to_string),
            frag: frag.map(str::to_string),
        })
    }

    pub fn parse(text: &str) -> Result<Self, RefParseError> {
        if text.is_empty() {
            return Err(RefParseError::Empty);
        }
        let Some((kind, rest)) = text.split_once(':') else {
            return Err(RefParseError::MissingColon(text.to_string()));
        };
        if kind.is_empty() {
            return Err(RefParseError::EmptyKind);
        }
        if !is_valid_kind(kind) {
            return Err(RefParseError::BadKind(kind.to_string()));
        }
        // The fragment is everything after the first `#`; it isn't encoded.
        let (before_frag, frag) = match rest.split_once('#') {
            Some((b, f)) => (b, Some(f)),
            None => (rest, None),
        };
        if frag.is_some_and(str::is_empty) {
            return Err(RefParseError::EmptyFrag);
        }
        let (id_enc, rev_enc) = match before_frag.split_once('@') {
            Some((i, r)) => (i, Some(r)),
            None => (before_frag, None),
        };
        if id_enc.is_empty() {
            return Err(RefParseError::EmptyId);
        }
        let id = decode(id_enc)?;
        let rev = match rev_enc {
            Some("") => return Err(RefParseError::EmptyRev),
            Some(r) => {
                let r = decode(r)?;
                check_rev(&r)?;
                Some(r)
            }
            None => None,
        };
        Ok(Self {
            kind: kind.to_string(),
            id,
            rev,
            frag: frag.map(str::to_string),
        })
    }
}

/// A revision is `<kind>:<value>` with a valid kind, so a bare `HEAD` is
/// refused: the reader must know which system's revision it is.
fn check_rev(rev: &str) -> Result<(), RefParseError> {
    match rev.split_once(':') {
        Some((k, v)) if is_valid_kind(k) && !v.is_empty() => Ok(()),
        _ => Err(RefParseError::BadRev(rev.to_string())),
    }
}

/// Decode `%XX` escapes; a raw reserved character is an error.
fn decode(s: &str) -> Result<String, RefParseError> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '%' => {
                let hex = s.get(i + 1..i + 3).filter(|h| h.len() == 2);
                let byte = hex.and_then(|h| u8::from_str_radix(h, 16).ok());
                match byte {
                    Some(b) => {
                        out.push(b as char);
                        chars.next();
                        chars.next();
                    }
                    None => {
                        let shown = s.get(i..(i + 3).min(s.len())).unwrap_or("%");
                        return Err(RefParseError::BadEscape(shown.to_string()));
                    }
                }
            }
            c if RESERVED.contains(&c) => return Err(RefParseError::Unescaped(c, c as u8)),
            c => out.push(c),
        }
    }
    Ok(out)
}

fn encode(s: &str, out: &mut String) {
    for c in s.chars() {
        if RESERVED.contains(&c) {
            out.push_str(&format!("%{:02X}", c as u8));
        } else {
            out.push(c);
        }
    }
}

impl fmt::Display for CanonicalRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::with_capacity(self.kind.len() + self.id.len() + 8);
        s.push_str(&self.kind);
        s.push(':');
        encode(&self.id, &mut s);
        if let Some(rev) = &self.rev {
            s.push('@');
            encode(rev, &mut s);
        }
        if let Some(frag) = &self.frag {
            s.push('#');
            s.push_str(frag);
        }
        f.write_str(&s)
    }
}

impl std::str::FromStr for CanonicalRef {
    type Err = RefParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}
