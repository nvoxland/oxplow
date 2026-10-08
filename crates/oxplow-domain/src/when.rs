//! A command's `when` (`.context/commands.md` "Offering a command to a
//! person"): VS Code's when-clause syntax over the facts the window
//! publishes about what it shows — its **context keys** ([`CONTEXT_KEYS`]).
//! A command whose `when` is false there isn't offered in search, the menu
//! bar or by its shortcut.
//!
//! The grammar is VS Code's: `!`, `&&` (binding tighter than `||`), `||`,
//! parentheses; `==` / `!=` against `true`, `false` or a string (quoted,
//! or a bare word); `=~` against a regular expression literal
//! (`/^work/i`). A key alone is truthy when it's `true` or a non-empty
//! string. The ordering comparisons (`<` `<=` `>` `>=`) and `in` / `not
//! in` are VS Code's too, but no context key holds a number or a list, so
//! they are refused as not supported yet.
//!
//! The window evaluates `when` (`apps/desktop/src/when.ts`); this module
//! checks it where a command is registered — it parses, names only keys in
//! the catalog, and compares each to a value of its kind — and evaluates
//! it the same way, so both read `fixtures/when_cases.json` alike.

use std::collections::BTreeMap;

/// What a context key holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Bool,
    Str,
}

/// One fact the window publishes for `when`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextKey {
    pub name: &'static str,
    pub kind: KeyKind,
    pub doc: &'static str,
}

/// Every context key the window publishes (`apps/desktop/src/when.ts`
/// `windowContext`), sorted.
pub const CONTEXT_KEYS: &[ContextKey] = &[
    ContextKey {
        name: "fileDirty",
        kind: KeyKind::Bool,
        doc: "The file shown has unsaved changes and has finished loading.",
    },
    ContextKey {
        name: "fileShown",
        kind: KeyKind::Bool,
        doc: "A file of the working tree is the page shown.",
    },
    ContextKey {
        name: "pageKind",
        kind: KeyKind::Str,
        doc: "The kind of the page shown (`file`, `wiki`, `git-dashboard`, …); empty when none is.",
    },
    ContextKey {
        name: "shellAvailable",
        kind: KeyKind::Bool,
        doc: "The window runs in the app, whose shell opens and creates projects (a browser window has none).",
    },
    ContextKey {
        name: "streamKind",
        kind: KeyKind::Str,
        doc: "The kind of the stream shown: `primary` or `worktree`; empty when none is.",
    },
    ContextKey {
        name: "streamShown",
        kind: KeyKind::Bool,
        doc: "A stream is shown.",
    },
    ContextKey {
        name: "threadShown",
        kind: KeyKind::Bool,
        doc: "A thread is shown.",
    },
    ContextKey {
        name: "vcsEnabled",
        kind: KeyKind::Bool,
        doc: "The project is a git workspace.",
    },
];

/// The context key `name`.
pub fn context_key(name: &str) -> Option<&'static ContextKey> {
    CONTEXT_KEYS.iter().find(|k| k.name == name)
}

/// A value a key is compared to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Literal {
    Bool(bool),
    Str(String),
}

/// A parsed `when`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhenExpr {
    /// A key alone: `true`, or a non-empty string.
    Key(String),
    Not(Box<WhenExpr>),
    And(Box<WhenExpr>, Box<WhenExpr>),
    Or(Box<WhenExpr>, Box<WhenExpr>),
    /// `key == value` (`equal`), or `key != value`.
    Equals {
        key: String,
        value: Literal,
        equal: bool,
    },
    /// `key =~ /pattern/flags`.
    Matches {
        key: String,
        pattern: String,
        flags: String,
    },
}

/// A value the window holds for a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextValue {
    Bool(bool),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Open,
    Close,
    Not,
    And,
    Or,
    Eq,
    NotEq,
    Match,
    /// `<`, `<=`, `>`, `>=`, `in`, `not in`: VS Code's, not supported yet.
    Unsupported(&'static str),
    /// A key, `true` / `false`, or a bare value.
    Word(String),
    Quoted(String),
    Regex {
        pattern: String,
        flags: String,
    },
}

