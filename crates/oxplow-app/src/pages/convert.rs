//! A one-off converter from an extension's lenses (`lenses/*.yaml` and the
//! manifest's `pages:` entries) to page files (`pages/<id>.yaml`,
//! [`super::schema`]). `oxplow-dev pages convert <ext-dir> [--write]`
//! drives it. It only writes the new files and a report: deleting the
//! lenses and rewriting the manifest is a later, separate step, so the
//! manifest rewrites are reported as suggestions.
//!
//! The mapping (`.context/pages-and-tabs.md` "Page files"):
//!
//! - A `pages:` entry drawn by a lens becomes `pages/<entry id>.yaml`: the
//!   entry's metadata, the lens's params, description, needs, tool, alert
//!   and actions (whole-lens actions are the page's; row actions stay on
//!   the viz node, as do whole actions that read the one row a `detail`,
//!   `form` or `markdown` holds). Its `count:` lens is a loader `count`
//!   and the page's `badge`.
//! - A lens's query is a loader named after it; its viz and role blocks a
//!   viz node `from` that loader.
//! - A `document` is a `stack` (with `view:`, a `switch`), a `dashboard` a
//!   `grid`; its own query is the loader `row`. A part whose lens nothing
//!   else uses (and that is no tool, has no alert, and isn't mounted
//!   into) is inlined: a loader named by the part, a node with its id. A
//!   part param the document's row holds binds from it (`params: { x: {
//!   from: row.x } }`). Any other part is an `embed` of that lens's page.
//! - A document other extensions mount into ends with its `slot`.
//! - Every other lens that is neither inlined nor counted only as a badge
//!   becomes its own hidden page (a launcher lens a visible one).
//! - What the migration deletes (`answer`, `thread-answers*`,
//!   `thread-notes`, the custom dashboards, `viz: answer`, the built-in
//!   `lens` and `resource` pages) is skipped.
//!
//! What can't be converted mechanically is written anyway with a
//! `# TODO(convert): <why>` line on top; every file is checked with
//! [`parse_page`], and one that doesn't parse says so in its TODO.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde_yaml::{Mapping, Value};

use super::schema::parse_page;

/// One extension's files: its manifest and its lenses (`(slug, text)`).
#[derive(Debug, Clone)]
pub struct Source {
    pub manifest: String,
    pub lenses: Vec<(String, String)>,
}

/// What a conversion produces: page files (paths relative to the
/// extension's directory) and what it did.
#[derive(Debug, Clone)]
pub struct Conversion {
    pub files: Vec<(PathBuf, String)>,
    pub report: Report,
}

/// What a conversion did, a line each.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Each page file written, and what it came from.
    pub converted: Vec<String>,
    /// Each lens inlined into a page.
    pub inlined: Vec<String>,
    /// Each part embedded as another page.
    pub embedded: Vec<String>,
    /// Each lens or page left out, and why.
    pub skipped: Vec<String>,
    /// Each file that needs a hand, and why.
    pub todos: Vec<String>,
    /// The manifest's references to lenses, as they would name pages.
    pub manifest_suggestions: Vec<String>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sections: [(&str, &Vec<String>); 6] = [
            ("converted", &self.converted),
            ("inlined", &self.inlined),
            ("embedded", &self.embedded),
            ("skipped", &self.skipped),
            ("todos", &self.todos),
            ("manifest suggestions", &self.manifest_suggestions),
        ];
        for (name, lines) in sections {
            writeln!(f, "{name} ({}):", lines.len())?;
            for l in lines {
                writeln!(f, "  {l}")?;
            }
        }
        Ok(())
    }
}

/// Read extension `dir`'s manifest and lenses.
pub fn read_source(dir: &Path) -> Result<Source, String> {
    let manifest = std::fs::read_to_string(dir.join("extension.yaml"))
        .map_err(|e| format!("{}: {e}", dir.join("extension.yaml").display()))?;
    let mut lenses = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("lenses")) {
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
            .collect();
        paths.sort();
        for p in paths {
            let slug = p
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            lenses.push((slug, text));
        }
    }
    Ok(Source { manifest, lenses })
}

/// Convert extension `dir`, reading the extensions beside it for the
/// mounts and references they make into it.
pub fn convert_dir(dir: &Path) -> Result<Conversion, String> {
    let ext = read_source(dir)?;
    let mut others = Vec::new();
    if let Some(parent) = dir.parent() {
        let me = dir.canonicalize().ok();
        if let Ok(entries) = std::fs::read_dir(parent) {
            let mut dirs: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.join("extension.yaml").is_file())
                .filter(|p| p.canonicalize().ok() != me)
                .collect();
            dirs.sort();
            for d in dirs {
                if let Ok(s) = read_source(&d) {
                    others.push(s);
                }
            }
        }
    }
    convert(&ext, &others)
}

const SKIPPED_LENSES: &[&str] = &[
    "answer",
    "thread-notes",
    "dashboard",
    "dashboard-tiles",
    "dashboards",
];
const SKIPPED_PAGES: &[&str] = &["lens", "resource", "custom-dashboard"];

/// The role blocks a viz node carries, in the order they are written.
const ROLE_BLOCKS: &[&str] = &[
    "chart", "tree", "timeline", "steps", "hunks", "code", "diff", "editor", "embed", "terminal",
    "markdown", "form", "custom", "group", "emphasis", "depth",
];

struct Ext {
    name: String,
    namespace: String,
    manifest: Mapping,
    lenses: BTreeMap<String, Mapping>,
    broken: Vec<(String, String)>,
}

fn s(v: &str) -> Value {
    Value::String(v.to_string())
}

fn get<'a>(m: &'a Mapping, k: &str) -> Option<&'a Value> {
    m.get(k)
}

fn get_str<'a>(m: &'a Mapping, k: &str) -> Option<&'a str> {
    m.get(k).and_then(Value::as_str)
}

fn get_seq<'a>(m: &'a Mapping, k: &str) -> &'a [Value] {
    m.get(k)
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn get_map<'a>(m: &'a Mapping, k: &str) -> Option<&'a Mapping> {
    m.get(k).and_then(Value::as_mapping)
}

fn truthy(m: &Mapping, k: &str) -> bool {
    m.get(k).and_then(Value::as_bool).unwrap_or(false)
}

