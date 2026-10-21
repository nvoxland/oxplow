//! The old ref grammar, `<kind>:<id>[@<rev>][#<frag>]`, kept only so
//! the migration can read what was stored in it (`super::v1_to_uri`).
//! Nothing else parses or builds one.

/// A parsed canonical ref. `id` is decoded (an `@` in a file name is a
/// real `@` here); `Display` re-encodes it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Uri {
    pub kind: String,
    pub id: String,
    /// `<rev_kind>:<value>`, e.g. `git:HEAD`; `None` means the working tree.
    pub rev: Option<String>,
    pub frag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum UriParseError {
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

/// The characters an id or revision must percent-encode.
const RESERVED: [char; 3] = ['@', '#', '%'];

fn is_valid_type(kind: &str) -> bool {
    let mut chars = kind.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

impl Uri {
    pub fn parse(text: &str) -> Result<Self, UriParseError> {
        if text.is_empty() {
            return Err(UriParseError::Empty);
        }
        let Some((kind, rest)) = text.split_once(':') else {
            return Err(UriParseError::MissingColon(text.to_string()));
        };
        if kind.is_empty() {
            return Err(UriParseError::EmptyKind);
        }
        if !is_valid_type(kind) {
            return Err(UriParseError::BadKind(kind.to_string()));
        }
        // The fragment is everything after the first `#`; it isn't encoded.
        let (before_frag, frag) = match rest.split_once('#') {
            Some((b, f)) => (b, Some(f)),
            None => (rest, None),
        };
        if frag.is_some_and(str::is_empty) {
            return Err(UriParseError::EmptyFrag);
        }
        let (id_enc, rev_enc) = match before_frag.split_once('@') {
            Some((i, r)) => (i, Some(r)),
            None => (before_frag, None),
        };
        if id_enc.is_empty() {
            return Err(UriParseError::EmptyId);
        }
        let id = decode(id_enc)?;
        let rev = match rev_enc {
            Some("") => return Err(UriParseError::EmptyRev),
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
fn check_rev(rev: &str) -> Result<(), UriParseError> {
    match rev.split_once(':') {
        Some((k, v)) if is_valid_type(k) && !v.is_empty() => Ok(()),
        _ => Err(UriParseError::BadRev(rev.to_string())),
    }
}

/// Decode `%XX` escapes; a raw reserved character is an error.
fn decode(s: &str) -> Result<String, UriParseError> {
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
                        return Err(UriParseError::BadEscape(shown.to_string()));
                    }
                }
            }
            c if RESERVED.contains(&c) => return Err(UriParseError::Unescaped(c, c as u8)),
            c => out.push(c),
        }
    }
    Ok(out)
}

/// An old lens's, page's or metric's id as its head and the params that
/// rode in it after `?` (decoded: `+` a space, `%XX` a byte).
pub(crate) fn view_of(id: &str) -> (String, Vec<(String, String)>) {
    let Some((head, query)) = id.split_once('?') else {
        return (id.to_string(), Vec::new());
    };
    let params = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (query_unescape(key), query_unescape(value))
        })
        .collect();
    (head.to_string(), params)
}

fn query_unescape(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                let hex = |b: u8| (b as char).to_digit(16);
                match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        i += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
