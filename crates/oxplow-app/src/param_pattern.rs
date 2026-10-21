//! The patterns a resource's params declare (`resources:` → `params:
//! { id: { pattern: '^\d+$' } }`), kept in one explicit ASCII form so
//! Rust's regex engine and the desktop's JS read a pattern alike: `\d` is
//! `[0-9]` in both, never Rust's Unicode digits.

/// The longest param pattern an extension may declare.
const MAX_ID_PATTERN: usize = 256;

/// The most one repeat (`{n,m}`) may ask for.
const MAX_REPEAT: u32 = 256;

/// One run of an id pattern: a set of printable ASCII characters (bit
/// `c` for character `c`), repeated `min` to `max` (`None`: unbounded)
/// times.
struct Run {
    set: u128,
    min: u32,
    max: Option<u32>,
}

fn bit(c: char) -> u128 {
    1u128 << (c as u32)
}

fn span(lo: char, hi: char) -> u128 {
    (lo as u32..=hi as u32).fold(0, |s, c| s | (1u128 << c))
}

fn digits() -> u128 {
    span('0', '9')
}

fn word() -> u128 {
    span('0', '9') | span('A', 'Z') | span('a', 'z') | bit('_')
}

/// A punctuation mark both engines read as itself when escaped — not `<`
/// or `>`, which Rust reads as word boundaries.
fn escapable(c: char) -> bool {
    c.is_ascii_punctuation() && !matches!(c, '<' | '>')
}

/// An id pattern in the one form the desktop's JS and oxplow's Rust read
/// alike, and that JS's backtracking engine can't be made to hang on
/// (tsk797, tsk917). The pattern is a run of characters, classes (`[…]`,
/// not negated, no set operations), `\d`, `\w` and escaped punctuation,
/// each with at most one quantifier (`? * + {n} {n,} {n,m}`, `n`, `m` ≤
/// 256), anchored `^…$`, at most 256 characters of printable ASCII. Two
/// repeats of varying length must be fenced by a character the first
/// can't match (`^[A-Z]+-\d+$`), so a failing match gives each one back
/// at most once. What is kept spells every set out — `\d` is `[0-9]`,
/// never Rust's Unicode digits — so neither engine reads it its own way.
pub(crate) fn param_pattern(pattern: &str) -> Result<String, String> {
    if pattern.len() > MAX_ID_PATTERN {
        return Err(format!("it is longer than {MAX_ID_PATTERN} characters"));
    }
    if let Some(c) = pattern.chars().find(|c| !(' '..='~').contains(c)) {
        return Err(format!("{c:?} isn't printable ASCII"));
    }
    let anchored = || "it must be anchored (`^…$`): it matches a whole id".to_string();
    let body = pattern.strip_prefix('^').ok_or_else(anchored)?;
    let body = body.strip_suffix('$').ok_or_else(anchored)?;
    // `\$` at the end is a dollar sign, not the anchor.
    if (body.len() - body.trim_end_matches('\\').len()) % 2 == 1 {
        return Err(anchored());
    }
    let mut chars = body.chars().peekable();
    let mut runs: Vec<Run> = Vec::new();
    while let Some(c) = chars.next() {
        let set = match c {
            '\\' => escape(chars.next())?,
            '[' => class(&mut chars)?,
            '(' | ')' => {
                return Err(
                    "no groups: an id pattern is a run of characters, classes and quantifiers"
                        .into(),
                )
            }
            '|' => return Err("no alternation (`|`): use a class".into()),
            '.' => {
                return Err("no `.`: the two engines differ on what it matches; use a class".into())
            }
            '^' | '$' => return Err(format!("`{c}` only anchors the ends")),
            '*' | '+' | '?' | '{' => return Err(format!("`{c}` has nothing to repeat")),
            ']' | '}' => return Err(format!("escape a literal `{c}`")),
            c => bit(c),
        };
        let (min, max) = quantifier(&mut chars)?;
        runs.push(Run { set, min, max });
    }
    for (i, run) in runs.iter().enumerate() {
        if run.max == Some(run.min) {
            continue;
        }
        let Some(next) = runs[i + 1..].iter().position(|r| r.max != Some(r.min)) else {
            break;
        };
        let fenced = runs[i + 1..i + 1 + next]
            .iter()
            .any(|r| r.min >= 1 && r.set & run.set == 0);
        if !fenced {
            return Err("two repeats that can share characters backtrack in the desktop's engine: put a character the first can't match between them".into());
        }
    }
    let mut kept = String::from("^");
    for run in &runs {
        kept.push_str(&spell(run.set));
        kept.push_str(&match (run.min, run.max) {
            (1, Some(1)) => String::new(),
            (0, None) => "*".into(),
            (1, None) => "+".into(),
            (0, Some(1)) => "?".into(),
            (n, Some(m)) if n == m => format!("{{{n}}}"),
            (n, None) => format!("{{{n},}}"),
            (n, Some(m)) => format!("{{{n},{m}}}"),
        });
    }
    kept.push('$');
    Ok(kept)
}