impl Ext {
    fn load(src: &Source) -> Result<Ext, String> {
        let manifest: Mapping =
            serde_yaml::from_str(&src.manifest).map_err(|e| format!("extension.yaml: {e}"))?;
        let name = get_str(&manifest, "name")
            .ok_or("extension.yaml: no `name`")?
            .to_string();
        let namespace = get_str(&manifest, "namespace").unwrap_or(&name).to_string();
        let mut lenses = BTreeMap::new();
        let mut broken = Vec::new();
        for (slug, text) in &src.lenses {
            match serde_yaml::from_str::<Mapping>(text) {
                Ok(m) => {
                    lenses.insert(slug.clone(), m);
                }
                Err(e) => broken.push((slug.clone(), e.to_string())),
            }
        }
        Ok(Ext {
            name,
            namespace,
            manifest,
            lenses,
            broken,
        })
    }

    fn entries(&self) -> &[Value] {
        get_seq(&self.manifest, "pages")
    }

    fn ui_seq(&self, key: &str) -> &[Value] {
        get_map(&self.manifest, "ui")
            .map(|ui| get_seq(ui, key))
            .unwrap_or(&[])
    }

    /// The `pages:` entry drawn by lens `slug`.
    fn entry_of(&self, slug: &str) -> Option<&Mapping> {
        self.entries()
            .iter()
            .filter_map(Value::as_mapping)
            .find(|e| get_str(e, "lens") == Some(slug))
    }

    /// `(extension, slug)` of a lens reference written in this extension.
    fn qualify(&self, r: &str) -> (String, String) {
        match r.split_once('/') {
            Some((e, s)) => (e.to_string(), s.to_string()),
            None => (self.name.clone(), r.to_string()),
        }
    }
}

fn viz_of(lens: &Mapping) -> &str {
    get_str(lens, "viz").unwrap_or("table")
}

fn is_layout(lens: &Mapping) -> bool {
    matches!(viz_of(lens), "document" | "dashboard")
}

fn parts_of(lens: &Mapping) -> (&'static str, &[Value]) {
    if viz_of(lens) == "dashboard" {
        ("tiles", get_seq(lens, "tiles"))
    } else {
        ("sections", get_seq(lens, "sections"))
    }
}

/// The id a part has: its `id`, else its lens's slug.
fn part_id(part: &Mapping) -> String {
    get_str(part, "id")
        .map(str::to_string)
        .or_else(|| get_str(part, "lens").map(|l| l.rsplit('/').next().unwrap_or(l).to_string()))
        .unwrap_or_default()
}

/// One reference to a lens.
#[derive(Debug, Clone)]
struct Use {
    kind: UseKind,
    /// The lens of the converted extension holding the reference, if any.
    host: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseKind {
    Part,
    Mount,
    Panel,
    Entry,
    Count,
    Rail,
    Ref,
}

/// The lens references each lens of an extension holds, as `(target
/// reference as written, kind)`.
fn lens_refs(lens: &Mapping) -> Vec<(String, UseKind)> {
    let mut out = Vec::new();
    for key in ["sections", "tiles"] {
        for p in get_seq(lens, key).iter().filter_map(Value::as_mapping) {
            if let Some(l) = get_str(p, "lens") {
                out.push((l.to_string(), UseKind::Part));
            }
        }
    }
    for c in get_seq(lens, "columns")
        .iter()
        .filter_map(Value::as_mapping)
    {
        if let Some(e) = get_str(c, "expand") {
            out.push((e.to_string(), UseKind::Ref));
        }
    }
    for p in get_seq(lens, "params").iter().filter_map(Value::as_mapping) {
        if let Some(o) = get_str(p, "options_from") {
            out.push((o.to_string(), UseKind::Ref));
        }
    }
    if let Some(form) = get_map(lens, "form") {
        if let Some(ch) = get_map(form, "choices") {
            for v in ch.values().filter_map(Value::as_str) {
                out.push((v.to_string(), UseKind::Ref));
            }
        }
        for f in get_seq(form, "fields").iter().filter_map(Value::as_mapping) {
            if let Some(v) = get_str(f, "from") {
                out.push((v.to_string(), UseKind::Ref));
            }
        }
    }
    for a in get_seq(lens, "actions")
        .iter()
        .filter_map(Value::as_mapping)
    {
        if let Some(ch) = get_map(a, "choices") {
            for v in ch.values().filter_map(Value::as_str) {
                out.push((v.to_string(), UseKind::Ref));
            }
        }
    }
    out
}

/// The slot names (`sections` / `tiles`) other extensions (and this one)
/// mount into, per lens slug of extension `target`.
fn slot_targets(all: &[&Ext], target: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for e in all {
        for m in e.ui_seq("slots").iter().filter_map(Value::as_mapping) {
            let Some(slot) = get_str(m, "slot") else {
                continue;
            };
            let Some((ext, rest)) = slot.split_once('/') else {
                continue;
            };
            if ext != target {
                continue;
            }
            if let Some((slug, kind)) = rest.rsplit_once('.') {
                if kind == "sections" || kind == "tiles" {
                    out.insert(slug.to_string(), kind.to_string());
                }
            }
        }
    }
    out
}

/// A loader name for `want`: a plain identifier, not a model's.
fn ident(want: &str) -> String {
    let mut out: String = want
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !out.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        out = format!("l_{out}");
    }
    if out.starts_with("v_") || out.starts_with("m_") || out == "temp" || out == "main" {
        out = format!("p_{out}");
    }
    out
}

/// The identifiers `sql` names as tables or columns (the schema's rule:
/// not in a string literal, not after `.` or `:`, not a call).
fn sql_words(sql: &str) -> BTreeSet<String> {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                i += 1;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let after = start > 0 && (chars[start - 1] == '.' || chars[start - 1] == ':');
            let mut j = i;
            while j < chars.len() && chars[j] == ' ' {
                j += 1;
            }
            let call = j < chars.len() && chars[j] == '(';
            if !after && !call {
                out.insert(word.to_ascii_lowercase());
            }
            continue;
        }
        i += 1;
    }
    out
}

/// The `{{row.` / `when` test: does whole action `a` read the row?
fn reads_row(a: &Mapping) -> bool {
    a.contains_key("when")
        || get(a, "input")
            .map(|i| {
                serde_yaml::to_string(i)
                    .unwrap_or_default()
                    .contains("{{row.")
            })
            .unwrap_or(false)
}

fn is_row_action(a: &Mapping) -> bool {
    truthy(a, "row") || truthy(a, "drag") || truthy(a, "edit") || a.contains_key("group")
}

fn holds_one_row(viz: &str) -> bool {
    matches!(viz, "detail" | "form" | "markdown")
}

/// A param's definition without its label: two with the same mean the
/// same.
fn param_meaning(p: &Mapping) -> Mapping {
    let mut m = p.clone();
    m.remove("label");
    m
}

