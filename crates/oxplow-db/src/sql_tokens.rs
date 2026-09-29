//! The one SQL tokenizer (P4.1): what the semantic layer needs to know
//! about a query's text without SQLite's parser — whether it is a single
//! `SELECT`/`WITH`, and where the layer's own calls sit (`ref()` and
//! `source()` in a model file, `metric_grid()` and `MEASURE()` in a
//! query). It knows strings, quoted identifiers, comments and parameters,
//! so none of those can be mistaken for a keyword or a call. It does not
//! parse: SQLite's `prepare` is the judge of everything else.

use oxplow_domain::DomainError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Space,
    /// `-- …` to the end of the line, or `/* … */`.
    Comment,
    /// `'…'` (and a blob `x'…'`).
    Str,
    /// `"…"`, `` `…` `` or `[…]`.
    QuotedIdent,
    /// A keyword or bare identifier.
    Word,
    Number,
    /// `?`, `?N`, `:name`, `@name` or `$name`.
    Param,
    /// Any other single character (`(`, `,`, `;`, `.`, operators).
    Punct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token<'a> {
    pub kind: TokenKind,
    pub text: &'a str,
    /// Byte offset in the SQL.
    pub start: usize,
}

impl Token<'_> {
    pub fn end(&self) -> usize {
        self.start + self.text.len()
    }

    fn significant(&self) -> bool {
        !matches!(self.kind, TokenKind::Space | TokenKind::Comment)
    }

    /// A word equal to `kw`, ignoring ASCII case.
    pub fn is_word(&self, kw: &str) -> bool {
        self.kind == TokenKind::Word && self.text.eq_ignore_ascii_case(kw)
    }

    pub fn is_punct(&self, c: char) -> bool {
        self.kind == TokenKind::Punct && self.text.starts_with(c) && self.text.len() == c.len_utf8()
    }
}

fn invalid(msg: String) -> DomainError {
    DomainError::Invalid(msg)
}

/// Split `sql` into tokens. An unterminated string or quoted identifier
/// is an error (an unterminated `/*` runs to the end, as in SQLite).
pub fn tokenize(sql: &str) -> Result<Vec<Token<'_>>, DomainError> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let c = bytes[i];
        let kind = match c {
            b' ' | b'\t' | b'\n' | b'\r' | 0x0c => {
                while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0c) {
                    i += 1;
                }
                TokenKind::Space
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                TokenKind::Comment
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = sql[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |p| i + 2 + p + 2);
                TokenKind::Comment
            }
            b'\'' => {
                i = close_quoted(sql, i, b'\'')?;
                TokenKind::Str
            }
            b'x' | b'X' if bytes.get(i + 1) == Some(&b'\'') => {
                i = close_quoted(sql, i + 1, b'\'')?;
                TokenKind::Str
            }
            b'"' | b'`' => {
                i = close_quoted(sql, i, c)?;
                TokenKind::QuotedIdent
            }
            b'[' => {
                i = sql[i..]
                    .find(']')
                    .map(|p| i + p + 1)
                    .ok_or_else(|| invalid(format!("unterminated `[` at byte {start}")))?;
                TokenKind::QuotedIdent
            }
            b'?' => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                TokenKind::Param
            }
            b':' | b'@' | b'$' if bytes.get(i + 1).is_some_and(|b| is_ident_start(*b)) => {
                i += 1;
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                TokenKind::Param
            }
            b'0'..=b'9' => {
                i += 1;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric()
                        || bytes[i] == b'.'
                        || ((bytes[i] == b'+' || bytes[i] == b'-')
                            && matches!(bytes[i - 1], b'e' | b'E')))
                {
                    i += 1;
                }
                TokenKind::Number
            }
            b'.' if bytes.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.') {
                    i += 1;
                }
                TokenKind::Number
            }
            c if is_ident_start(c) => {
                while i < bytes.len() && is_ident_char(bytes[i]) {
                    i += 1;
                }
                TokenKind::Word
            }
            _ => {
                // One character (a multi-byte one whole).
                i += sql[i..].chars().next().map_or(1, char::len_utf8);
                TokenKind::Punct
            }
        };
        out.push(Token {
            kind,
            text: &sql[start..i],
            start,
        });
    }
    Ok(out)
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn is_ident_char(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit() || b == b'$'
}

