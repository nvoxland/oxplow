//! An extension's ref kinds (`ref_kinds:` in its manifest; P8.D6, stable
//! since P10 on the github example's pull requests): new kinds of thing a
//! ref can name,
//! registered into the running vocabulary's `KindRegistry` by
//! `vocabulary_reactor`, so `[[acme_pr:12]]` (and `[[pr:12]]`, with a
//! `wikilink:` prefix) links while the extension is installed.
//!
//! ```yaml
//! ref_kinds:
//!   - kind: acme_pr             # <namespace>_<name>; the namespace is the extension's name, `-` → `_`
//!     label: Pull request
//!     id: '^\d+$'               # anchored
//!     resolve: prs              # one of its models, with `ref` and `title` columns
//!     page: pr                  # one of its pages, opened with `?ref=<ref>`
//!     wikilink: pr              # optional `[[pr:12]]` sugar
//!     searchable: found         # optional: one of its models, with `ref`, `title` and `body` (P9.D3)
//!     icon: git-pull-request    # one of REF_KIND_ICONS
//! ```

use serde::{Deserialize, Serialize};

use crate::extensions::manifest_v2::{at, item_lines, key_line};
use crate::extensions::ExtensionPage;

/// The icons a ref kind may name (lucide names; the desktop maps each).
pub const REF_KIND_ICONS: &[&str] = &[
    "book-open",
    "box",
    "bug",
    "calendar",
    "circle-dot",
    "database",
    "file-text",
    "flag",
    "folder",
    "git-branch",
    "git-commit",
    "git-pull-request",
    "link",
    "message-square",
    "package",
    "server",
    "shield",
    "star",
    "tag",
    "ticket",
    "user",
    "zap",
];

/// Wikilink sugar core reads before kinds (`dir:`, `finding:`): a prefix
/// may not shadow it. A work list's own ids (`tsk42`) are its id
/// pattern's, not a prefix core keeps.
const RESERVED_PREFIXES: &[&str] = &["dir", "finding"];

/// A ref kind an extension declares (valid ones; invalid ones are in its
/// `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct RefKindDecl {
    pub kind: String,
    pub extension: String,
    pub label: String,
    /// The anchored regex its ids match.
    pub id_pattern: String,
    /// The view whose `title` names one (`v_<extension>_<model>`, by `ref`).
    pub resolve: String,
    /// The page that opens one: `page:ext.<extension>.<page>`, given
    /// `?ref=<ref>`.
    pub page: String,
    pub wikilink: Option<String>,
    /// The view whose rows (`ref`, `title`, `body`) search indexes under
    /// the kind (`kind_search`); none, its refs aren't found by search.
    pub searchable: Option<String>,
    pub icon: String,
    /// `file:line` of the declaration.
    pub declared_at: String,
}

/// A `ref_kinds:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RefKindFile {
    kind: String,
    label: String,
    id: String,
    resolve: String,
    page: String,
    #[serde(default)]
    wikilink: Option<String>,
    #[serde(default)]
    searchable: Option<String>,
    icon: String,
}

/// The registry's spec for `decl`: its id pattern and its `wikilink:`
/// sugar — the one constructor the vocabulary reactor and `extension test`
/// register an extension's kinds with.
pub fn kind_spec(
    decl: &RefKindDecl,
) -> Result<oxplow_domain::refs::kind::KindSpec, oxplow_domain::refs::kind::KindError> {
    let spec = oxplow_domain::refs::kind::KindSpec::new(&decl.kind, &decl.id_pattern)?;
    Ok(match &decl.wikilink {
        Some(w) => spec.wikilink_prefix(w),
        None => spec,
    })
}

/// Parse a `ref_kinds:` block against the extension's models and pages:
/// the valid kinds, and an error (`file:line: …`) for each broken one.
/// Collisions with core and other extensions are the reactor's to find.
pub fn parse_ref_kinds(
    extension: &str,
    models: &[oxplow_db::models::ModelSource],
    pages: &[ExtensionPage],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<RefKindDecl>, Vec<String>) {
    let block = key_line(manifest, "ref_kinds");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`ref_kinds` must be a list")],
        );
    };
    let mut out: Vec<RefKindDecl> = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "ref_kinds");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let f: RefKindFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, item_line, format!("ref kind: {e}")));
                continue;
            }
        };
        let line = item_line;
        let declared_at = at(file, line, "").trim_end_matches(": ").to_string();
        match decl_of(extension, models, pages, f, declared_at) {
            Ok(d) if out.iter().any(|o| o.kind == d.kind) => errors.push(at(
                file,
                line,
                format!("ref kind `{}` is declared twice", d.kind),
            )),
            Ok(d) if d.wikilink.is_some() && out.iter().any(|o| o.wikilink == d.wikilink) => errors
                .push(at(
                    file,
                    line,
                    format!(
                        "ref kind `{}`: `wikilink: {}` is another of its kinds' too",
                        d.kind,
                        d.wikilink.as_deref().unwrap_or_default()
                    ),
                )),
            Ok(d) => out.push(d),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    // A sugar that is another of its own kinds would make `[[x:…]]` name
    // two refs (tsk933): both kinds stay, the sugar goes.
    let kinds: Vec<String> = out.iter().map(|d| d.kind.clone()).collect();
    for d in &mut out {
        if let Some(w) = d.wikilink.as_ref().filter(|w| kinds.contains(w)) {
            errors.push(format!(
                "{}: ref kind `{}`: `wikilink: {w}` is its own kind `{w}`; choose another prefix",
                d.declared_at, d.kind
            ));
            d.wikilink = None;
        }
    }
    (out, errors)
}