struct Ctx<'a> {
    ext: &'a Ext,
    all: Vec<&'a Ext>,
    uses: BTreeMap<String, Vec<Use>>,
    skipped: BTreeMap<String, String>,
    slots: BTreeMap<String, String>,
    /// The page id each lens of the extension is shown as.
    page_of: BTreeMap<String, String>,
    /// Lenses that must have a page of their own (an embed of them was
    /// needed after all).
    forced: BTreeSet<String>,
}

impl<'a> Ctx<'a> {
    fn lens(&self, slug: &str) -> Option<&'a Mapping> {
        self.ext.lenses.get(slug)
    }

    fn uses(&self, slug: &str) -> &[Use] {
        self.uses.get(slug).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Lens `slug` may be inlined into `host`: its one use is that part,
    /// and nothing about it needs a page of its own.
    fn inlinable(&self, slug: &str, host: &str) -> bool {
        let Some(lens) = self.lens(slug) else {
            return false;
        };
        let uses = self.uses(slug);
        uses.len() == 1
            && uses[0].kind == UseKind::Part
            && uses[0].host.as_deref() == Some(host)
            && !lens.contains_key("tool")
            && !lens.contains_key("alert")
            && !lens.contains_key("launcher")
            && !self.slots.contains_key(slug)
            && !self.skipped.contains_key(slug)
            && !self.forced.contains(slug)
    }

    /// Only a page's badge counts it.
    fn count_only(&self, slug: &str) -> bool {
        let uses = self.uses(slug);
        uses.len() == 1 && uses[0].kind == UseKind::Count
    }

    /// `<ext>/<page>` for a lens reference written in this extension.
    fn page_ref(&self, r: &str) -> String {
        let (ext, slug) = self.ext.qualify(r);
        if ext == self.ext.name {
            let page = self.page_of.get(&slug).cloned().unwrap_or(slug);
            return format!("{ext}/{page}");
        }
        let page = self
            .all
            .iter()
            .find(|e| e.name == ext)
            .and_then(|e| e.entry_of(&slug))
            .and_then(|e| get_str(e, "id"))
            .map(str::to_string)
            .unwrap_or(slug);
        format!("{ext}/{page}")
    }

    /// The page id lens `slug` of extension `ext` is shown as.
    fn page_id_in(&self, ext: &str, slug: &str) -> String {
        self.page_ref(&format!("{ext}/{slug}"))
            .split_once('/')
            .map(|(_, p)| p.to_string())
            .unwrap_or_default()
    }
}

/// A page being written.
struct Builder<'c, 'a> {
    ctx: &'c mut Ctx<'a>,
    params: Vec<Mapping>,
    /// The top lens's (and entry's) params: never bound from a row.
    top_params: BTreeSet<String>,
    loaders: Vec<(String, Mapping)>,
    node_ids: BTreeSet<String>,
    needs: Vec<String>,
    refresh: bool,
    actions: Vec<Value>,
    todos: Vec<String>,
    inlined: Vec<String>,
    embedded: Vec<String>,
    page_slug: String,
}

/// A document row in scope: its loader and its columns.
type Rows = Vec<(String, Vec<String>)>;

impl Builder<'_, '_> {
    fn add_param(&mut self, p: &Mapping, from: &str) {
        let Some(name) = get_str(p, "name") else {
            return;
        };
        let p = self.rewrite_param(p);
        if let Some(have) = self
            .params
            .iter_mut()
            .find(|h| get_str(h, "name") == Some(name))
        {
            let (a, b) = (param_meaning(have), param_meaning(&p));
            // One restating part of the other (a part repeating the
            // page's param with just its default) is the same param.
            let within = |x: &Mapping, y: &Mapping| x.iter().all(|(k, v)| y.get(k) == Some(v));
            if within(&b, &a) {
                return;
            }
            if within(&a, &b) {
                *have = p;
                return;
            }
            self.todos.push(format!(
                "param `{name}` of `{from}` means something else than the page's (`{}` vs `{}`)",
                one_line(&Value::Mapping(a)),
                one_line(&Value::Mapping(b))
            ));
            return;
        }
        self.params.push(p);
    }

    fn rewrite_param(&self, p: &Mapping) -> Mapping {
        let mut p = p.clone();
        if let Some(o) = get_str(&p, "options_from").map(str::to_string) {
            p.insert(s("options_from"), s(&self.ctx.page_ref(&o)));
        }
        p
    }

    fn add_needs(&mut self, lens: &Mapping) {
        for n in get_seq(lens, "needs").iter().filter_map(Value::as_str) {
            if !self.needs.iter().any(|x| x == n) {
                self.needs.push(n.to_string());
            }
        }
        self.refresh |= truthy(lens, "refresh");
    }

    /// Add a loader named like `want`; its name, made unique and not one
    /// an earlier loader's query names.
    fn add_loader(&mut self, want: &str, body: Mapping) -> String {
        let base = ident(want);
        let earlier: BTreeSet<String> = self
            .loaders
            .iter()
            .filter_map(|(_, m)| get_str(m, "sql"))
            .flat_map(sql_words)
            .collect();
        let taken = |n: &str, loaders: &[(String, Mapping)]| {
            loaders.iter().any(|(x, _)| x == n) || earlier.contains(n)
        };
        let mut name = base.clone();
        if taken(&name, &self.loaders) {
            name = format!("{base}_rows");
        }
        let mut n = 2;
        while taken(&name, &self.loaders) {
            name = format!("{base}_{n}");
            n += 1;
        }
        self.loaders.push((name.clone(), body));
        name
    }

    fn node_id(&mut self, want: &str, parent: Option<&str>) -> String {
        let mut id = want.to_string();
        if self.node_ids.contains(&id) {
            if let Some(p) = parent {
                id = format!("{p}-{want}");
            }
            let mut n = 2;
            let base = id.clone();
            while self.node_ids.contains(&id) {
                id = format!("{base}-{n}");
                n += 1;
            }
        }
        self.node_ids.insert(id.clone());
        id
    }

    /// Does lens `lens` read a param a row in scope holds anywhere but its
    /// query (an action, a link)?
    fn row_param_outside_sql(&self, lens: &Mapping, rows: &Rows) -> bool {
        let mut rest = lens.clone();
        rest.remove("query");
        rest.remove("params");
        let text = serde_yaml::to_string(&rest).unwrap_or_default();
        get_seq(lens, "params")
            .iter()
            .filter_map(Value::as_mapping)
            .filter_map(|p| get_str(p, "name"))
            .filter(|n| !self.top_params.contains(*n))
            .filter(|n| rows.iter().any(|(_, cols)| cols.iter().any(|c| c == n)))
            .any(|n| text.contains(&format!("{{{{param.{n}")))
    }

