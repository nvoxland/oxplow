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

/// The value a condition compares a column with: a string literal's value,
/// a parameter as written (`:name`, `?2`), or a concatenation of such
/// parts (`:start || '..' || :end`), which is one value built in SQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Text(String),
    Param(String),
    Concat(Vec<Operand>),
}

/// [`equality_filters`] with each value as text: a literal's, or the text
/// `params` binds its parameter to. A condition whose parameter is bound to
/// something else, or to nothing, is left out: it narrows nothing.
pub fn text_filters(
    sql: &str,
    view: &str,
    columns: &[&str],
    params: &crate::SqlParams,
) -> Result<Vec<(String, String)>, DomainError> {
    use crate::{SqlCell, SqlParams};
    fn value(operand: Operand, params: &SqlParams) -> Option<String> {
        let cell = match operand {
            Operand::Text(t) => return Some(t),
            // Every part a text, or the whole is none (a NULL part makes
            // the SQL value NULL, which no row's text equals).
            Operand::Concat(parts) => {
                return parts
                    .into_iter()
                    .map(|p| value(p, params))
                    .collect::<Option<Vec<_>>>()
                    .map(|v| v.concat())
            }
            Operand::Param(p) => match params {
                SqlParams::Positional(values) => {
                    let n: usize = p.strip_prefix('?')?.parse().ok()?;
                    values.get(n.checked_sub(1)?)?
                }
                SqlParams::Named(values) => {
                    let bare = p.trim_start_matches([':', '@', '$']);
                    &values.iter().find(|(n, _)| n == bare)?.1
                }
            },
        };
        match cell {
            SqlCell::Text(t) => Some(t.clone()),
            _ => None,
        }
    }
    Ok(equality_filters(sql, view, columns)?
        .into_iter()
        .filter_map(|(column, operand)| Some((column, value(operand, params)?)))
        .collect())
}