/// The index just past the quote closing the one at `open`; a doubled
/// quote is an escaped one.
fn close_quoted(sql: &str, open: usize, q: u8) -> Result<usize, DomainError> {
    let bytes = sql.as_bytes();
    let mut i = open + 1;
    while i < bytes.len() {
        if bytes[i] == q {
            if bytes.get(i + 1) == Some(&q) {
                i += 2;
                continue;
            }
            return Ok(i + 1);
        }
        i += 1;
    }
    Err(invalid(format!(
        "unterminated {} at byte {open}",
        if q == b'\'' {
            "string"
        } else {
            "quoted identifier"
        }
    )))
}

/// The tokens that mean something (no whitespace or comments).
pub fn significant<'a>(tokens: &'a [Token<'a>]) -> impl Iterator<Item = &'a Token<'a>> {
    tokens.iter().filter(|t| t.significant())
}

/// `sql` is exactly one `SELECT` or `WITH` statement (an optional trailing
/// `;` aside) — the layer's read contract, checked before SQLite sees it.
pub fn check_single_read(sql: &str) -> Result<(), DomainError> {
    let tokens = tokenize(sql)?;
    let mut sig = significant(&tokens);
    match sig.next() {
        Some(t) if t.is_word("SELECT") || t.is_word("WITH") => {}
        _ => {
            return Err(invalid(
                "query_sql accepts a single SELECT or WITH statement".into(),
            ))
        }
    }
    let rest: Vec<&Token<'_>> = sig.collect();
    if let Some(semi) = rest.iter().position(|t| t.is_punct(';')) {
        if rest[semi + 1..].iter().any(|t| !t.is_punct(';')) {
            return Err(invalid(
                "query_sql accepts a single statement; found more after `;`".into(),
            ));
        }
    }
    Ok(())
}

/// One call of a function the layer rewrites, e.g. `ref('task')`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// Byte range of the whole call, name through `)`.
    pub start: usize,
    pub end: usize,
    /// Each argument's text, trimmed of whitespace and comments.
    pub args: Vec<String>,
}

/// Every call of `name(…)` (case-insensitive, a bare word followed by `(`,
/// not a qualified `x.name(`), with its top-level arguments.
pub fn calls(sql: &str, name: &str) -> Result<Vec<Call>, DomainError> {
    let tokens = tokenize(sql)?;
    let sig: Vec<&Token<'_>> = significant(&tokens).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < sig.len() {
        let qualified = i > 0 && sig[i - 1].is_punct('.');
        if sig[i].is_word(name) && !qualified && sig.get(i + 1).is_some_and(|t| t.is_punct('(')) {
            let start = sig[i].start;
            let mut depth = 0usize;
            let mut args: Vec<String> = Vec::new();
            let mut arg_start: Option<usize> = None;
            let mut j = i + 1;
            let mut end = None;
            while j < sig.len() {
                let t = sig[j];
                if t.is_punct('(') {
                    depth += 1;
                    if depth == 1 {
                        j += 1;
                        continue;
                    }
                } else if t.is_punct(')') {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = arg_start {
                            args.push(sql[s..sig[j - 1].end()].to_string());
                        }
                        end = Some(t.end());
                        break;
                    }
                } else if t.is_punct(',') && depth == 1 {
                    if let Some(s) = arg_start.take() {
                        args.push(sql[s..sig[j - 1].end()].to_string());
                    }
                    j += 1;
                    continue;
                }
                if arg_start.is_none() {
                    arg_start = Some(t.start);
                }
                j += 1;
            }
            let end = end.ok_or_else(|| {
                let (line, col) = line_col(sql, start);
                invalid(format!("unclosed `{name}(` at {line}:{col}"))
            })?;
            out.push(Call { start, end, args });
            i = j + 1;
            continue;
        }
        i += 1;
    }
    Ok(out)
}