    /// The node showing lens `slug`, its loaders added. `top`: the page's
    /// own lens (its whole actions are the page's).
    #[allow(clippy::too_many_arguments)]
    fn lens_node(
        &mut self,
        slug: &str,
        lens: &Mapping,
        head: Mapping,
        want_loader: &str,
        rows: &Rows,
        top: bool,
        node_id: Option<&str>,
    ) -> Value {
        self.add_needs(lens);
        if is_layout(lens) {
            return self.layout_node(slug, lens, head, rows, top, node_id);
        }
        // Params: the page's, unless a row in scope holds them.
        let mut bind = Mapping::new();
        for p in get_seq(lens, "params").iter().filter_map(Value::as_mapping) {
            let Some(name) = get_str(p, "name") else {
                continue;
            };
            let row = (!top && !self.top_params.contains(name))
                .then(|| {
                    rows.iter()
                        .rev()
                        .find(|(_, cols)| cols.iter().any(|c| c == name))
                })
                .flatten();
            match row {
                Some((loader, _)) => {
                    let mut from = Mapping::new();
                    from.insert(s("from"), s(&format!("{loader}.{name}")));
                    bind.insert(s(name), Value::Mapping(from));
                }
                None => self.add_param(p, slug),
            }
        }
        let viz = viz_of(lens).to_string();
        let query = get_str(lens, "query").unwrap_or("").trim().to_string();
        if viz == "custom" {
            let component = get_map(lens, "custom")
                .and_then(|c| get_str(c, "component"))
                .unwrap_or("");
            let fetches = get_seq(&self.ctx.ext.manifest, "custom_components")
                .iter()
                .filter_map(Value::as_mapping)
                .find(|c| get_str(c, "id") == Some(component))
                .is_some_and(|c| c.contains_key("sql") || c.contains_key("commands"));
            if fetches {
                self.todos.push(format!(
                    "`{slug}`'s component `{component}` reads oxplow's data itself: decide what its loader is"
                ));
            }
            if query.is_empty() {
                let mut node = headed(node_id, head);
                let mut c = Mapping::new();
                c.insert(s("id"), s(component));
                if let Some(props) = get_map(lens, "custom").and_then(|c| get(c, "props")) {
                    c.insert(s("props"), props.clone());
                }
                node.insert(s("component"), Value::Mapping(c));
                return Value::Mapping(node);
            }
        }
        for key in ["tile", "answer"] {
            if lens.contains_key(key) {
                self.todos
                    .push(format!("`{slug}` has `{key}:`, which no page node takes"));
            }
        }
        let mut node = headed(node_id, head);
        if !query.is_empty() {
            let mut body = Mapping::new();
            body.insert(s("sql"), s(&format!("{query}\n")));
            if !bind.is_empty() {
                body.insert(s("params"), Value::Mapping(bind));
            }
            let name = self.add_loader(want_loader, body);
            node.insert(s("from"), s(&name));
        }
        node.insert(s("viz"), s(&viz));
        for key in ROLE_BLOCKS {
            if let Some(v) = get(lens, key) {
                let v = if *key == "form" {
                    self.rewrite_form(v)
                } else {
                    v.clone()
                };
                node.insert(s(key), v);
            }
        }
        if let Some(cols) = get(lens, "columns") {
            node.insert(s("columns"), self.rewrite_columns(cols));
        }
        if let Some(e) = get(lens, "empty") {
            node.insert(s("empty"), e.clone());
        }
        let mut node_actions = Vec::new();
        for a in get_seq(lens, "actions") {
            let a = self.rewrite_action(a);
            let m = a.as_mapping().cloned().unwrap_or_default();
            let to_page = top && !is_row_action(&m) && !(reads_row(&m) && holds_one_row(&viz));
            if to_page {
                self.actions.push(a);
            } else {
                node_actions.push(a);
            }
        }
        if !node_actions.is_empty() {
            node.insert(s("actions"), Value::Sequence(node_actions));
        }
        Value::Mapping(node)
    }

    fn layout_node(
        &mut self,
        slug: &str,
        lens: &Mapping,
        head: Mapping,
        rows: &Rows,
        top: bool,
        node_id: Option<&str>,
    ) -> Value {
        if !top {
            for p in get_seq(lens, "params").iter().filter_map(Value::as_mapping) {
                let name = get_str(p, "name").unwrap_or_default();
                let held = rows.iter().any(|(_, c)| c.iter().any(|c| c == name));
                if !held || self.top_params.contains(name) {
                    self.add_param(p, slug);
                }
            }
        }
        if lens.contains_key("tile") {
            self.todos.push(format!(
                "`{slug}` is a dashboard whose rows are its tiles (`tile:`), which no page node takes"
            ));
        }
        let mut rows = rows.clone();
        // A page action reads the page's first loader's one row: this
        // document's row, when it is the page's and comes first.
        let mut row_first = false;
        let query = get_str(lens, "query").unwrap_or("").trim().to_string();
        if !query.is_empty() {
            let mut body = Mapping::new();
            body.insert(s("sql"), s(&format!("{query}\n")));
            let cols: Vec<String> = get_seq(lens, "columns")
                .iter()
                .filter_map(Value::as_mapping)
                .filter_map(|c| get_str(c, "key").map(str::to_string))
                .collect();
            if let Some(c) = get(lens, "columns") {
                body.insert(s("columns"), c.clone());
            }
            let want = match node_id {
                Some(id) if !top => format!("{id}_row"),
                _ => "row".to_string(),
            };
            let name = self.add_loader(&want, body);
            row_first = top && self.loaders.len() == 1;
            rows.push((name, cols));
        }
        for a in get_seq(lens, "actions") {
            let a = self.rewrite_action(a);
            let m = a.as_mapping().cloned().unwrap_or_default();
            let id = get_str(&m, "id").unwrap_or_default();
            if is_row_action(&m) {
                self.todos.push(format!(
                    "`{slug}`'s action `{id}` is a row action on a document, which has no rows to offer it on"
                ));
            } else if reads_row(&m) && !row_first {
                self.todos.push(format!(
                    "`{slug}`'s action `{id}` reads the document's row, which isn't the page's first loader"
                ));
            }
            self.actions.push(a);
        }
        let (kind, parts) = parts_of(lens);
        let mut children = Vec::new();
        for part in parts.iter().filter_map(Value::as_mapping) {
            if let Some(child) = self.part_node(slug, part, &rows, node_id) {
                children.push(child);
            }
        }
        if let Some(k) = self.ctx.slots.get(slug).cloned() {
            if k == kind {
                let mut m = Mapping::new();
                let id = self.node_id(&k, node_id);
                if id != k {
                    m.insert(s("id"), s(&id));
                }
                m.insert(s("slot"), s(&k));
                children.push(Value::Mapping(m));
            }
        }
        let mut node = headed(node_id, head);
        let children = Value::Sequence(children);
        if viz_of(lens) == "dashboard" {
            node.insert(s("grid"), children);
        } else if let Some(param) = get_str(lens, "view") {
            let mut sw = Mapping::new();
            sw.insert(s("param"), s(param));
            sw.insert(s("children"), children);
            node.insert(s("switch"), Value::Mapping(sw));
        } else {
            node.insert(s("stack"), children);
        }
        Value::Mapping(node)
    }

