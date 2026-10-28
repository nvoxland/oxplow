//! A metric's `expression:` — the one place its aggregation, filter or
//! formula is written. A closed grammar, compiled (never interpreted) into
//! the stored form the engine already reads:
//!
//! ```text
//! expression := formula | aggregate [ FILTER ( WHERE cond { AND cond } ) ]
//! aggregate  := COUNT(*) | SUM(value) | AVG(value) | MIN(value) | MAX(value)
//!             | LAST(value) | RATIO(num, den)
//! cond       := value >= n | value <= n | severity = 's' | dim('key') = 's'
//! formula    := METRIC('a') (+ | - | * | /) METRIC('b')
//! ```
//!
//! `value >= n` and `value <= n` are the bounds a fact filter has
//! (`minValue`, `maxValue`); a strict `>` or `<` has no stored form, so it is
//! refused with the closest spelling. At most one of each condition; one
//! `dim()`. [`render`] writes the canonical form back, so an expression
//! round-trips through [`compile`].

use crate::{FilterConfig, FormulaConfig};

/// What an expression says, in the stored form.
#[derive(Debug, Clone, PartialEq)]
pub enum Compiled {
    /// An aggregation of a measure's facts, with the filter before it.
    Measure {
        aggregation: String,
        filter: Option<FilterConfig>,
    },
    /// A binary operation over two other metrics.
    Formula(FormulaConfig),
}

/// The grammar, as a refusal quotes it.
pub const GRAMMAR: &str = "`COUNT(*)`, `SUM(value)`, `AVG(value)`, `MIN(value)`, `MAX(value)`, \
`LAST(value)` or `RATIO(num, den)`, optionally followed by `FILTER (WHERE …)` joining `value >= n`, \
`value <= n`, `severity = 's'` and `dim('key') = 's'` with AND (one of each; strict `>` and `<` \
aren't stored: use `>=` / `<=`); or `METRIC('a') + - * / METRIC('b')`";

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Num(f64),
    Str(String),
    Sym(&'static str),
}

fn lex(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '(' | ')' | ',' | '*' | '/' | '+' | '-' | '=' => {
                out.push(Tok::Sym(match c {
                    '(' => "(",
                    ')' => ")",
                    ',' => ",",
                    '*' => "*",
                    '/' => "/",
                    '+' => "+",
                    '-' => "-",
                    _ => "=",
                }));
                i += 1;
            }
            '>' | '<' => {
                let eq = chars.get(i + 1) == Some(&'=');
                out.push(Tok::Sym(match (c, eq) {
                    ('>', true) => ">=",
                    ('>', false) => ">",
                    ('<', true) => "<=",
                    _ => "<",
                }));
                i += if eq { 2 } else { 1 };
            }
            '\'' => {
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err("a string isn't closed".into()),
                        Some('\'') if chars.get(i + 1) == Some(&'\'') => {
                            s.push('\'');
                            i += 2;
                        }
                        Some('\'') => {
                            i += 1;
                            break;
                        }
                        Some(c) => {
                            s.push(*c);
                            i += 1;
                        }
                    }
                }
                out.push(Tok::Str(s));
            }
            c if c.is_ascii_digit() => {
                let start = i;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_digit() || *c == '.')
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Tok::Num(
                    text.parse()
                        .map_err(|_| format!("`{text}` isn't a number"))?,
                ));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while chars
                    .get(i)
                    .is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_')
                {
                    i += 1;
                }
                out.push(Tok::Word(chars[start..i].iter().collect()));
            }
            other => return Err(format!("unexpected `{other}`")),
        }
    }
    Ok(out)
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn sym(&mut self, s: &'static str) -> Result<(), String> {
        match self.next() {
            Some(Tok::Sym(got)) if got == s => Ok(()),
            other => Err(format!("expected `{s}`, found {}", show(other.as_ref()))),
        }
    }

    fn word(&mut self, w: &str) -> Result<(), String> {
        match self.next() {
            Some(Tok::Word(got)) if got.eq_ignore_ascii_case(w) => Ok(()),
            other => Err(format!("expected `{w}`, found {}", show(other.as_ref()))),
        }
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(got)) if got.eq_ignore_ascii_case(w))
    }

    fn string(&mut self) -> Result<String, String> {
        match self.next() {
            Some(Tok::Str(s)) => Ok(s),
            other => Err(format!(
                "expected a 'string', found {}",
                show(other.as_ref())
            )),
        }
    }

    fn number(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(n),
            other => Err(format!("expected a number, found {}", show(other.as_ref()))),
        }
    }
}

fn show(t: Option<&Tok>) -> String {
    match t {
        None => "the end".into(),
        Some(Tok::Word(w)) => format!("`{w}`"),
        Some(Tok::Num(n)) => format!("`{n}`"),
        Some(Tok::Str(s)) => format!("'{s}'"),
        Some(Tok::Sym(s)) => format!("`{s}`"),
    }
}