/// A string literal's value (`'it''s'` → `it's`), or `None` for anything
/// that isn't one plain `'…'` literal.
pub fn string_literal(text: &str) -> Option<String> {
    let tokens = tokenize(text).ok()?;
    let sig: Vec<&Token<'_>> = significant(&tokens).collect();
    match sig.as_slice() {
        [t] if t.kind == TokenKind::Str && t.text.starts_with('\'') => {
            Some(t.text[1..t.text.len() - 1].replace("''", "'"))
        }
        _ => None,
    }
}

/// 1-based line and column of byte `offset` in `sql`.
pub fn line_col(sql: &str, offset: usize) -> (usize, usize) {
    let before = &sql[..offset.min(sql.len())];
    let line = before.matches('\n').count() + 1;
    let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, col)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(sql: &str) -> Vec<(TokenKind, &str)> {
        tokenize(sql)
            .unwrap()
            .into_iter()
            .filter(|t| t.kind != TokenKind::Space)
            .map(|t| (t.kind, t.text))
            .collect()
    }

    #[test]
    fn strings_comments_quotes_and_params_are_their_own_tokens() {
        use TokenKind::*;
        assert_eq!(
            kinds("SELECT 'it''s ;', \"a b\", [c], `d` -- ref(x)\n/* ; */ FROM t WHERE x = :id AND y = ?2 AND z = 1.5e-3"),
            vec![
                (Word, "SELECT"),
                (Str, "'it''s ;'"),
                (Punct, ","),
                (QuotedIdent, "\"a b\""),
                (Punct, ","),
                (QuotedIdent, "[c]"),
                (Punct, ","),
                (QuotedIdent, "`d`"),
                (Comment, "-- ref(x)"),
                (Comment, "/* ; */"),
                (Word, "FROM"),
                (Word, "t"),
                (Word, "WHERE"),
                (Word, "x"),
                (Punct, "="),
                (Param, ":id"),
                (Word, "AND"),
                (Word, "y"),
                (Punct, "="),
                (Param, "?2"),
                (Word, "AND"),
                (Word, "z"),
                (Punct, "="),
                (Number, "1.5e-3"),
            ]
        );
        assert!(tokenize("SELECT 'open").is_err());
        assert!(tokenize("SELECT \"open").is_err());
    }

    #[test]
    fn a_read_is_one_select_or_with() {
        for ok in [
            "SELECT 1",
            "  -- note\n with x AS (SELECT 1) SELECT * FROM x;",
            "/* c */ select 1 ;  ",
            "SELECT ';DELETE FROM t'",
        ] {
            assert!(check_single_read(ok).is_ok(), "{ok}");
        }
        for bad in [
            "DELETE FROM t",
            "SELECT 1; DELETE FROM t",
            "PRAGMA table_info(t)",
            "",
            "-- only a comment",
        ] {
            assert!(check_single_read(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn calls_are_found_with_their_arguments_outside_strings_and_comments() {
        let sql = "SELECT * FROM ref('task') t\n  JOIN REF( 'effort' ) e ON e.id = t.x\n  -- ref('not')\n  WHERE t.s = 'ref(''no'')' AND s.ref('q') AND f(ref('a', g(1, 2)))";
        let found = calls(sql, "ref").unwrap();
        let args: Vec<Vec<String>> = found.iter().map(|c| c.args.clone()).collect();
        assert_eq!(
            args,
            vec![
                vec!["'task'".to_string()],
                vec!["'effort'".to_string()],
                vec!["'a'".to_string(), "g(1, 2)".to_string()],
            ]
        );
        assert_eq!(&sql[found[0].start..found[0].end], "ref('task')");
        assert_eq!(string_literal("'it''s'").as_deref(), Some("it's"));
        assert_eq!(string_literal("x"), None);
        let (line, col) = line_col(sql, found[1].start);
        assert_eq!((line, col), (2, 8));
        assert!(calls("SELECT ref('a'", "ref").is_err());
    }
}