    fn part_node(
        &mut self,
        host: &str,
        part: &Mapping,
        rows: &Rows,
        parent: Option<&str>,
    ) -> Option<Value> {
        let pid = part_id(part);
        let id = self.node_id(&pid, parent);
        let mut head = Mapping::new();
        if let Some(t) = get(part, "title") {
            head.insert(s("title"), t.clone());
        }
        if let Some(r) = get(part, "replaceable") {
            head.insert(s("replaceable"), r.clone());
        }
        if let Some(sz) = get(part, "size") {
            head.insert(s("size"), sz.clone());
        }
        if let Some(md) = get(part, "markdown") {
            let mut node = Mapping::new();
            node.insert(s("id"), s(&id));
            node.extend(head);
            node.insert(s("markdown"), md.clone());
            return Some(Value::Mapping(node));
        }
        let r = get_str(part, "lens")?;
        let (ext, slug) = self.ctx.ext.qualify(r);
        let local = ext == self.ctx.ext.name;
        if local && self.ctx.skipped.contains_key(&slug) {
            self.todos.push(format!(
                "part `{pid}` shows `{slug}`, which the migration deletes: left out"
            ));
            return None;
        }
        if local {
            if let Some(lens) = self.ctx.lens(&slug) {
                if self.ctx.inlinable(&slug, host) && !self.row_param_outside_sql(lens, rows) {
                    if !head.contains_key("title") {
                        if let Some(t) = get(lens, "title") {
                            head.insert(s("title"), t.clone());
                        }
                    }
                    self.inlined
                        .push(format!("{slug} into {} as `{id}`", self.page_slug));
                    return Some(self.lens_node(&slug, lens, head, &pid, rows, false, Some(&id)));
                }
                if self.ctx.inlinable(&slug, host) {
                    // A row param it reads outside its query: embedded,
                    // so it needs a page.
                    self.ctx.forced.insert(slug.clone());
                }
            } else {
                self.todos.push(format!(
                    "part `{pid}` names lens `{slug}`, which isn't there"
                ));
            }
        }
        let page = self.ctx.page_ref(r);
        self.embedded.push(format!(
            "{ext}/{slug} in {} at `{id}` as {page}",
            self.page_slug
        ));
        let mut node = Mapping::new();
        node.insert(s("id"), s(&id));
        node.extend(head);
        let mut e = Mapping::new();
        e.insert(s("page"), s(&page));
        node.insert(s("embed"), Value::Mapping(e));
        Some(Value::Mapping(node))
    }

    fn rewrite_columns(&self, cols: &Value) -> Value {
        let Some(seq) = cols.as_sequence() else {
            return cols.clone();
        };
        Value::Sequence(
            seq.iter()
                .map(|c| {
                    let mut c = c.clone();
                    if let Some(m) = c.as_mapping_mut() {
                        if let Some(e) = get_str(m, "expand").map(str::to_string) {
                            m.insert(s("expand"), s(&self.ctx.page_ref(&e)));
                        }
                    }
                    c
                })
                .collect(),
        )
    }

    fn rewrite_choices(&self, m: &mut Mapping) {
        if let Some(ch) = m.get_mut("choices").and_then(Value::as_mapping_mut) {
            for v in ch.values_mut() {
                if let Some(r) = v.as_str().map(str::to_string) {
                    *v = s(&self.ctx.page_ref(&r));
                }
            }
        }
    }

    fn rewrite_form(&self, form: &Value) -> Value {
        let mut form = form.clone();
        if let Some(m) = form.as_mapping_mut() {
            self.rewrite_choices(m);
            if let Some(fields) = m.get_mut("fields").and_then(Value::as_sequence_mut) {
                for f in fields.iter_mut().filter_map(Value::as_mapping_mut) {
                    if let Some(r) = get_str(f, "from").map(str::to_string) {
                        f.insert(s("from"), s(&self.ctx.page_ref(&r)));
                    }
                }
            }
        }
        form
    }

    fn rewrite_action(&self, a: &Value) -> Value {
        let mut a = a.clone();
        if let Some(m) = a.as_mapping_mut() {
            self.rewrite_choices(m);
        }
        a
    }
}

/// A node's mapping: its `id` first, then `head` (title, replaceable,
/// size).
fn headed(id: Option<&str>, head: Mapping) -> Mapping {
    let mut node = Mapping::new();
    if let Some(id) = id {
        node.insert(s("id"), s(id));
    }
    node.extend(head);
    node
}

fn one_line(v: &Value) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// What a page is made from.
enum Job {
    /// A `pages:` entry (its index), drawn by a lens or a component.
    Entry(usize),
    /// A lens shown as a page of its own (its page id).
    Lens(String, String),
}