/// Compile `expr`; a refusal names what is wrong and the grammar.
pub fn compile(expr: &str) -> Result<Compiled, String> {
    parse(expr).map_err(|why| {
        format!(
            "`expression: {}`: {why} — the grammar is {GRAMMAR}",
            expr.trim()
        )
    })
}

fn parse(expr: &str) -> Result<Compiled, String> {
    let mut p = Parser {
        toks: lex(expr)?,
        at: 0,
    };
    let head = match p.next() {
        Some(Tok::Word(w)) => w.to_ascii_uppercase(),
        other => return Err(format!("starts with {}", show(other.as_ref()))),
    };
    let out = match head.as_str() {
        "METRIC" => {
            p.sym("(")?;
            let left = p.string()?;
            p.sym(")")?;
            let op = match p.next() {
                Some(Tok::Sym("+")) => "add",
                Some(Tok::Sym("-")) => "sub",
                Some(Tok::Sym("*")) => "mul",
                Some(Tok::Sym("/")) => "div",
                other => {
                    return Err(format!(
                        "expected `+`, `-`, `*` or `/` between two metrics, found {}",
                        show(other.as_ref())
                    ))
                }
            };
            p.word("METRIC")?;
            p.sym("(")?;
            let right = p.string()?;
            p.sym(")")?;
            Compiled::Formula(FormulaConfig {
                op: op.into(),
                left,
                right,
            })
        }
        "COUNT" => {
            p.sym("(")?;
            p.sym("*")?;
            p.sym(")")?;
            measure(&mut p, "count")?
        }
        "SUM" | "AVG" | "MIN" | "MAX" | "LAST" => {
            p.sym("(")?;
            p.word("value")?;
            p.sym(")")?;
            measure(&mut p, &head.to_ascii_lowercase())?
        }
        "RATIO" => {
            p.sym("(")?;
            p.word("num")?;
            p.sym(",")?;
            p.word("den")?;
            p.sym(")")?;
            measure(&mut p, "ratio")?
        }
        other => return Err(format!("`{other}` isn't an aggregate or `METRIC`")),
    };
    match p.peek() {
        None => Ok(out),
        Some(t) => Err(format!("{} follows a complete expression", show(Some(t)))),
    }
}

fn measure(p: &mut Parser, aggregation: &str) -> Result<Compiled, String> {
    let mut filter = FilterConfig::default();
    let mut any = false;
    if p.is_word("FILTER") {
        p.next();
        p.sym("(")?;
        p.word("WHERE")?;
        loop {
            condition(p, &mut filter)?;
            any = true;
            if p.is_word("AND") {
                p.next();
            } else {
                break;
            }
        }
        p.sym(")")?;
    }
    Ok(Compiled::Measure {
        aggregation: aggregation.into(),
        filter: any.then_some(filter),
    })
}

fn condition(p: &mut Parser, f: &mut FilterConfig) -> Result<(), String> {
    let once = |taken: bool, what: &str| {
        if taken {
            Err(format!("`{what}` is there twice"))
        } else {
            Ok(())
        }
    };
    match p.next() {
        Some(Tok::Word(w)) if w.eq_ignore_ascii_case("value") => match p.next() {
            Some(Tok::Sym(">=")) => {
                once(f.min_value.is_some(), "value >=")?;
                f.min_value = Some(p.number()?);
                Ok(())
            }
            Some(Tok::Sym("<=")) => {
                once(f.max_value.is_some(), "value <=")?;
                f.max_value = Some(p.number()?);
                Ok(())
            }
            Some(Tok::Sym(s @ (">" | "<"))) => Err(format!(
                "a strict `value {s} n` isn't stored — write `value {s}= n`"
            )),
            other => Err(format!(
                "`value` is compared with `>=` or `<=`, found {}",
                show(other.as_ref())
            )),
        },
        Some(Tok::Word(w)) if w.eq_ignore_ascii_case("severity") => {
            p.sym("=")?;
            once(f.severity.is_some(), "severity =")?;
            f.severity = Some(p.string()?);
            Ok(())
        }
        Some(Tok::Word(w)) if w.eq_ignore_ascii_case("dim") => {
            p.sym("(")?;
            let key = p.string()?;
            p.sym(")")?;
            p.sym("=")?;
            once(f.dim_eq.is_some(), "dim(…) =")?;
            f.dim_eq = Some(vec![key, p.string()?]);
            Ok(())
        }
        other => Err(format!(
            "a condition is on `value`, `severity` or `dim('key')`, found {}",
            show(other.as_ref())
        )),
    }
}

fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The canonical expression for `compiled`.
pub fn render(compiled: &Compiled) -> String {
    match compiled {
        Compiled::Formula(f) => {
            let op = match f.op.as_str() {
                "add" => "+",
                "sub" => "-",
                "mul" => "*",
                _ => "/",
            };
            format!(
                "METRIC({}) {op} METRIC({})",
                quote(&f.left),
                quote(&f.right)
            )
        }
        Compiled::Measure {
            aggregation,
            filter,
        } => {
            let mut out = match aggregation.as_str() {
                "count" => "COUNT(*)".to_string(),
                "ratio" => "RATIO(num, den)".to_string(),
                other => format!("{}(value)", other.to_ascii_uppercase()),
            };
            let mut conds = Vec::new();
            if let Some(f) = filter {
                if let Some(n) = f.min_value {
                    conds.push(format!("value >= {}", number(n)));
                }
                if let Some(n) = f.max_value {
                    conds.push(format!("value <= {}", number(n)));
                }
                if let Some(s) = &f.severity {
                    conds.push(format!("severity = {}", quote(s)));
                }
                if let Some(pair) = &f.dim_eq {
                    if let [key, value] = pair.as_slice() {
                        conds.push(format!("dim({}) = {}", quote(key), quote(value)));
                    }
                }
            }
            if !conds.is_empty() {
                out.push_str(&format!(" FILTER (WHERE {})", conds.join(" AND ")));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape the shipped metrics use: the canonical text compiles to
    /// the stored form and renders back as it was.
    #[test]
    fn an_expression_round_trips_through_its_stored_form() {
        for expr in [
            "COUNT(*)",
            "SUM(value)",
            "AVG(value)",
            "MIN(value)",
            "MAX(value)",
            "LAST(value)",
            "RATIO(num, den)",
            "COUNT(*) FILTER (WHERE value >= 11)",
            "COUNT(*) FILTER (WHERE value <= 0)",
            "COUNT(*) FILTER (WHERE value >= 1.5 AND value <= 9)",
            "COUNT(*) FILTER (WHERE severity = 'error')",
            "SUM(value) FILTER (WHERE dim('oxplow.rule') = 'unsafe_block')",
            "AVG(value) FILTER (WHERE dim('oxplow.tests_stat') = 'at_close')",
            "METRIC('a.b') / METRIC('c.d')",
            "METRIC('a') + METRIC('b')",
            "METRIC('a') - METRIC('b')",
            "METRIC('a') * METRIC('b')",
        ] {
            let compiled = compile(expr).unwrap_or_else(|e| panic!("{expr}: {e}"));
            assert_eq!(render(&compiled), expr);
        }
    }

    #[test]
    fn it_compiles_to_what_the_engine_stores() {
        assert_eq!(
            compile("count(*) filter (where value >= 11)").unwrap(),
            Compiled::Measure {
                aggregation: "count".into(),
                filter: Some(FilterConfig {
                    min_value: Some(11.0),
                    ..Default::default()
                }),
            }
        );
        assert_eq!(
            compile("SUM(value) FILTER (WHERE dim('oxplow.rule') = 'x')").unwrap(),
            Compiled::Measure {
                aggregation: "sum".into(),
                filter: Some(FilterConfig {
                    dim_eq: Some(vec!["oxplow.rule".into(), "x".into()]),
                    ..Default::default()
                }),
            }
        );
        assert_eq!(
            compile("METRIC('a') / METRIC('b')").unwrap(),
            Compiled::Formula(FormulaConfig {
                op: "div".into(),
                left: "a".into(),
                right: "b".into()
            })
        );
    }

    /// Anything outside the grammar is refused, naming the grammar.
    #[test]
    fn what_the_grammar_does_not_say_is_refused() {
        for (expr, says) in [
            ("MEDIAN(value)", "isn't an aggregate"),
            ("SUM(other)", "expected `value`"),
            (
                "SUM(value) FILTER (WHERE value >= 1 OR value <= 0)",
                "found `OR`",
            ),
            ("COUNT(*) FILTER (WHERE path = 'a')", "found `path`"),
            ("COUNT(*) FILTER (WHERE value > 10)", "write `value >= n`"),
            ("COUNT(*) FILTER (WHERE value >= 1 AND value >= 2)", "twice"),
            (
                "COUNT(*) FILTER (WHERE dim('a') = 'x' AND dim('b') = 'y')",
                "twice",
            ),
            ("COUNT(*) extra", "follows"),
            ("COUNT(*) FILTER (WHERE value >= 'x')", "expected a number"),
            ("METRIC('a') % METRIC('b')", "unexpected"),
            ("", "starts with the end"),
            ("SUM(value", "expected `)`"),
        ] {
            let err = compile(expr).expect_err(expr);
            assert!(err.contains(says), "{expr}: {err}");
            assert!(err.contains("the grammar is"), "{expr}: {err}");
        }
    }
}