/// The longest id pattern an extension may declare.
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
fn id_pattern(pattern: &str) -> Result<String, String> {
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

fn decl_of(
    extension: &str,
    models: &[oxplow_db::models::ModelSource],
    pages: &[ExtensionPage],
    f: RefKindFile,
    declared_at: String,
) -> Result<RefKindDecl, String> {
    let named = |m: String| format!("ref kind `{}`: {m}", f.kind);
    let prefix = format!(
        "{}_",
        oxplow_domain::events::schema::extension_namespace(extension)
    );
    if !oxplow_domain::refs::grammar::is_valid_kind(&f.kind)
        || !f.kind.starts_with(&prefix)
        || f.kind.len() == prefix.len()
    {
        return Err(format!(
            "ref kind `{}` must be `{prefix}<name>`: lowercase letters, digits and `_`",
            f.kind
        ));
    }
    let id = id_pattern(&f.id).map_err(|why| named(format!("`id: {}`: {why}", f.id)))?;
    oxplow_domain::refs::kind::KindSpec::new(&f.kind, &id).map_err(|e| named(e.to_string()))?;
    // A model of the extension's own with these columns, for `key:`.
    let model_with = |key: &str, name: &str, columns: &[&str], why: &str| {
        let model = models.iter().find(|m| m.decl.name == name).ok_or_else(|| {
            named(format!(
                "`{key}: {name}` isn't one of this extension's models (`models:`)"
            ))
        })?;
        match columns
            .iter()
            .find(|col| !model.decl.columns.iter().any(|c| c.name == **col))
        {
            Some(col) => Err(named(format!(
                "model `{name}` has no `{col}` column ({why})"
            ))),
            None => Ok(()),
        }
    };
    model_with(
        "resolve",
        &f.resolve,
        &["ref", "title"],
        "a ref kind's model gives each ref's title",
    )?;
    if let Some(searchable) = &f.searchable {
        model_with(
            "searchable",
            searchable,
            &["ref", "title", "body"],
            "what search indexes for each ref",
        )?;
    }
    let page = pages.iter().find(|p| p.id == f.page).ok_or_else(|| {
        named(format!(
            "`page: {}` isn't one of this extension's pages (`pages:`)",
            f.page
        ))
    })?;
    if let Some(w) = &f.wikilink {
        let core = oxplow_domain::refs::kind::core_kinds();
        if !oxplow_domain::refs::grammar::is_valid_kind(w) {
            return Err(named(format!(
                "`wikilink: {w}` must be lowercase letters, digits and `_`"
            )));
        }
        if core.get(w).is_some()
            || core.kind_for_wikilink_prefix(w).is_some()
            || RESERVED_PREFIXES.contains(&w.as_str())
        {
            return Err(named(format!("`wikilink: {w}` is oxplow's own")));
        }
    }
    if !REF_KIND_ICONS.contains(&f.icon.as_str()) {
        return Err(named(format!(
            "`icon: {}` isn't one oxplow draws; use one of {}",
            f.icon,
            REF_KIND_ICONS.join(", ")
        )));
    }
    Ok(RefKindDecl {
        extension: extension.to_string(),
        label: f.label,
        id_pattern: id,
        resolve: oxplow_db::models::extension_view(extension, &f.resolve),
        page: page.page_ref.clone(),
        wikilink: f.wikilink,
        searchable: f
            .searchable
            .as_deref()
            .map(|m| oxplow_db::models::extension_view(extension, m)),
        icon: f.icon,
        declared_at,
        kind: f.kind,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use crate::extensions::load_extensions;

    pub(crate) const MANIFEST: &str = "manifest: 2
name: acme
sharing: private
intent: { purpose: PRs., origin: null, examples: [] }
models:
  - name: prs
    version: 1
    description: Pull requests.
    columns:
      - { name: ref, type: TEXT, doc: The ref. }
      - { name: title, type: TEXT, doc: Its title. }
pages:
  - { id: pr, title: Pull request, category: Work, lens: open }
ref_kinds:
  - kind: acme_pr
    label: Pull request
    id: '^\\d+$'
    resolve: prs
    page: pr
    wikilink: pr
    icon: git-pull-request
";

    pub(crate) fn write_acme(root: &std::path::Path, manifest: &str) {
        let dir = root.join("oxplow/extensions/acme");
        std::fs::create_dir_all(dir.join("models")).unwrap();
        std::fs::create_dir_all(dir.join("lenses")).unwrap();
        std::fs::write(dir.join("extension.yaml"), manifest).unwrap();
        std::fs::write(
            dir.join("models/prs.sql"),
            "SELECT 'acme_pr:1' AS ref, 'One' AS title",
        )
        .unwrap();
        std::fs::write(
            dir.join("lenses/open.yaml"),
            "title: Open\nquery: \"SELECT 1 AS n\"\n",
        )
        .unwrap();
    }

    fn acme(manifest: &str) -> crate::extensions::Extension {
        let dir = tempfile::tempdir().unwrap();
        write_acme(dir.path(), manifest);
        load_extensions(dir.path())
            .into_iter()
            .find(|e| e.name == "acme")
            .unwrap()
    }

    /// The desktop draws exactly the icons a ref kind may name: its
    /// `REF_KIND_ICONS` map (`apps/desktop/src/refKinds.ts`) has a key for
    /// each, and no other.
    #[test]
    fn the_desktop_draws_every_allowed_icon() {
        let ts = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src/refKinds.ts"),
        )
        .unwrap();
        let start = ts.find("REF_KIND_ICONS").unwrap();
        let body = &ts[start..start + ts[start..].find("};").unwrap()];
        let mut drawn: Vec<&str> = body
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"')?.split('"').next())
            .collect();
        drawn.sort();
        let mut allowed = super::REF_KIND_ICONS.to_vec();
        allowed.sort();
        assert_eq!(drawn, allowed);
    }

    #[test]
    fn a_ref_kind_loads_with_its_model_and_page() {
        let ext = acme(MANIFEST);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        let k = &ext.ref_kinds[0];
        assert_eq!(
            (k.kind.as_str(), k.resolve.as_str(), k.page.as_str()),
            ("acme_pr", "v_acme_prs", "page:ext.acme.pr")
        );
        assert_eq!(k.declared_at, "oxplow/extensions/acme/extension.yaml:15");
    }

    #[test]
    fn a_broken_ref_kind_is_an_error_at_its_line() {
        for (from, to, says) in [
            ("id: '^\\d+$'", "id: '^(\\d+$'", "no groups"),
            // Patterns the renderer's JS engine would backtrack on, or read
            // differently from Rust's (tsk797).
            ("id: '^\\d+$'", "id: '^(a+)+$'", "no groups"),
            ("id: '^\\d+$'", "id: '^a|b$'", "no alternation"),
            ("id: '^\\d+$'", "id: '^\\p{L}+$'", "`\\p`"),
            ("id: '^\\d+$'", "id: '\\d+'", "anchored"),
            // tsk917: repeats that can share characters backtrack
            // polynomially in JS; the two engines read these apart.
            ("id: '^\\d+$'", "id: '^\\w*\\w*\\w*!$'", "share characters"),
            ("id: '^\\d+$'", "id: '^\\w+_\\w+$'", "share characters"),
            ("id: '^\\d+$'", "id: '^\\d+\\$'", "anchored"),
            ("id: '^\\d+$'", "id: '^\\<\\d+$'", "`\\<`"),
            ("id: '^\\d+$'", "id: '^[a-z&&b]+$'", "set operation"),
            ("id: '^\\d+$'", "id: '^[^a]+$'", "negated"),
            ("id: '^\\d+$'", "id: '^a.b$'", "`.`"),
            ("id: '^\\d+$'", "id: '^\\S+$'", "`\\S`"),
            ("id: '^\\d+$'", "id: '^a{,3}$'", "`{,m}`"),
            ("id: '^\\d+$'", "id: '^\\d+?$'", "one quantifier"),
            ("kind: acme_pr", "kind: other_pr", "must be `acme_<name>`"),
            (
                "resolve: prs",
                "resolve: nope",
                "isn't one of this extension's models",
            ),
            (
                "page: pr",
                "page: nope",
                "isn't one of this extension's pages",
            ),
            ("wikilink: pr", "wikilink: git", "oxplow's own"),
            (
                "icon: git-pull-request",
                "icon: skull",
                "isn't one oxplow draws",
            ),
        ] {
            let ext = acme(&MANIFEST.replace(from, to));
            assert!(ext.ref_kinds.is_empty(), "{to}");
            let errors = ext.errors.join("\n");
            assert!(
                errors.contains("extension.yaml:15:") && errors.contains(says),
                "{to}: {errors}"
            );
        }
    }

    /// Core's own sugar is reserved (`dir:`, `finding:`); a work list's
    /// ids are that list's to say (its id pattern), not a prefix core
    /// keeps, so `tsk` is free.
    #[test]
    fn only_cores_own_sugar_is_reserved() {
        for (prefix, refused) in [("dir", true), ("finding", true), ("tsk", false)] {
            let ext = acme(&MANIFEST.replace("wikilink: pr", &format!("wikilink: {prefix}")));
            assert_eq!(
                ext.errors.iter().any(|e| e.contains("oxplow's own")),
                refused,
                "{prefix}: {:?}",
                ext.errors
            );
        }
    }

    /// tsk933: a `wikilink:` that is another of the extension's own kinds
    /// would make one link name two refs; it is refused at its line, and
    /// costs only the sugar — both kinds load.
    #[test]
    fn a_wikilink_that_is_one_of_its_own_kinds_is_refused() {
        let two = MANIFEST.replace("wikilink: pr", "wikilink: acme_issue")
            + "  - kind: acme_issue
    label: Issue
    id: '^\\d+$'
    resolve: prs
    page: pr
    icon: ticket
";
        let ext = acme(&two);
        let errors = ext.errors.join("\n");
        assert!(
            errors.contains("`wikilink: acme_issue`") && errors.contains("its own kind"),
            "{errors}"
        );
        assert_eq!(
            ext.ref_kinds
                .iter()
                .map(|k| (k.kind.as_str(), k.wikilink.as_deref()))
                .collect::<Vec<_>>(),
            [("acme_pr", None), ("acme_issue", None)]
        );
    }

    /// tsk917: an id pattern far past the cap is refused.
    #[test]
    fn a_long_id_pattern_is_refused() {
        let long = format!("id: '^{}$'", "a".repeat(300));
        let ext = acme(&MANIFEST.replace("id: '^\\d+$'", &long));
        assert!(ext.ref_kinds.is_empty());
        assert!(ext.errors.join("\n").contains("256"), "{:?}", ext.errors);
    }

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
            assert_eq!(super::id_pattern(written).as_deref(), Ok(kept), "{written}");
        }
        let kept = super::id_pattern(r"^\d+$").unwrap();
        let spec = oxplow_domain::refs::kind::KindSpec::new("acme_pr", &kept).unwrap();
        assert!(spec.id_regex.is_match("12"));
        assert!(
            !spec.id_regex.is_match("\u{0661}\u{0662}"),
            "Unicode digits"
        );
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

    /// P9.D3: `searchable:` names one of the extension's models with
    /// `ref`, `title` and `body` — what search indexes under the kind.
    #[test]
    fn a_searchable_kind_names_a_model_with_ref_title_and_body() {
        let found = "  - name: found
    version: 1
    description: Pull requests, as search finds them.
    columns:
      - { name: ref, type: TEXT, doc: The ref. }
      - { name: title, type: TEXT, doc: Its title. }
      - { name: body, type: TEXT, doc: Its description. }
pages:";
        let manifest = MANIFEST.replace("pages:", found).replace(
            "    wikilink: pr\n",
            "    wikilink: pr\n    searchable: found\n",
        );
        let load = |manifest: &str| {
            let dir = tempfile::tempdir().unwrap();
            write_acme(dir.path(), manifest);
            std::fs::write(
                dir.path().join("oxplow/extensions/acme/models/found.sql"),
                "SELECT 'acme_pr:1' AS ref, 'One' AS title, 'The first.' AS body",
            )
            .unwrap();
            load_extensions(dir.path())
                .into_iter()
                .find(|e| e.name == "acme")
                .unwrap()
        };
        let ext = load(&manifest);
        assert!(ext.errors.is_empty(), "{:?}", ext.errors);
        assert_eq!(ext.ref_kinds[0].searchable.as_deref(), Some("v_acme_found"));
        // Without it, a kind resolves and opens but isn't searched.
        assert_eq!(acme(MANIFEST).ref_kinds[0].searchable, None);
        for (to, says) in [
            (
                "searchable: nope",
                "`searchable: nope` isn't one of this extension's models",
            ),
            // `prs` has `ref` and `title`, no `body`.
            ("searchable: prs", "model `prs` has no `body` column"),
        ] {
            let ext = load(&manifest.replace("searchable: found", to));
            assert!(ext.ref_kinds.is_empty(), "{to}");
            let errors = ext.errors.join("\n");
            assert!(errors.contains(says), "{to}: {errors}");
        }
    }
}