fn describe(t: &Token) -> String {
    match t {
        Token::Open => "`(`".into(),
        Token::Close => "`)`".into(),
        Token::Not => "`!`".into(),
        Token::And => "`&&`".into(),
        Token::Or => "`||`".into(),
        Token::Eq => "`==`".into(),
        Token::NotEq => "`!=`".into(),
        Token::Match => "`=~`".into(),
        Token::Unsupported(op) => format!("`{op}`"),
        Token::Word(w) => format!("`{w}`"),
        Token::Quoted(s) => format!("'{s}'"),
        Token::Regex { pattern, flags } => format!("/{pattern}/{flags}"),
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@' | '#')
}

fn tokens(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            ' ' | '\t' | '\n' => i += 1,
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            '&' if next == Some('&') => {
                out.push(Token::And);
                i += 2;
            }
            '|' if next == Some('|') => {
                out.push(Token::Or);
                i += 2;
            }
            '=' if next == Some('=') => {
                out.push(Token::Eq);
                i += 2;
                // VS Code takes `===` too.
                if chars.get(i) == Some(&'=') {
                    i += 1;
                }
            }
            '=' if next == Some('~') => {
                out.push(Token::Match);
                i += 2;
                while chars.get(i).is_some_and(|c| *c == ' ') {
                    i += 1;
                }
                if chars.get(i) != Some(&'/') {
                    return Err("`=~` takes a regular expression, `/pattern/flags`".into());
                }
                let mut pattern = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("unterminated regular expression".into()),
                        Some('\\') => {
                            if let Some(n) = chars.get(i + 1) {
                                // `\/` is a `/` in the pattern.
                                if *n != '/' {
                                    pattern.push('\\');
                                }
                                pattern.push(*n);
                            }
                            i += 2;
                        }
                        Some('/') => {
                            i += 1;
                            break;
                        }
                        Some(c) => {
                            pattern.push(*c);
                            i += 1;
                        }
                    }
                }
                let mut flags = String::new();
                while let Some(f) = chars.get(i).filter(|c| c.is_ascii_alphabetic()) {
                    flags.push(*f);
                    i += 1;
                }
                out.push(Token::Regex { pattern, flags });
            }
            '!' if next == Some('=') => {
                out.push(Token::NotEq);
                i += 2;
                if chars.get(i) == Some(&'=') {
                    i += 1;
                }
            }
            '!' => {
                out.push(Token::Not);
                i += 1;
            }
            '<' | '>' => {
                let op = match (c, next) {
                    ('<', Some('=')) => "<=",
                    ('>', Some('=')) => ">=",
                    ('<', _) => "<",
                    _ => ">",
                };
                out.push(Token::Unsupported(op));
                i += op.len();
            }
            '\'' | '"' => {
                let quote = c;
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("unterminated string".into()),
                        Some(c) if *c == quote => {
                            i += 1;
                            break;
                        }
                        Some(c) => {
                            s.push(*c);
                            i += 1;
                        }
                    }
                }
                out.push(Token::Quoted(s));
            }
            c if is_word(c) => {
                let start = i;
                while chars.get(i).is_some_and(|c| is_word(*c)) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                match word.as_str() {
                    "in" => out.push(Token::Unsupported("in")),
                    "not" if out_next_word(&chars, i) == Some("in") => {
                        // `not in`: skip to past `in`.
                        while chars.get(i).is_some_and(|c| *c == ' ') {
                            i += 1;
                        }
                        i += 2;
                        out.push(Token::Unsupported("not in"));
                    }
                    _ => out.push(Token::Word(word)),
                }
            }
            other => return Err(format!("unexpected `{other}`")),
        }
    }
    Ok(out)
}