/// The `<column> = <value>` conditions every row `sql` reads of `view` must
/// meet, for the `columns` asked about (the name as `columns` spells it) —
/// so a reader of `view` may hand the query only the rows that meet them.
/// It says so only when it is sure: `sql` is one plain `SELECT` reading
/// `view` once, as its only source (`FROM view [[AS] alias] WHERE …`), and
/// each condition is a top-level conjunct of exactly `[q.]column = value`
/// (either way round; `value` a string literal, a parameter, or parts of
/// those joined by `||`). An `OR`,
/// `BETWEEN` or `CASE` in the `WHERE`, a join, a compound, a `WITH` or a
/// second read of `view` gives none.
pub fn equality_filters(
    sql: &str,
    view: &str,
    columns: &[&str],
) -> Result<Vec<(String, Operand)>, DomainError> {
    let tokens = tokenize(sql)?;
    let sig: Vec<&Token<'_>> = significant(&tokens).collect();
    let names_view = |t: &Token<'_>| match t.kind {
        TokenKind::Word => t.text.eq_ignore_ascii_case(view),
        TokenKind::QuotedIdent => t.text[1..t.text.len() - 1].eq_ignore_ascii_case(view),
        _ => false,
    };
    // Depth of each token: 0 outside every parenthesis.
    let mut depths = Vec::with_capacity(sig.len());
    let mut depth = 0usize;
    for t in &sig {
        if t.is_punct(')') {
            depth = depth.saturating_sub(1);
        }
        depths.push(depth);
        if t.is_punct('(') {
            depth += 1;
        }
    }
    let top = |i: usize, kw: &str| depths[i] == 0 && sig[i].is_word(kw);
    if !sig.first().is_some_and(|t| t.is_word("SELECT"))
        || (0..sig.len())
            .filter(|&j| names_view(sig[j]) && !sig.get(j + 1).is_some_and(|n| n.is_punct('.')))
            .count()
            != 1
        || (0..sig.len()).any(|i| top(i, "UNION") || top(i, "INTERSECT") || top(i, "EXCEPT"))
    {
        return Ok(Vec::new());
    }
    let Some(at) = sig.iter().position(|t| names_view(t)) else {
        return Ok(Vec::new());
    };
    if depths[at] != 0 || at == 0 || !top(at - 1, "FROM") {
        return Ok(Vec::new());
    }
    let (alias, mut i) = match sig.get(at + 1) {
        Some(t) if t.is_word("AS") => (sig.get(at + 2).map(|a| a.text), at + 3),
        Some(t) if t.kind == TokenKind::Word && !t.is_word("WHERE") => (Some(t.text), at + 2),
        _ => (None, at + 1),
    };
    if !sig.get(i).is_some_and(|t| t.is_word("WHERE")) {
        return Ok(Vec::new());
    }
    i += 1;
    let end = (i..sig.len())
        .find(|&j| {
            depths[j] == 0
                && (["GROUP", "ORDER", "LIMIT", "HAVING", "WINDOW"]
                    .iter()
                    .any(|kw| sig[j].is_word(kw))
                    || sig[j].is_punct(';'))
        })
        .unwrap_or(sig.len());
    if (i..end).any(|j| top(j, "OR") || top(j, "BETWEEN") || top(j, "CASE")) {
        return Ok(Vec::new());
    }
    let column = |c: &[&&Token<'_>]| -> Option<String> {
        let (qualifier, name) = match c {
            [name] => (None, name),
            [q, dot, name] if dot.is_punct('.') => (Some(q.text), name),
            _ => return None,
        };
        if name.kind != TokenKind::Word {
            return None;
        }
        if let Some(q) = qualifier {
            let ok =
                q.eq_ignore_ascii_case(view) || alias.is_some_and(|a| q.eq_ignore_ascii_case(a));
            if !ok {
                return None;
            }
        }
        columns
            .iter()
            .find(|c| c.eq_ignore_ascii_case(name.text))
            .map(|c| c.to_string())
    };
    let operand = |t: &Token<'_>| match t.kind {
        TokenKind::Str => string_literal(t.text).map(Operand::Text),
        TokenKind::Param => Some(Operand::Param(t.text.to_string())),
        _ => None,
    };
    // One operand, or operands joined by `||` (two `|` tokens): `a || b`.
    let value = |v: &[&&Token<'_>]| -> Option<Operand> {
        let mut parts = Vec::new();
        let mut rest = v;
        loop {
            let (first, after) = rest.split_first()?;
            parts.push(operand(first)?);
            match after {
                [] => break,
                [a, b, more @ ..] if a.is_punct('|') && b.is_punct('|') => rest = more,
                _ => return None,
            }
        }
        Some(match parts.len() {
            1 => parts.pop()?,
            _ => Operand::Concat(parts),
        })
    };
    let mut found = Vec::new();
    let mut start = i;
    for j in i..=end {
        if j < end && !top(j, "AND") {
            continue;
        }
        let conjunct: Vec<&&Token<'_>> = sig[start..j].iter().collect();
        start = j + 1;
        let Some(eq) = conjunct.iter().position(|t| t.is_punct('=')) else {
            continue;
        };
        let (left, right) = (&conjunct[..eq], &conjunct[eq + 1..]);
        let pair = column(left)
            .zip(value(right))
            .or_else(|| column(right).zip(value(left)));
        found.extend(pair);
    }
    Ok(found)
}