/// Convert extension `ext`; `others` are the extensions that may mount
/// into it or name its lenses.
pub fn convert(ext: &Source, others: &[Source]) -> Result<Conversion, String> {
    let ext = Ext::load(ext)?;
    let others: Vec<Ext> = others.iter().filter_map(|o| Ext::load(o).ok()).collect();
    let mut all: Vec<&Ext> = vec![&ext];
    all.extend(others.iter().filter(|o| o.name != ext.name));
    let mut report = Report::default();
    for (slug, e) in &ext.broken {
        report.todos.push(format!(
            "lenses/{slug}.yaml doesn't parse, not converted: {e}"
        ));
    }

    // Every reference to this extension's lenses.
    let mut uses: BTreeMap<String, Vec<Use>> = BTreeMap::new();
    for e in &all {
        let mine = e.name == ext.name;
        let mut add = |r: &str, kind: UseKind, host: Option<&str>| {
            let (x, slug) = e.qualify(r);
            if x == ext.name {
                uses.entry(slug).or_default().push(Use {
                    kind,
                    host: host.filter(|_| mine).map(str::to_string),
                });
            }
        };
        for (slug, lens) in &e.lenses {
            for (r, kind) in lens_refs(lens) {
                add(&r, kind, Some(slug));
            }
        }
        for m in e.ui_seq("slots").iter().filter_map(Value::as_mapping) {
            if let Some(l) = get_str(m, "lens") {
                add(l, UseKind::Mount, None);
            }
        }
        for m in e
            .ui_seq("replacements")
            .iter()
            .filter_map(Value::as_mapping)
        {
            if let Some(l) = get_str(m, "lens") {
                add(l, UseKind::Mount, None);
            }
        }
        for m in get_seq(&e.manifest, "panels")
            .iter()
            .filter_map(Value::as_mapping)
        {
            for key in ["body", "collapsed", "badge", "count"] {
                if let Some(l) = get_str(m, key) {
                    add(l, UseKind::Panel, None);
                }
            }
        }
        for m in e.entries().iter().filter_map(Value::as_mapping) {
            let lens = get_str(m, "lens");
            if let Some(l) = lens {
                add(l, UseKind::Entry, None);
            }
            if let Some(l) = get_str(m, "count") {
                add(l, UseKind::Count, lens);
            }
            if let Some(l) = get_str(m, "rail") {
                add(l, UseKind::Rail, lens);
            }
        }
    }

    // What the migration deletes.
    let mut skipped: BTreeMap<String, String> = BTreeMap::new();
    for (slug, lens) in &ext.lenses {
        if SKIPPED_LENSES.contains(&slug.as_str()) || slug.starts_with("thread-answers") {
            skipped.insert(slug.clone(), "deleted by the migration".into());
        } else if viz_of(lens) == "answer" {
            skipped.insert(slug.clone(), "a `viz: answer` lens".into());
        }
    }
    let mut skipped_entries: BTreeMap<String, String> = BTreeMap::new();
    for m in ext.entries().iter().filter_map(Value::as_mapping) {
        let id = get_str(m, "id").unwrap_or_default();
        if SKIPPED_PAGES.contains(&id) {
            skipped_entries.insert(id.to_string(), "a page the migration deletes".into());
            if let Some(l) = get_str(m, "lens") {
                skipped
                    .entry(l.to_string())
                    .or_insert_with(|| format!("draws page `{id}`, which the migration deletes"));
            }
        } else if let Some(l) = get_str(m, "lens").filter(|l| skipped.contains_key(*l)) {
            skipped_entries.insert(id.to_string(), format!("drawn by `{l}`, which is skipped"));
        }
    }
    // A lens only skipped lenses use goes with them.
    loop {
        let more: Vec<String> = ext
            .lenses
            .keys()
            .filter(|slug| !skipped.contains_key(*slug))
            .filter(|slug| {
                let u = uses.get(*slug).map(Vec::as_slice).unwrap_or(&[]);
                !u.is_empty()
                    && u.iter().all(|u| {
                        u.host
                            .as_ref()
                            .is_some_and(|h| skipped.contains_key(h.as_str()))
                    })
            })
            .cloned()
            .collect();
        if more.is_empty() {
            break;
        }
        for m in more {
            skipped.insert(m, "used only by skipped lenses".into());
        }
    }
    // Panels showing skipped lenses go too.
    for m in get_seq(&ext.manifest, "panels")
        .iter()
        .filter_map(Value::as_mapping)
    {
        let refs: Vec<&str> = ["body", "collapsed", "badge", "count"]
            .iter()
            .filter_map(|k| get_str(m, k))
            .collect();
        if refs.iter().any(|r| skipped.contains_key(*r)) {
            report.manifest_suggestions.push(format!(
                "panels: {} -> remove (it shows a lens the migration deletes)",
                one_line(&Value::Mapping(m.clone()))
            ));
        }
    }

    let entry_ids: BTreeMap<String, Option<String>> = ext
        .entries()
        .iter()
        .filter_map(Value::as_mapping)
        .filter_map(|m| {
            Some((
                get_str(m, "id")?.to_string(),
                get_str(m, "lens").map(str::to_string),
            ))
        })
        .collect();
    let mut ctx = Ctx {
        ext: &ext,
        all: all.clone(),
        uses,
        skipped,
        slots: slot_targets(&all, &ext.name),
        page_of: BTreeMap::new(),
        forced: BTreeSet::new(),
    };
    let mut collisions: BTreeSet<String> = BTreeSet::new();
    for slug in ext.lenses.keys() {
        let page = match ext.entry_of(slug).and_then(|e| get_str(e, "id")) {
            Some(id) => id.to_string(),
            None if entry_ids.contains_key(slug) => {
                collisions.insert(slug.clone());
                format!("{slug}-lens")
            }
            None => slug.clone(),
        };
        ctx.page_of.insert(slug.clone(), page);
    }

    // The pages: entries first, then the lenses that need one, until an
    // embed asks for no more.
    let mut jobs: Vec<Job> = Vec::new();
    for (i, m) in ext.entries().iter().enumerate() {
        let Some(m) = m.as_mapping() else { continue };
        let id = get_str(m, "id").unwrap_or_default();
        if skipped_entries.contains_key(id) {
            continue;
        }
        if get_str(m, "lens").is_some()
            || get_str(m, "component").is_some_and(|c| !c.starts_with("oxplow:"))
        {
            jobs.push(Job::Entry(i));
        } else {
            skipped_entries.insert(id.to_string(), "a built-in page, no lens draws it".into());
        }
    }
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut seeded = false;
    loop {
        if jobs.is_empty() {
            let more: Vec<String> = ext
                .lenses
                .keys()
                .filter(|slug| !done.contains(*slug) && !ctx.skipped.contains_key(*slug))
                .filter(|slug| ext.entry_of(slug).is_none())
                .filter(|slug| {
                    if ctx.forced.contains(*slug) {
                        return true;
                    }
                    if seeded {
                        return false;
                    }
                    let inline_host = ctx
                        .uses(slug)
                        .first()
                        .and_then(|u| u.host.clone())
                        .filter(|h| ctx.inlinable(slug, h) && !ctx.skipped.contains_key(h));
                    let rail_only = ctx.uses(slug).len() == 1
                        && ctx.uses(slug)[0].kind == UseKind::Rail
                        && !ctx.lens(slug).is_some_and(|l| l.contains_key("tool"));
                    inline_host.is_none() && !ctx.count_only(slug) && !rail_only
                })
                .cloned()
                .collect();
            seeded = true;
            if more.is_empty() {
                break;
            }
            for slug in more {
                let page = ctx.page_of[&slug].clone();
                jobs.push(Job::Lens(slug, page));
            }
        }
        let job = jobs.remove(0);
        let (page_id, from) = match &job {
            Job::Entry(i) => {
                let m = ext.entries()[*i].as_mapping().cloned().unwrap_or_default();
                let id = get_str(&m, "id").unwrap_or_default().to_string();
                let from = match get_str(&m, "lens") {
                    Some(l) => format!("page entry `{id}`, lens `{l}`"),
                    None => format!("page entry `{id}`, its component"),
                };
                (id, from)
            }
            Job::Lens(slug, page) => (page.clone(), format!("lens `{slug}`")),
        };
        if let Job::Lens(slug, _) = &job {
            done.insert(slug.clone());
        }
        if let Job::Entry(i) = &job {
            if let Some(l) = ext.entries()[*i]
                .as_mapping()
                .and_then(|m| get_str(m, "lens"))
            {
                done.insert(l.to_string());
            }
        }
        let (text, todos, inlined, embedded) = build_page(&mut ctx, &job, &page_id, &collisions);
        let rel = PathBuf::from("pages").join(format!("{page_id}.yaml"));
        for t in &todos {
            report.todos.push(format!("{}: {t}", rel.display()));
        }
        report.inlined.extend(inlined);
        report.embedded.extend(embedded);
        report
            .converted
            .push(format!("{} <- {from}", rel.display()));
        files.push((rel, text));
    }

    for (slug, why) in &ctx.skipped {
        report.skipped.push(format!("{slug}: lens, {why}"));
    }
    for (id, why) in &skipped_entries {
        report.skipped.push(format!("{id}: page entry, {why}"));
    }
    suggestions(&ctx, &mut report);
    files.sort();
    report.converted.sort();
    report.todos.sort();
    Ok(Conversion { files, report })
}