/// The word starting after the spaces at `i`, if it is one.
fn out_next_word(chars: &[char], mut i: usize) -> Option<&'static str> {
    while chars.get(i).is_some_and(|c| *c == ' ') {
        i += 1;
    }
    let rest: String = chars[i.min(chars.len())..]
        .iter()
        .take_while(|c| is_word(**c))
        .collect();
    (rest == "in").then_some("in")
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn or(&mut self) -> Result<WhenExpr, String> {
        let mut left = self.and()?;
        while self.peek() == Some(&Token::Or) {
            self.at += 1;
            left = WhenExpr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<WhenExpr, String> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Token::And) {
            self.at += 1;
            left = WhenExpr::And(Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<WhenExpr, String> {
        if self.peek() == Some(&Token::Not) {
            self.at += 1;
            return Ok(WhenExpr::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<WhenExpr, String> {
        match self.next() {
            Some(Token::Open) => {
                let inner = self.or()?;
                match self.next() {
                    Some(Token::Close) => Ok(inner),
                    Some(t) => Err(format!("expected `)`, found {}", describe(&t))),
                    None => Err("expected `)`".into()),
                }
            }
            Some(Token::Word(key)) => self.comparison(key),
            Some(t) => Err(format!("expected a context key, found {}", describe(&t))),
            None => Err("expected a context key".into()),
        }
    }

    fn comparison(&mut self, key: String) -> Result<WhenExpr, String> {
        match self.peek().cloned() {
            Some(op @ (Token::Eq | Token::NotEq)) => {
                self.at += 1;
                let value = match self.next() {
                    Some(Token::Word(w)) if w == "true" => Literal::Bool(true),
                    Some(Token::Word(w)) if w == "false" => Literal::Bool(false),
                    Some(Token::Word(w)) | Some(Token::Quoted(w)) => Literal::Str(w),
                    Some(t) => return Err(format!("expected a value, found {}", describe(&t))),
                    None => return Err("expected a value".into()),
                };
                Ok(WhenExpr::Equals {
                    key,
                    value,
                    equal: op == Token::Eq,
                })
            }
            Some(Token::Match) => {
                self.at += 1;
                match self.next() {
                    Some(Token::Regex { pattern, flags }) => {
                        compile(&pattern, &flags)?;
                        Ok(WhenExpr::Matches {
                            key,
                            pattern,
                            flags,
                        })
                    }
                    _ => Err("`=~` takes a regular expression, `/pattern/flags`".into()),
                }
            }
            Some(Token::Unsupported(op)) => Err(format!(
                "`{op}` is not supported yet: no context key holds a number or a list"
            )),
            _ => Ok(WhenExpr::Key(key)),
        }
    }
}

/// `pattern` with `flags` (`i`, `m`, `s`) as a regular expression.
fn compile(pattern: &str, flags: &str) -> Result<regex::Regex, String> {
    if let Some(f) = flags.chars().find(|f| !matches!(f, 'i' | 'm' | 's')) {
        return Err(format!(
            "regular expression flag `{f}` isn't one of `i`, `m`, `s`"
        ));
    }
    let source = if flags.is_empty() {
        pattern.to_string()
    } else {
        format!("(?{flags}){pattern}")
    };
    regex::Regex::new(&source).map_err(|e| format!("not a regular expression: {e}"))
}

/// `src` as a `when`: its syntax, not yet its keys ([`check`]).
pub fn parse(src: &str) -> Result<WhenExpr, String> {
    let mut parser = Parser {
        tokens: tokens(src)?,
        at: 0,
    };
    let expr = parser.or()?;
    match parser.next() {
        None => Ok(expr),
        Some(t) => Err(format!("unexpected {}", describe(&t))),
    }
}

/// `src` as a command's `when`: it parses, every key it names is one the
/// window publishes, and each is compared to a value of its kind — `true`
/// / `false` for a boolean key, a string or a regular expression for a
/// string key.
pub fn check(src: &str) -> Result<WhenExpr, String> {
    let expr = parse(src)?;
    check_keys(&expr)?;
    Ok(expr)
}

fn known(key: &str) -> Result<&'static ContextKey, String> {
    context_key(key).ok_or_else(|| {
        let names: Vec<&str> = CONTEXT_KEYS.iter().map(|k| k.name).collect();
        format!(
            "`{key}` isn't a context key; the window publishes {}",
            names.join(", ")
        )
    })
}

fn check_keys(expr: &WhenExpr) -> Result<(), String> {
    match expr {
        WhenExpr::Key(key) => known(key).map(|_| ()),
        WhenExpr::Not(e) => check_keys(e),
        WhenExpr::And(a, b) | WhenExpr::Or(a, b) => {
            check_keys(a)?;
            check_keys(b)
        }
        WhenExpr::Equals { key, value, .. } => match (known(key)?.kind, value) {
            (KeyKind::Bool, Literal::Bool(_)) | (KeyKind::Str, Literal::Str(_)) => Ok(()),
            (KeyKind::Bool, Literal::Str(s)) => Err(format!(
                "`{key}` is true or false; compare it to `true` or `false`, not `{s}`"
            )),
            (KeyKind::Str, Literal::Bool(b)) => {
                Err(format!("`{key}` is a string; compare it to one, not `{b}`"))
            }
        },
        WhenExpr::Matches { key, .. } => match known(key)?.kind {
            KeyKind::Str => Ok(()),
            KeyKind::Bool => Err(format!(
                "`{key}` is true or false; `=~` matches a string key"
            )),
        },
    }
}

/// `expr` against what the window holds: a key it doesn't hold is false
/// alone, equal to nothing and unmatched.
pub fn eval(expr: &WhenExpr, context: &BTreeMap<String, ContextValue>) -> bool {
    match expr {
        WhenExpr::Key(key) => match context.get(key) {
            Some(ContextValue::Bool(b)) => *b,
            Some(ContextValue::Str(s)) => !s.is_empty(),
            None => false,
        },
        WhenExpr::Not(e) => !eval(e, context),
        WhenExpr::And(a, b) => eval(a, context) && eval(b, context),
        WhenExpr::Or(a, b) => eval(a, context) || eval(b, context),
        WhenExpr::Equals { key, value, equal } => {
            let same = match (context.get(key), value) {
                (Some(ContextValue::Bool(b)), Literal::Bool(l)) => b == l,
                (Some(ContextValue::Str(s)), Literal::Str(l)) => s == l,
                // A key the window doesn't hold is `false` when it's
                // compared to `false` (VS Code's undefined is falsy).
                (None, Literal::Bool(l)) => !l,
                _ => false,
            };
            same == *equal
        }
        WhenExpr::Matches {
            key,
            pattern,
            flags,
        } => match context.get(key) {
            Some(ContextValue::Str(s)) => compile(pattern, flags).is_ok_and(|re| re.is_match(s)),
            _ => false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../fixtures/when_cases.json")).unwrap()
    }

    fn context(v: &Value) -> BTreeMap<String, ContextValue> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                let value = match v {
                    Value::Bool(b) => ContextValue::Bool(*b),
                    other => ContextValue::Str(other.as_str().unwrap().to_string()),
                };
                (k.clone(), value)
            })
            .collect()
    }

    /// The shared cases: what each `when` is in its context, the same as
    /// the window's evaluator reads them.
    #[test]
    fn the_shared_cases_evaluate_alike() {
        for case in cases()["cases"].as_array().unwrap() {
            let src = case["when"].as_str().unwrap();
            let expr = parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            assert_eq!(
                eval(&expr, &context(&case["context"])),
                case["is"].as_bool().unwrap(),
                "{src} in {}",
                case["context"]
            );
        }
    }

    /// The shared refusals: each says why.
    #[test]
    fn the_shared_refusals_say_why() {
        for case in cases()["refused"].as_array().unwrap() {
            let src = case["when"].as_str().unwrap();
            let says = case["says"].as_str().unwrap();
            let err = parse(src).unwrap_err();
            assert!(err.contains(says), "{src}: `{err}` doesn't say `{says}`");
        }
    }

    /// Where a command is registered, a `when` names only the keys the
    /// window publishes, each compared to a value of its kind.
    #[test]
    fn a_when_names_known_keys_compared_to_their_kind() {
        check("fileShown && fileDirty || pageKind == wiki && streamKind =~ /^w/").unwrap();
        for (src, says) in [
            ("fileDirt", "`fileDirt` isn't a context key"),
            ("fileDirty == yes", "compare it to `true` or `false`"),
            ("pageKind == true", "is a string"),
            ("fileDirty =~ /x/", "`=~` matches a string key"),
        ] {
            let err = check(src).unwrap_err();
            assert!(err.contains(says), "{src}: {err}");
        }
    }

    #[test]
    fn the_context_keys_are_sorted_and_documented() {
        let names: Vec<&str> = CONTEXT_KEYS.iter().map(|k| k.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert!(CONTEXT_KEYS.iter().all(|k| !k.doc.is_empty()));
    }
}