/// The names of the common table expressions `sql` defines, at any depth
/// (`WITH a AS (…)`, `b(x) AS MATERIALIZED (…)`), lowercased and unquoted.
/// SQLite names a CTE — not the view around it — as the accessor of the
/// reads inside it, so the authorizer uses these to tell the statement's
/// own reads from a view's.
pub fn cte_names(sql: &str) -> Result<std::collections::BTreeSet<String>, DomainError> {
    let tokens = tokenize(sql)?;
    let sig: Vec<&Token<'_>> = significant(&tokens).collect();
    let mut out = std::collections::BTreeSet::new();
    for i in 0..sig.len() {
        let name = match sig[i].kind {
            TokenKind::Word => sig[i].text.to_string(),
            TokenKind::QuotedIdent => {
                let t = sig[i].text;
                t[1..t.len() - 1].replace("\"\"", "\"")
            }
            _ => continue,
        };
        let mut j = i + 1;
        // An optional column list.
        if sig.get(j).is_some_and(|t| t.is_punct('(')) {
            let mut depth = 0;
            while let Some(t) = sig.get(j) {
                if t.is_punct('(') {
                    depth += 1;
                } else if t.is_punct(')') {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                j += 1;
            }
            j += 1;
        }
        if !sig.get(j).is_some_and(|t| t.is_word("AS")) {
            continue;
        }
        j += 1;
        while sig
            .get(j)
            .is_some_and(|t| t.is_word("NOT") || t.is_word("MATERIALIZED"))
        {
            j += 1;
        }
        if sig.get(j).is_some_and(|t| t.is_punct('(')) {
            out.insert(name.to_lowercase());
        }
    }
    Ok(out)
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

    #[test]
    fn cte_names_are_the_statements_own() {
        let names = cte_names(
            "WITH a AS (SELECT 1), \"B c\"(x, y) AS MATERIALIZED (SELECT 2, 3),
             d AS NOT MATERIALIZED (SELECT 4)
             SELECT CAST(x AS INTEGER) AS n FROM a, (WITH inner_one AS (SELECT 5) SELECT * FROM inner_one)",
        )
        .unwrap();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            vec!["a", "b c", "d", "inner_one"]
        );
        assert!(cte_names("SELECT x AS y FROM t").unwrap().is_empty());
    }

    fn filters(sql: &str) -> Vec<(String, Operand)> {
        equality_filters(sql, "v_workspace_file", &["parent", "path"]).unwrap()
    }

    fn text(col: &str, v: &str) -> (String, Operand) {
        (col.to_string(), Operand::Text(v.to_string()))
    }

    #[test]
    fn a_top_level_equality_on_the_one_source_is_a_filter() {
        assert_eq!(
            filters("SELECT * FROM v_workspace_file WHERE parent = 'src'"),
            vec![text("parent", "src")]
        );
        // Either side, qualified by the view or its alias, a parameter, any
        // case, beside conditions on other columns and before what follows.
        assert_eq!(
            filters(
                "select name from V_WORKSPACE_FILE f where f.stream_id = :stream_id \
                 and :p = f.Parent and kind <> 'dir' order by name limit 5"
            ),
            vec![("parent".to_string(), Operand::Param(":p".to_string()))]
        );
        assert_eq!(
            filters("SELECT 1 FROM v_workspace_file WHERE v_workspace_file.path = ?2"),
            vec![("path".to_string(), Operand::Param("?2".to_string()))]
        );
        assert_eq!(
            filters("SELECT 1 FROM v_workspace_file AS w WHERE w.path = 'x'"),
            vec![text("path", "x")]
        );
        // A subquery in a condition is that condition's own business.
        assert_eq!(
            filters(
                "SELECT 1 FROM v_workspace_file WHERE parent = 'a' \
                 AND path IN (SELECT path FROM v_change_file WHERE status = 'x' OR 1)"
            ),
            vec![text("parent", "a")]
        );
        assert_eq!(
            filters("SELECT 1 FROM v_workspace_file WHERE parent = 'it''s'"),
            vec![text("parent", "it's")]
        );
    }

    /// Only a condition every row read must meet narrows the read: an `OR`,
    /// a `BETWEEN` or `CASE` whose `AND` isn't a conjunction, a join, a
    /// second read of the view, a compound or a `WITH`, a comparison that
    /// isn't a plain `=` — each reads everything.
    #[test]
    fn anything_but_a_plain_conjunction_on_the_one_source_reads_everything() {
        for sql in [
            "SELECT * FROM v_workspace_file",
            "SELECT * FROM v_workspace_file WHERE parent = 'a' OR parent = 'b'",
            "SELECT * FROM v_workspace_file WHERE name BETWEEN 'a' AND parent = 'b'",
            "SELECT * FROM v_workspace_file WHERE CASE WHEN 1 THEN 1 END AND parent = 'b'",
            "SELECT * FROM v_workspace_file f JOIN v_change_file c ON c.path = f.path WHERE f.parent = 'a'",
            "SELECT * FROM v_workspace_file, v_stream WHERE parent = 'a'",
            "SELECT * FROM v_stream WHERE id IN (SELECT stream_id FROM v_workspace_file WHERE parent = 'a')",
            "SELECT * FROM v_workspace_file WHERE parent = 'a' UNION SELECT * FROM v_workspace_file",
            "SELECT (SELECT count(*) FROM v_workspace_file) FROM v_workspace_file WHERE parent = 'a'",
            "WITH x AS (SELECT 1) SELECT * FROM v_workspace_file WHERE parent = 'a'",
            "SELECT * FROM v_workspace_file WHERE NOT parent = 'a'",
            "SELECT * FROM v_workspace_file WHERE parent = 'a' COLLATE NOCASE",
            "SELECT * FROM v_workspace_file WHERE parent = lower('A')",
            "SELECT * FROM v_workspace_file WHERE other.parent = 'a'",
        ] {
            assert_eq!(filters(sql), vec![], "{sql}");
        }
        // A column it wasn't asked about isn't one.
        assert_eq!(
            filters("SELECT * FROM v_workspace_file WHERE kind = 'dir'"),
            vec![]
        );
    }

    /// A filter's value is its literal, or the text bound to its parameter;
    /// one bound to something else, or to nothing, filters nothing.
    #[test]
    fn a_filters_value_is_its_literal_or_the_text_its_parameter_is_bound_to() {
        use crate::{SqlCell, SqlParams};
        let sql = "SELECT 1 FROM v_change WHERE kind = 'commit' AND target = :sha AND n = :n";
        let named = SqlParams::Named(vec![
            ("sha".into(), SqlCell::Text("abc".into())),
            ("n".into(), SqlCell::Int(3)),
        ]);
        assert_eq!(
            text_filters(sql, "v_change", &["kind", "target", "n"], &named).unwrap(),
            vec![
                ("kind".to_string(), "commit".to_string()),
                ("target".to_string(), "abc".to_string())
            ]
        );
        let positional = SqlParams::Positional(vec![SqlCell::Text("def".into())]);
        assert_eq!(
            text_filters(
                "SELECT 1 FROM v_commit WHERE sha = ?1",
                "v_commit",
                &["sha"],
                &positional
            )
            .unwrap(),
            vec![("sha".to_string(), "def".to_string())]
        );
        assert!(text_filters(
            "SELECT 1 FROM v_commit WHERE sha = ?2",
            "v_commit",
            &["sha"],
            &positional
        )
        .unwrap()
        .is_empty());
    }

    /// A value may be built in SQL: `a || b || …`, each part a literal or
    /// a parameter, is one value (a page names a range of revisions as
    /// `:start || '..' || :end`), its text the parts' joined. A part that
    /// is neither (a column, a call), or a parameter bound to nothing,
    /// makes the condition narrow nothing.
    #[test]
    fn a_concatenation_of_literals_and_parameters_is_one_value() {
        use crate::{SqlCell, SqlParams};
        assert_eq!(
            filters("SELECT 1 FROM v_workspace_file WHERE path = :start || '..' || :end"),
            vec![(
                "path".to_string(),
                Operand::Concat(vec![
                    Operand::Param(":start".into()),
                    Operand::Text("..".into()),
                    Operand::Param(":end".into()),
                ])
            )]
        );
        for sql in [
            "SELECT 1 FROM v_workspace_file WHERE path = :start || parent",
            "SELECT 1 FROM v_workspace_file WHERE path = :start || lower('x')",
            "SELECT 1 FROM v_workspace_file WHERE path = :start ||",
            "SELECT 1 FROM v_workspace_file WHERE path = :start || || :end",
            "SELECT 1 FROM v_workspace_file WHERE path = :start | :end",
        ] {
            assert_eq!(filters(sql), vec![], "{sql}");
        }
        let sql = "SELECT 1 FROM v_change WHERE kind = 'range' AND target = :start || '..' || :end";
        let bound = |start: Option<SqlCell>| {
            let mut p = vec![("end".to_string(), SqlCell::Text("working".into()))];
            p.extend(start.map(|s| ("start".to_string(), s)));
            SqlParams::Named(p)
        };
        assert_eq!(
            text_filters(
                sql,
                "v_change",
                &["kind", "target"],
                &bound(Some(SqlCell::Text("snap:1".into())))
            )
            .unwrap(),
            vec![
                ("kind".to_string(), "range".to_string()),
                ("target".to_string(), "snap:1..working".to_string())
            ]
        );
        for start in [None, Some(SqlCell::Int(1)), Some(SqlCell::Null(()))] {
            assert_eq!(
                text_filters(sql, "v_change", &["kind", "target"], &bound(start.clone())).unwrap(),
                vec![("kind".to_string(), "range".to_string())],
                "{start:?}"
            );
        }
    }
}