/// Write the page `job` makes: its text and what it did.
fn build_page(
    ctx: &mut Ctx<'_>,
    job: &Job,
    page_id: &str,
    collisions: &BTreeSet<String>,
) -> (String, Vec<String>, Vec<String>, Vec<String>) {
    let ext = ctx.ext;
    let entry: Option<Mapping> = match job {
        Job::Entry(i) => ext.entries()[*i].as_mapping().cloned(),
        Job::Lens(..) => None,
    };
    let lens_slug: Option<String> = match job {
        Job::Entry(_) => entry
            .as_ref()
            .and_then(|e| get_str(e, "lens"))
            .map(str::to_string),
        Job::Lens(slug, _) => Some(slug.clone()),
    };
    let lens: Option<Mapping> = lens_slug.as_deref().and_then(|l| ctx.lens(l)).cloned();
    let mut b = Builder {
        ctx,
        params: Vec::new(),
        top_params: BTreeSet::new(),
        loaders: Vec::new(),
        node_ids: BTreeSet::new(),
        needs: Vec::new(),
        refresh: false,
        actions: Vec::new(),
        todos: Vec::new(),
        inlined: Vec::new(),
        embedded: Vec::new(),
        page_slug: page_id.to_string(),
    };
    if let Some(slug) = lens_slug.as_deref() {
        if collisions.contains(slug) {
            b.todos.push(format!(
                "lens `{slug}` is named like page entry `{slug}`, so its page is `{page_id}`: rename one"
            ));
        }
        if lens.is_none() {
            b.todos.push(format!("lens `{slug}` isn't there"));
        }
    }
    if let Some(l) = &lens {
        for p in get_seq(l, "params").iter().filter_map(Value::as_mapping) {
            if let Some(n) = get_str(p, "name") {
                b.top_params.insert(n.to_string());
            }
            b.add_param(p, lens_slug.as_deref().unwrap_or_default());
        }
    }
    if let Some(e) = &entry {
        for n in get_seq(e, "params").iter().filter_map(Value::as_str) {
            b.top_params.insert(n.to_string());
            if !b.params.iter().any(|p| get_str(p, "name") == Some(n)) {
                let mut p = Mapping::new();
                p.insert(s("name"), s(n));
                b.params.push(p);
            }
        }
    }

    let view = match (&lens, &entry) {
        (Some(l), _) => {
            let slug = lens_slug.clone().unwrap_or_default();
            b.lens_node(&slug, l, Mapping::new(), &slug, &Vec::new(), true, None)
        }
        (None, Some(e)) => {
            let mut c = Mapping::new();
            c.insert(s("id"), s(get_str(e, "component").unwrap_or_default()));
            let mut node = Mapping::new();
            node.insert(s("component"), Value::Mapping(c));
            Value::Mapping(node)
        }
        (None, None) => Value::Mapping(Mapping::new()),
    };
    let rail = entry
        .as_ref()
        .and_then(|e| get_str(e, "rail"))
        .map(|r| r.to_string())
        .map(|r| {
            let rail_lens = b.ctx.lens(&r).cloned();
            match rail_lens {
                Some(rl) if b.ctx.uses(&r).len() == 1 && !rl.contains_key("tool") => {
                    b.inlined.push(format!("{r} into {page_id} as its rail"));
                    for p in get_seq(&rl, "params").iter().filter_map(Value::as_mapping) {
                        b.add_param(p, &r);
                    }
                    let mut head = Mapping::new();
                    head.insert(s("id"), s(&r));
                    b.lens_node(&r, &rl, head, &r, &Vec::new(), false, Some(&r))
                }
                _ => {
                    let page = b.ctx.page_ref(&r);
                    b.embedded.push(format!("{r} in {page_id} as its rail"));
                    let mut e = Mapping::new();
                    e.insert(s("page"), s(&page));
                    let mut node = Mapping::new();
                    node.insert(s("embed"), Value::Mapping(e));
                    Value::Mapping(node)
                }
            }
        });

    // The badge.
    let mut badge = None;
    if let Some(count) = entry.as_ref().and_then(|e| get_str(e, "count")) {
        match b.ctx.lens(count).cloned() {
            Some(cl) => {
                for p in get_seq(&cl, "params").iter().filter_map(Value::as_mapping) {
                    b.add_param(p, count);
                }
                b.add_needs(&cl);
                let mut body = Mapping::new();
                let q = get_str(&cl, "query").unwrap_or("").trim().to_string();
                body.insert(s("sql"), s(&format!("{q}\n")));
                let name = b.add_loader("count", body);
                let mut m = Mapping::new();
                m.insert(s("loader"), s(&name));
                badge = Some(Value::Mapping(m));
            }
            None => b.todos.push(format!("count lens `{count}` isn't there")),
        }
    }

    // The page, its keys in their fixed order.
    let mut page = Mapping::new();
    let pick = |k: &str| -> Option<Value> {
        entry
            .as_ref()
            .and_then(|e| get(e, k))
            .cloned()
            .or_else(|| lens.as_ref().and_then(|l| get(l, k)).cloned())
    };
    page.insert(s("title"), pick("title").unwrap_or_else(|| s(page_id)));
    if let Some(d) = lens.as_ref().and_then(|l| get(l, "description")) {
        if d.as_str().is_some_and(|d| !d.is_empty()) {
            page.insert(s("description"), d.clone());
        }
    }
    let from_entry = |k: &str| entry.as_ref().and_then(|e| get(e, k)).cloned();
    for k in ["label", "icon"] {
        if let Some(v) = from_entry(k) {
            page.insert(s(k), v);
        }
    }
    let launcher = lens
        .as_ref()
        .and_then(|l| get_map(l, "launcher"))
        .and_then(|l| get(l, "category"))
        .cloned();
    if let Some(c) = from_entry("category").or(launcher.clone()) {
        page.insert(s("category"), c);
    }
    for k in [
        "keywords", "for", "accepts", "default", "pinned", "comments",
    ] {
        if let Some(v) = from_entry(k) {
            page.insert(s(k), v);
        }
    }
    let refresh = b.refresh || entry.as_ref().is_some_and(|e| truthy(e, "refresh"));
    if refresh {
        page.insert(s("refresh"), Value::Bool(true));
    }
    if entry.is_none() && launcher.is_none() {
        page.insert(s("hidden"), Value::Bool(true));
    }
    if !b.needs.is_empty() {
        page.insert(
            s("needs"),
            Value::Sequence(b.needs.iter().map(|n| s(n)).collect()),
        );
    }
    if let Some(t) = lens.as_ref().and_then(|l| get(l, "tool")) {
        page.insert(s("tool"), t.clone());
    }
    if !b.params.is_empty() {
        page.insert(
            s("params"),
            Value::Sequence(b.params.iter().cloned().map(Value::Mapping).collect()),
        );
    }
    if !b.loaders.is_empty() {
        let mut data = Mapping::new();
        for (name, body) in &b.loaders {
            data.insert(s(name), Value::Mapping(body.clone()));
        }
        page.insert(s("data"), Value::Mapping(data));
    }
    page.insert(s("view"), view);
    if let Some(r) = rail {
        page.insert(s("rail"), r);
    }
    if !b.actions.is_empty() {
        page.insert(s("actions"), Value::Sequence(b.actions.clone()));
    }
    if let Some(a) = lens.as_ref().and_then(|l| get(l, "alert")) {
        page.insert(s("alert"), a.clone());
    }
    if let Some(bd) = badge {
        page.insert(s("badge"), bd);
    }

    let body = serde_yaml::to_string(&Value::Mapping(page)).unwrap_or_default();
    let mut todos = b.todos;
    if todos.is_empty() {
        let rel = format!("pages/{page_id}.yaml");
        if let Err(e) = parse_page(&ext.name, &ext.namespace, page_id, &rel, &body) {
            todos.push(format!("does not parse yet: {e}"));
        }
    }
    let mut text = String::new();
    for t in &todos {
        text.push_str(&format!("# TODO(convert): {}\n", t.replace('\n', " ")));
    }
    text.push_str(&body);
    (text, todos, b.inlined, b.embedded)
}