/// What `\<e>` matches outside a class.
fn escape(e: Option<char>) -> Result<u128, String> {
    match e {
        Some('d') => Ok(digits()),
        Some('w') => Ok(word()),
        Some(p) if escapable(p) => Ok(bit(p)),
        Some(e) => Err(format!(
            "`\\{e}` isn't one both engines read alike; use `\\d`, `\\w`, a class or an escaped punctuation mark"
        )),
        None => Err("it ends in a lone `\\`".into()),
    }
}

/// A class's set, after its `[`.
fn class(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Result<u128, String> {
    let mut set = 0u128;
    // The last single character, which a `-` may start a range from.
    let mut from: Option<char> = None;
    let mut first = true;
    loop {
        let c = chars.next().ok_or("a class (`[`) isn't closed")?;
        let next = chars.peek().copied();
        match c {
            ']' if first => return Err("an empty class (`[]`) matches nothing".into()),
            ']' => return Ok(set),
            '^' if first => {
                return Err("no negated class (`[^…]`): list what an id may hold".into())
            }
            '[' => return Err("escape a `[` inside a class".into()),
            '&' | '~' | '-' if next == Some(c) => {
                return Err(format!(
                    "`{c}{c}` is a set operation in one engine and two characters in the other; escape them"
                ))
            }
            '-' if from.is_some() && next.is_some_and(|n| n != ']') => {
                let lo = from.take().unwrap_or('-');
                let hi = match chars.next() {
                    Some('\\') => match chars.next() {
                        Some(p) if escapable(p) => p,
                        _ => return Err("a range ends in a character".into()),
                    },
                    Some('[') => return Err("escape a `[` inside a class".into()),
                    Some(h) => h,
                    None => return Err("a class (`[`) isn't closed".into()),
                };
                if lo > hi {
                    return Err(format!("the range `{lo}-{hi}` runs backwards"));
                }
                set |= span(lo, hi);
            }
            '\\' => {
                from = None;
                match chars.next() {
                    Some('d') => set |= digits(),
                    Some('w') => set |= word(),
                    Some(p) if escapable(p) => {
                        set |= bit(p);
                        from = Some(p);
                    }
                    e => return escape(e).map(|_| set),
                }
            }
            c => {
                set |= bit(c);
                from = Some(c);
            }
        }
        first = false;
    }
}

/// The quantifier after a run, if any: its `(min, max)`.
fn quantifier(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<(u32, Option<u32>), String> {
    let q = match chars.peek() {
        Some('*') => (0, None),
        Some('+') => (1, None),
        Some('?') => (0, Some(1)),
        Some('{') => {
            chars.next();
            let mut inside = String::new();
            loop {
                match chars.next() {
                    Some('}') => break,
                    Some(c) => inside.push(c),
                    None => return Err("a `{` isn't closed".into()),
                }
            }
            let number = |s: &str| -> Result<u32, String> {
                let n: u32 = s
                    .parse()
                    .map_err(|_| format!("`{{{inside}}}` isn't a repeat: `{{n}}`, `{{n,}}` or `{{n,m}}` (no `{{,m}}`)"))?;
                if n > MAX_REPEAT {
                    return Err(format!("a repeat is at most {MAX_REPEAT}"));
                }
                Ok(n)
            };
            let q = match inside.split_once(',') {
                None => {
                    let n = number(&inside)?;
                    (n, Some(n))
                }
                Some((n, "")) => (number(n)?, None),
                Some((n, m)) => (number(n)?, Some(number(m)?)),
            };
            if q.1.is_some_and(|m| m < q.0) {
                return Err(format!("`{{{inside}}}` runs backwards"));
            }
            if q == (0, Some(0)) {
                return Err("`{0}` matches nothing".into());
            }
            if matches!(chars.peek(), Some('*' | '+' | '?' | '{')) {
                return Err("one quantifier per run (no lazy `?`, no stacking)".into());
            }
            return Ok(q);
        }
        _ => return Ok((1, Some(1))),
    };
    chars.next();
    if matches!(chars.peek(), Some('*' | '+' | '?' | '{')) {
        return Err("one quantifier per run (no lazy `?`, no stacking)".into());
    }
    Ok(q)
}

/// `set` spelled out: one character, or a class of ranges in ASCII order.
fn spell(set: u128) -> String {
    // Escaped: what either engine reads as syntax, in or out of a class.
    let one = |c: char| {
        if "\\.+*?()|[]{}^$-&~#".contains(c) {
            format!("\\{c}")
        } else {
            c.to_string()
        }
    };
    if set.count_ones() == 1 {
        return one(char::from(set.trailing_zeros() as u8));
    }
    let mut out = String::from("[");
    let mut c = 0x20u32;
    while c <= 0x7e {
        if set & (1u128 << c) == 0 {
            c += 1;
            continue;
        }
        let start = c;
        while c < 0x7e && set & (1u128 << (c + 1)) != 0 {
            c += 1;
        }
        let (lo, hi) = (char::from(start as u8), char::from(c as u8));
        match c - start {
            0 => out.push_str(&one(lo)),
            1 => {
                out.push_str(&one(lo));
                out.push_str(&one(hi));
            }
            _ => out.push_str(&format!("{}-{}", one(lo), one(hi))),
        }
        c += 1;
    }
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    /// tsk917: a pattern is kept in one explicit ASCII form, so Rust and
    /// the desktop's JS read it alike — `\d` is `[0-9]` in both, never
    /// Rust's Unicode digits — and repeats fenced by a character the first
    /// can't match load.
    #[test]
    fn an_id_pattern_is_kept_in_one_form_both_engines_read_alike() {
        for (written, kept) in [
            (r"^\d+$", r"^[0-9]+$"),
            (r"^[A-Z]+-\d+$", r"^[A-Z]+\-[0-9]+$"),
            (r"^\w{2,5}\.v\d$", r"^[0-9A-Z_a-z]{2,5}\.v[0-9]$"),
            (r"^[a-c_\-]?x$", r"^[\-_a-c]?x$"),
        ] {
            assert_eq!(
                super::param_pattern(written).as_deref(),
                Ok(kept),
                "{written}"
            );
        }
        let kept = regex::Regex::new(&super::param_pattern(r"^\d+$").unwrap()).unwrap();
        assert!(kept.is_match("12"));
        assert!(!kept.is_match("\u{0661}\u{0662}"), "Unicode digits");
    }

    /// tsk917: every printable character, spelled alone or in a class,
    /// is itself to Rust's engine — the spelling escapes exactly what
    /// either engine reads as syntax.
    #[test]
    fn every_character_is_spelled_as_itself() {
        for c in ' '..='~' {
            for set in [super::bit(c), super::bit(c) | super::bit('a')] {
                let re = regex::Regex::new(&format!("^{}$", super::spell(set))).unwrap();
                for d in ' '..='~' {
                    let want = set & super::bit(d) != 0;
                    assert_eq!(re.is_match(&d.to_string()), want, "{c:?} {d:?} {re}");
                }
            }
        }
    }
}