/// The manifest's lens references, as they would name pages.
fn suggestions(ctx: &Ctx<'_>, report: &mut Report) {
    let ext = ctx.ext;
    let page = |r: &str| -> String {
        let p = ctx.page_ref(r);
        match p.split_once('/') {
            Some((e, id)) if e == ext.name => id.to_string(),
            _ => p,
        }
    };
    for m in ext.ui_seq("slots").iter().filter_map(Value::as_mapping) {
        let Some(l) = get_str(m, "lens") else {
            continue;
        };
        let mut new = Mapping::new();
        for (k, v) in m {
            match k.as_str() {
                Some("lens") => {
                    new.insert(s("page"), s(&page(l)));
                }
                Some("slot") => {
                    let slot = v.as_str().unwrap_or_default();
                    let renamed = match slot.split_once('/') {
                        Some((e, rest)) => match rest.rsplit_once('.') {
                            Some((slug, kind)) => {
                                format!("{e}/{}.{kind}", ctx.page_id_in(e, slug))
                            }
                            None => slot.to_string(),
                        },
                        None => slot.to_string(),
                    };
                    new.insert(s("slot"), s(&renamed));
                }
                _ => {
                    new.insert(k.clone(), v.clone());
                }
            }
        }
        report.manifest_suggestions.push(format!(
            "ui.slots: {} -> {}",
            one_line(&Value::Mapping(m.clone())),
            one_line(&Value::Mapping(new))
        ));
    }
    for m in ext
        .ui_seq("replacements")
        .iter()
        .filter_map(Value::as_mapping)
    {
        let Some(l) = get_str(m, "lens") else {
            continue;
        };
        let mut new = Mapping::new();
        for (k, v) in m {
            if k.as_str() == Some("lens") {
                new.insert(s("page"), s(&page(l)));
            } else {
                new.insert(k.clone(), v.clone());
            }
        }
        report.manifest_suggestions.push(format!(
            "ui.replacements: {} -> {}",
            one_line(&Value::Mapping(m.clone())),
            one_line(&Value::Mapping(new))
        ));
    }
    for m in get_seq(&ext.manifest, "panels")
        .iter()
        .filter_map(Value::as_mapping)
    {
        let keys = ["body", "collapsed", "badge", "count"];
        if keys
            .iter()
            .filter_map(|k| get_str(m, k))
            .any(|r| ctx.skipped.contains_key(r))
        {
            continue;
        }
        let mut new = m.clone();
        for k in keys {
            if let Some(r) = get_str(m, k) {
                new.insert(s(k), s(&page(r)));
            }
        }
        report.manifest_suggestions.push(format!(
            "panels: {} -> {} (each names a page)",
            one_line(&Value::Mapping(m.clone())),
            one_line(&Value::Mapping(new))
        ));
    }
    for m in ext.entries().iter().filter_map(Value::as_mapping) {
        let id = get_str(m, "id").unwrap_or_default();
        if get_str(m, "lens").is_some() && !SKIPPED_PAGES.contains(&id) {
            report.manifest_suggestions.push(format!(
                "pages: `{id}` -> drop the entry: pages/{id}.yaml declares it"
            ));
        }
    }
}
