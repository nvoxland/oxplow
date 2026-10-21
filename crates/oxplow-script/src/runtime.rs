//! Layer-2 **transform runtimes** — the engines that run a collector's
//! field-mapping step and produce the kind's typed output.
//!
//! Three tiers, in trust/preference order:
//! - [`run_jaq`] — jq via `jaq` (pure Rust). Primary: JSON→JSON reshaping.
//! - [`run_starlark`] — Starlark (pure Rust). General/imperative tier.
//! - [`run_exec`] — an external process (JSON on stdin, JSON on stdout). The
//!   escape hatch; lower-trust (it can do I/O), tagged so the UI can mark it.
//!
//! The two script tiers are deterministic and do no I/O, so their output is
//! `observed`-eligible. Their engines here run only inside a script worker
//! process (`crate::worker`), which the host kills past its
//! [`Budget`](oxplow_domain::run::Budget) or memory cap (`crate::workers`). [`run_exec`] is a
//! child process of its own and honours the same budget. It also drains
//! stdout/stderr on their own threads while writing stdin, because doing
//! those in sequence deadlocks any streaming filter once a pipe buffer fills
//! (tsk161).
//!
//! Every engine takes an already-parsed JSON input value (the host applied the
//! collector's declared container parser first — see `CollectorInput`) and
//! returns a JSON value, which the caller deserializes into the kind's typed
//! output via [`value_to_output`].

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;

use oxplow_coverage::{
    AnalysisFinding, AnalysisReport, CoverageReport, FileCoverage, Severity, TestCase, TestReport,
    TestStatus, TestSuite,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{CollectError, CollectorKind, CollectorOutput};
use oxplow_domain::run::Budget;

/// A run's time spent waiting on its host — answering a command's
/// `scope(…)` or a collector's `ai_*` call (the calls finished and the one
/// in flight) — which its budget's `timeout` leaves out and its `ceiling`
/// doesn't.
#[derive(Debug, Default)]
pub struct RunClock {
    state: std::sync::Mutex<(Duration, Option<std::time::Instant>)>,
}

impl RunClock {
    /// Run `f` with the clock paused.
    pub fn paused<T>(&self, f: impl FnOnce() -> T) -> T {
        self.lock().1 = Some(std::time::Instant::now());
        let out = f();
        let mut st = self.lock();
        if let Some(since) = st.1.take() {
            st.0 += since.elapsed();
        }
        out
    }

    /// Everything paused so far, the call in flight included.
    pub fn total(&self) -> Duration {
        let st = self.lock();
        st.0 + st.1.map(|s| s.elapsed()).unwrap_or_default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, (Duration, Option<std::time::Instant>)> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Run a jq program (`jaq`) against `input`, returning its single output value.
/// More or fewer than one output is an error — a collector maps to exactly one
/// report object.
pub fn run_jaq(program: &str, input: &Value) -> Result<Value, CollectError> {
    use jaq_core::load::{Arena, File, Loader};
    use jaq_core::{data, Compiler, Ctx, Vars};
    use jaq_json::Val;

    let input_bytes =
        serde_json::to_vec(input).map_err(|e| CollectError::Runtime(e.to_string()))?;
    let input_val = jaq_json::read::parse_single(input_bytes.as_slice())
        .map_err(|e| CollectError::Runtime(format!("jaq input parse: {e}")))?;

    let program_file = File {
        code: program,
        path: (),
    };
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    // Drop jaq-std's impure natives so a collection transform stays
    // deterministic and does no I/O — its output is trusted as `observed`.
    // We keep the full `funs()` (not `base_funs()`) because jaq-std's *defs*
    // depend on pure extras like `split_`/`matches`/`pow`; we only filter the
    // ones that read host state. A `Fun` is `(name, arity, impl)`, so `f.0` is
    // the name. None of these are referenced by jaq-std defs, so removing them
    // doesn't break the library — a program calling them just fails to compile.
    const IMPURE_JAQ_FUNS: &[&str] = &["env", "now", "localtime", "input", "inputs"];
    let funs = jaq_core::funs()
        .chain(jaq_std::funs().filter(|f| !IMPURE_JAQ_FUNS.contains(&f.0)))
        .chain(jaq_json::funs());

    let loader = Loader::new(defs);
    let arena = Arena::default();
    let modules = loader
        .load(&arena, program_file)
        .map_err(|e| CollectError::Runtime(format!("jaq compile: {e:?}")))?;
    let filter = Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|e| CollectError::Runtime(format!("jaq compile: {e:?}")))?;

    let ctx = Ctx::<data::JustLut<Val>>::new(&filter.lut, Vars::new([]));
    let mut outputs = filter.id.run((ctx, input_val)).map(jaq_core::unwrap_valr);

    let first = outputs
        .next()
        .ok_or_else(|| CollectError::Runtime("jaq produced no output".into()))?
        .map_err(|e| CollectError::Runtime(format!("jaq run: {e}")))?;
    if outputs.next().is_some() {
        return Err(CollectError::Runtime(
            "jaq produced more than one output (expected a single report object)".into(),
        ));
    }
    // jaq_json::Val's Display is its JSON serialization.
    serde_json::from_str(&first.to_string()).map_err(|e| CollectError::Shape(e.to_string()))
}

/// Native Starlark builtins exposing the layer-1 container-parse helpers, so a
/// Starlark script can parse raw text itself (e.g. `input: text` + bespoke
/// logic) instead of relying only on the host pre-parse. Each returns a real
/// Starlark value — starlark implements `AllocValue` for `serde_json::Value`,
/// so `heap.alloc(...)` does the conversion. (jaq can't call host functions,
/// which is why the bundled parsers pre-parse via `CollectorInput` instead.)
#[starlark::starlark_module]
fn collect_helpers(builder: &mut starlark::environment::GlobalsBuilder) {
    fn parse_xml<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::parse_xml(content).map_err(helper_anyhow)?))
    }
    fn parse_json<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::parse_json(content).map_err(helper_anyhow)?))
    }
    fn lcov_records<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::lcov_records(content)))
    }
    /// `text` as one inert line of markdown (`helpers::md_text`): for
    /// splicing someone else's text into a comment or checklist.
    fn md_text(
        #[starlark(require = pos)] text: starlark::values::Value,
        #[starlark(require = named, default = 300)] limit: u32,
    ) -> anyhow::Result<String> {
        let text = match text.unpack_str() {
            Some(s) => s.to_string(),
            None => text.to_str(),
        };
        Ok(crate::helpers::md_text(&text, limit as usize))
    }
    fn lines<'v>(
        content: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::lines(content)))
    }
    fn regex_find<'v>(
        pattern: &str,
        text: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::regex_find(pattern, text).map_err(helper_anyhow)?))
    }
    fn xpath<'v>(
        content: &str,
        expr: &str,
        heap: starlark::values::Heap<'v>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        Ok(heap.alloc(crate::helpers::xpath(content, expr).map_err(helper_anyhow)?))
    }
    /// Parse `text` as `language` and run a tree-sitter S-expression `query`,
    /// returning a flat list of `{capture, text, start_row, start_col, end_row,
    /// end_col}` matches. Pure (text passed inline → deterministic, `observed`).
    fn ast_query<'v>(
        text: &str,
        language: &str,
        query: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let value = Hosts::memoized(eval, &["ast_query", language, query], text, || {
            let matches = oxplow_code_metrics::ast_query(text, language, query)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let arr: Vec<serde_json::Value> = matches
                .into_iter()
                .map(|m| {
                    serde_json::json!({
                        "capture": m.capture,
                        "text": m.text,
                        "start_row": m.start_row,
                        "start_col": m.start_col,
                        "end_row": m.end_row,
                        "end_col": m.end_col,
                    })
                })
                .collect();
            Ok(serde_json::Value::Array(arr))
        })?;
        Ok(eval.heap().alloc(value))
    }
    /// Per-function code metrics for `text` (the file at `path`) in
    /// `language` — a flat list of `{name, id, ref, container, kind,
    /// complexity, length, parameter_count, start_line, end_line,
    /// visibility, has_doc}`. `id` is the symbol's id within the file,
    /// `ref` its `symbol:<path>/<id>` ref (a fact's subject), `container`
    /// the `::`-joined names it sits in (or None), `kind` its symbol kind. Backed by `oxplow_code_metrics`' tree-sitter walker (the
    /// cyclomatic-complexity / length / param computations that the in-process
    /// code-quality producer used), exposed generically so a bundled or
    /// user-authored metric can project them. Pure → `observed`.
    fn code_metrics<'v>(
        text: &str,
        language: &str,
        path: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let value = Hosts::memoized(eval, &["code_metrics", language, path], text, || {
            let lang = oxplow_code_metrics::language_from_name(language)
                .ok_or_else(|| anyhow::anyhow!("unknown language \"{language}\""))?;
            let fns = oxplow_code_metrics::analyze_with_language(path, text, lang);
            let arr: Vec<serde_json::Value> = fns
            .into_iter()
            .map(|m| {
                let visibility = match m.visibility {
                    oxplow_code_metrics::Visibility::Public => "public",
                    oxplow_code_metrics::Visibility::Private => "private",
                    oxplow_code_metrics::Visibility::Unknown => "unknown",
                };
                serde_json::json!({
                    "name": m.name,
                    "id": m.id,
                    "ref": oxplow_domain::uri::build::symbol_uri(path, &m.id),
                    "container": (!m.container_path.is_empty()).then(|| m.container_path.join("::")),
                    "kind": m.kind.as_str(),
                    "complexity": m.complexity,
                    "length": m.length,
                    "parameter_count": m.parameter_count,
                    "start_line": m.start_line,
                    "end_line": m.end_line,
                    "visibility": visibility,
                    "has_doc": m.has_doc,
                })
            })
            .collect();
            Ok(serde_json::Value::Array(arr))
        })?;
        Ok(eval.heap().alloc(value))
    }
    /// Return the snapshot files matching `glob` as `[{path, text}]`, read from
    /// the run's [`TreeHost`]. A run given no tree (one that didn't declare
    /// `tree.read`) is told so; a malformed glob is an error.
    fn files<'v>(
        glob: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let matcher = globset::Glob::new(glob)
            .map_err(|e| anyhow::anyhow!("bad glob \"{glob}\": {e}"))?
            .compile_matcher();
        let host = Hosts::tree(eval, "files")?;
        let mut entries: Vec<(&String, &String)> = host
            .files
            .iter()
            .filter(|(path, _)| matcher.is_match(path.as_str()))
            .collect();
        // Deterministic order so a gauge's output is reproducible.
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let arr: Vec<serde_json::Value> = entries
            .into_iter()
            .map(|(path, text)| serde_json::json!({ "path": path, "text": text }))
            .collect();
        Ok(eval.heap().alloc(serde_json::Value::Array(arr)))
    }

    /// The language-agnostic **reader**: every *recognized* source file from the
    /// host, each tagged with its detected language —
    /// `[{path, text, language}]`. Files whose extension isn't a supported
    /// language are skipped. A metric script sweeps this and calls a capability
    /// (`code_metrics` / `markers`) per file, never naming a language itself.
    fn source_files<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let host = Hosts::tree(eval, "source_files")?;
        let mut entries: Vec<(&String, &String)> = host
            .files
            .iter()
            .filter(|(path, _)| oxplow_code_metrics::is_supported_path(path.as_str()))
            // Codegen output (generated/ path, do-not-edit header) is
            // not hand-written code — keep it out of the code-shape
            // corpus so a 3k-line bindings file can't dominate the
            // fn_length/complexity tails (tsk68).
            .filter(|(path, text)| !oxplow_code_metrics::is_generated_source(path.as_str(), text))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let arr: Vec<serde_json::Value> = entries
            .into_iter()
            .filter_map(|(path, text)| {
                oxplow_code_metrics::language_for_path(path).map(|lang| {
                    serde_json::json!({ "path": path, "text": text, "language": lang.name() })
                })
            })
            .collect();
        Ok(eval.heap().alloc(serde_json::Value::Array(arr)))
    }

    /// The duplicate-block capability: every pair of duplicated blocks of
    /// at least `min_lines` lines among the host's files —
    /// `[{a_path, a_start_line, a_end_line, b_path, b_start_line,
    /// b_end_line, line_count}]`, from `oxplow_code_dup::detect_duplicates`.
    /// Files in no supported language are skipped.
    fn duplicate_blocks<'v>(
        min_lines: u32,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let host = Hosts::tree(eval, "duplicate_blocks")?;
        let mut files: Vec<(&String, &String)> = host.files.iter().collect();
        files.sort_by(|a, b| a.0.cmp(b.0));
        let opts = oxplow_code_dup::DupOptions {
            min_lines,
            ..Default::default()
        };
        let arr: Vec<serde_json::Value> = oxplow_code_dup::detect_duplicates(
            files.into_iter().map(|(p, t)| (p.clone(), t)),
            opts,
        )
        .into_iter()
        .map(|b| {
            serde_json::json!({
                "a_path": b.a_path,
                "a_start_line": b.a_start_line,
                "a_end_line": b.a_end_line,
                "b_path": b.b_path,
                "b_start_line": b.b_start_line,
                "b_end_line": b.b_end_line,
                "line_count": b.line_count,
            })
        })
        .collect();
        Ok(eval.heap().alloc(serde_json::Value::Array(arr)))
    }

    /// The comment-marker capability: `[{line, kind, text}]` for the
    /// TODO/FIXME/HACK/XXX/BUG markers in `text`, comment-aware via the
    /// language grammar. Dispatches into `oxplow_code_metrics::markers`.
    fn markers<'v>(
        text: &str,
        language: &str,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<starlark::values::Value<'v>> {
        let value = Hosts::memoized(eval, &["markers", language], text, || {
            let lang = oxplow_code_metrics::language_from_name(language)
                .ok_or_else(|| anyhow::anyhow!("unknown language \"{language}\""))?;
            let arr: Vec<serde_json::Value> = oxplow_code_metrics::markers(text, lang)
                .into_iter()
                .map(|m| serde_json::json!({ "line": m.line, "kind": m.kind, "text": m.text }))
                .collect();
            Ok(serde_json::Value::Array(arr))
        })?;
        Ok(eval.heap().alloc(value))
    }
}

/// The captured tree a run reads (`tree.read`): what `files(glob)`,
/// `source_files()` and `duplicate_blocks()` answer from. The file map
/// (path → content) is held behind an `Arc` so the *same* captured snapshot
/// can be shared across every collector of a run without cloning the whole
/// map per collector (a real cost when many run on one snapshot). It carries no borrow lifetime across the
/// Starlark boundary and stays `Send + 'static` (it can move into the sandbox
/// worker thread).
#[derive(Debug, Default)]
pub struct TreeHost {
    files: std::sync::Arc<std::collections::HashMap<String, String>>,
}

impl TreeHost {
    /// A host exposing `files` (repo-relative path → UTF-8 content) to
    /// `files(glob)`, owning the map.
    pub fn new(files: std::collections::HashMap<String, String>) -> Self {
        Self {
            files: std::sync::Arc::new(files),
        }
    }

    /// A host sharing an already-`Arc`'d file map — the per-collector
    /// construction path, so N collectors over one snapshot share one map (a cheap refcount bump
    /// each) instead of N full clones.
    pub fn from_shared(files: std::sync::Arc<std::collections::HashMap<String, String>>) -> Self {
        Self { files }
    }
}

fn helper_anyhow(e: crate::HelperError) -> anyhow::Error {
    anyhow::anyhow!(e.to_string())
}

/// Whether `script` (the file at `path`, which parse errors name) parses
/// as Starlark and defines `transform` at the top level of its module —
/// what a loader checks before anything runs it. A message saying what's
/// wrong.
pub fn check_starlark(path: &str, script: &str) -> Result<(), String> {
    check_starlark_defines(path, script, &["transform"])
}

/// Whether `script` parses and defines each of `functions` at the top
/// level of its module; a message naming the first one missing.
pub fn check_starlark_defines(path: &str, script: &str, functions: &[&str]) -> Result<(), String> {
    use starlark::syntax::ast::StmtP;
    use starlark::syntax::{AstModule, Dialect};
    let ast = AstModule::parse(path, script.to_string(), &Dialect::Standard)
        .map_err(|e| format!("doesn't parse: {e}"))?;
    let top = match &ast.statement().node {
        StmtP::Statements(stmts) => stmts.iter().collect::<Vec<_>>(),
        _ => vec![ast.statement()],
    };
    for name in functions {
        let defines = top
            .iter()
            .any(|s| matches!(&s.node, StmtP::Def(d) if d.name.ident == *name));
        if !defines {
            return Err(format!(
                "must define `{name}` (`def {name}(x):`) at the top level"
            ));
        }
    }
    Ok(())
}

/// Run a Starlark script against `input`. The script must define
/// `def transform(input): … return <object>`; the host appends a call that
/// JSON-encodes the result, so the return value crosses back as JSON. The
/// `json` stdlib extension and the container-parse helpers
/// (`parse_xml`/`parse_json`/`lcov_records`/`lines`/`regex_find`/`xpath`) are
/// available to the script. It has no services: a builtin that needs one
/// says so.
pub fn run_starlark(script: &str, input: &Value) -> Result<Value, CollectError> {
    run_starlark_inner(script, "transform", input, &Hosts::default())
}

/// [`run_starlark`], calling the script's function `func` instead of
/// `transform`, with no services.
pub fn run_starlark_fn(script: &str, func: &str, input: &Value) -> Result<Value, CollectError> {
    run_starlark_inner(script, func, input, &Hosts::default())
}

/// [`run_starlark`] with the services `hosts` gives it.
pub fn run_starlark_hosted(
    script: &str,
    input: &Value,
    hosts: &Hosts,
) -> Result<Value, CollectError> {
    run_starlark_inner(script, "transform", input, hosts)
}

/// The services a run was given (`.context/extensions.md` "Services"),
/// the one thing in its `Evaluator::extra`: each builtin takes its own,
/// and one whose service wasn't given says which need would give it.
#[derive(Default, starlark::any::ProvidesStaticType)]
pub struct Hosts {
    pub tree: Option<TreeHost>,
    pub ai: Option<crate::ai::AiHost>,
    pub scope: Option<crate::scope::ScopeHost>,
    /// Where parse results are shared across runs (a worker's line to its
    /// host); none in process, where every call computes.
    pub parses: Option<std::sync::Arc<dyn ParseMemo>>,
}

/// A store of parse results shared across runs, keyed by [`parse_key`].
/// The values are JSON text, as a builtin returns them.
pub trait ParseMemo: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn put(&self, key: &str, value: &str);
}

/// The key of a parse result: what computed it (`parts`: the builtin, the
/// language, and whatever else the answer depends on) and the content's
/// SHA-256, so an edit is a new key and an unchanged file is a hit in any
/// run, sweep or snapshot.
pub fn parse_key(parts: &[&str], text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.update(Sha256::digest(text.as_bytes()));
    hex::encode(hash.finalize())
}

impl Hosts {
    /// `compute`'s answer for (`parts`, `text`), shared through the run's
    /// [`ParseMemo`] when it has one: a file is parsed once however many
    /// scripts of a sweep read it.
    pub(crate) fn memoized(
        eval: &starlark::eval::Evaluator<'_, '_, '_>,
        parts: &[&str],
        text: &str,
        compute: impl FnOnce() -> anyhow::Result<Value>,
    ) -> anyhow::Result<Value> {
        let Some(memo) = Self::of(eval).and_then(|h| h.parses.as_ref()) else {
            return compute();
        };
        let key = parse_key(parts, text);
        if let Some(hit) = memo
            .get(&key)
            .and_then(|json| serde_json::from_str(&json).ok())
        {
            return Ok(hit);
        }
        let value = compute()?;
        if let Ok(json) = serde_json::to_string(&value) {
            memo.put(&key, &json);
        }
        Ok(value)
    }
}

impl Hosts {
    fn of<'a>(eval: &starlark::eval::Evaluator<'_, 'a, '_>) -> Option<&'a Hosts> {
        eval.extra.and_then(|e| e.downcast_ref::<Hosts>())
    }

    /// `pick`'s host for `builtin`, or the need it wants.
    pub(crate) fn get<'a, T>(
        eval: &starlark::eval::Evaluator<'_, 'a, '_>,
        builtin: &str,
        pick: impl FnOnce(&'a Hosts) -> Option<&'a T>,
    ) -> anyhow::Result<&'a T> {
        Self::of(eval)
            .and_then(pick)
            .ok_or_else(|| anyhow::anyhow!(oxplow_domain::service::undeclared(builtin)))
    }

    fn tree<'a>(
        eval: &starlark::eval::Evaluator<'_, 'a, '_>,
        builtin: &str,
    ) -> anyhow::Result<&'a TreeHost> {
        Self::get(eval, builtin, |h| h.tree.as_ref())
    }
}

pub(crate) fn run_starlark_inner(
    script: &str,
    func: &str,
    input: &Value,
    hosts: &Hosts,
) -> Result<Value, CollectError> {
    use starlark::environment::{GlobalsBuilder, LibraryExtension, Module};
    use starlark::eval::Evaluator;
    use starlark::syntax::{AstModule, Dialect};

    // Embed the input as a JSON document, then as a JSON string *literal*
    // (serializing a String yields a quoted, escaped form that is also a valid
    // Starlark string literal), so the script gets it via json.decode.
    let input_doc =
        serde_json::to_string(input).map_err(|e| CollectError::Runtime(e.to_string()))?;
    let input_literal =
        serde_json::to_string(&input_doc).map_err(|e| CollectError::Runtime(e.to_string()))?;
    let source = format!("{script}\njson.encode({func}(json.decode({input_literal})))\n");

    let ast = AstModule::parse("transform.star", source, &Dialect::Standard)
        .map_err(|e| CollectError::Runtime(format!("starlark parse: {e}")))?;
    let globals = GlobalsBuilder::extended_by(&[LibraryExtension::Json])
        .with(collect_helpers)
        .with(crate::ai::ai_builtins)
        .with(crate::scope::scope_builtins)
        .with(crate::uri::uri_builtins)
        .build();

    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.extra = Some(hosts);
        let result = eval
            .eval_module(ast, &globals)
            .map_err(|e| CollectError::Runtime(format!("starlark run: {e}")))?;
        let json = result.unpack_str().ok_or_else(|| {
            CollectError::Shape("starlark transform did not return a JSON string".into())
        })?;
        serde_json::from_str(json).map_err(|e| CollectError::Shape(e.to_string()))
    })
}

/// Run an external program (the escape hatch). `argv[0]` is the program; the
/// raw report `content` is written to its stdin and its stdout is parsed as the
/// kind's JSON output. Unlike the in-process tiers this can do I/O, so callers
/// tag it lower-trust.
pub fn run_exec(budget: &Budget, argv: &[String], content: &str) -> Result<Value, CollectError> {
    run_exec_authorized(budget, argv, content, &|| Ok(()))
}

/// External execution under a live host authority check.
pub fn run_exec_authorized(
    budget: &Budget,
    argv: &[String],
    content: &str,
    check: &dyn Fn() -> Result<(), String>,
) -> Result<Value, CollectError> {
    run_exec_authorized_at(budget, argv, content, check, None)
}

/// Host-owned code location. The caller keeps the deployment's copy alive through
/// its authority guard; project data lives at a separate, explicit path.
#[derive(Debug, Clone)]
pub struct ExecLocation {
    pub directory: std::path::PathBuf,
    pub project: std::path::PathBuf,
}

/// Run from the host's verified code copy under the same live authority.
pub fn run_exec_authorized_at(
    budget: &Budget,
    argv: &[String],
    content: &str,
    check: &dyn Fn() -> Result<(), String>,
    location: Option<&ExecLocation>,
) -> Result<Value, CollectError> {
    check().map_err(CollectError::Exec)?;
    use std::io::Read;
    use std::process::{Command, Stdio};

    let (program, args) = argv
        .split_first()
        .ok_or_else(|| CollectError::Exec("empty exec argv".into()))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(location) = location {
        command
            .current_dir(&location.directory)
            .env("OXPLOW_PROJECT_DIR", &location.project)
            .env("OXPLOW_CODE_DIR", &location.directory);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    check().map_err(CollectError::Exec)?;
    let mut child = command
        .spawn()
        .map_err(|e| CollectError::Exec(format!("spawn {program}: {e}")))?;

    struct Group(Option<u32>);
    impl Group {
        fn stop(&mut self) {
            let pid = self.0.take();
            #[cfg(not(unix))]
            let _ = pid;
            #[cfg(unix)]
            if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()).filter(|p| *p > 0) {
                // SAFETY: the host created this child's process group.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
            }
        }
    }
    impl Drop for Group {
        fn drop(&mut self) {
            self.stop();
        }
    }
    let mut group = Group(Some(child.id()));

    // Every pipe gets its own thread (tsk161). Writing the whole report to
    // stdin and only THEN calling `wait_with_output` deadlocks any streaming
    // filter: once the child's stdout pipe fills (~64 KB) the child blocks
    // writing, so it stops reading stdin, so the parent blocks writing — and
    // neither side ever moves. Reproduced with 4 MB through `cat`.
    //
    // The stdin write needs a thread of its own too: a child that never reads
    // stdin blocks the writer regardless of how diligently we drain stdout, and
    // the caller must stay free to enforce the deadline below.
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let deadline = std::time::Instant::now() + budget.timeout;
    let payload = content.to_string();
    let writer = std::thread::spawn(move || match stdin {
        Some(pipe) => crate::exec_io::write(pipe, payload.as_bytes(), deadline),
        None => Ok(()),
    });
    fn drain<R: Read + crate::exec_io::Pipe + Send + 'static>(
        pipe: Option<R>,
        deadline: std::time::Instant,
    ) -> std::thread::JoinHandle<Result<Vec<u8>, String>> {
        std::thread::spawn(move || match pipe {
            Some(pipe) => crate::exec_io::read(pipe, 64 * 1024 * 1024, deadline),
            None => Ok(Vec::new()),
        })
    }
    let out_thread = drain(stdout, deadline);
    let err_thread = drain(stderr, deadline);

    let status = loop {
        if let Err(reason) = check() {
            group.stop();
            let _ = child.kill();
            let _ = child.wait();
            return Err(CollectError::Exec(reason));
        }
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    group.stop();
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(e) => {
                group.stop();
                let _ = child.kill();
                let _ = child.wait();
                return Err(CollectError::Exec(e.to_string()));
            }
        }
    };

    group.stop();
    let stdout_result = out_thread
        .join()
        .unwrap_or_else(|_| Err("output drain panicked".into()));
    let stderr_result = err_thread
        .join()
        .unwrap_or_else(|_| Err("error drain panicked".into()));
    let _ = writer.join();
    if status.is_none() {
        return Err(CollectError::Exec(format!(
            "{program} exceeded the {}s budget and was killed",
            budget.timeout.as_secs()
        )));
    }
    let stdout_bytes = stdout_result.map_err(CollectError::Exec)?;
    let stderr_bytes = stderr_result.map_err(CollectError::Exec)?;

    if stdout_bytes.len() > 64 * 1024 * 1024 || stderr_bytes.len() > 64 * 1024 * 1024 {
        return Err(CollectError::Exec(
            "external collector output exceeded 64 MiB".into(),
        ));
    }
    let Some(status) = status else {
        return Err(CollectError::Exec(format!(
            "{program} exceeded the {}s budget and was killed",
            budget.timeout.as_secs()
        )));
    };
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr_bytes);
        return Err(CollectError::Exec(format!(
            "{program} exited with {}: {}",
            status,
            stderr.trim()
        )));
    }
    check().map_err(CollectError::Exec)?;
    serde_json::from_slice(&stdout_bytes).map_err(|e| CollectError::Shape(e.to_string()))
}

// ---- typed-output deserialization (one place: JSON value → kind output) ----

/// Convert a script's raw JSON output into the typed [`CollectorOutput`] for
/// `kind`. The accepted shapes mirror what oxplow already stores:
/// `{ "files": { "<path>": { "instrumented": [..], "covered": [..] } } }` for
/// coverage and `{ "suites": [ { "name", "cases": [ { "classname", "name",
/// "status", "timeMs"? } ] } ] }` for tests.
pub fn value_to_output(kind: CollectorKind, value: Value) -> Result<CollectorOutput, CollectError> {
    match kind {
        CollectorKind::Coverage => {
            let parsed: CoverageJson =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Coverage(parsed.into()))
        }
        CollectorKind::Test => {
            let parsed: TestReportJson =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Test(parsed.into()))
        }
        CollectorKind::Analysis => {
            let parsed: AnalysisReportJson =
                serde_json::from_value(value).map_err(|e| CollectError::Shape(e.to_string()))?;
            Ok(CollectorOutput::Analysis(parsed.into()))
        }
    }
}

#[derive(Deserialize)]
struct CoverageJson {
    #[serde(default)]
    files: BTreeMap<String, FileCoverageJson>,
}

#[derive(Deserialize)]
struct FileCoverageJson {
    #[serde(default)]
    instrumented: Vec<u32>,
    #[serde(default)]
    covered: Vec<u32>,
    // Branch/function counts are optional — a line-only report omits them, and
    // they default to 0 ("no branch/function data for this file").
    #[serde(default, rename = "branchesFound")]
    branches_found: u32,
    #[serde(default, rename = "branchesHit")]
    branches_hit: u32,
    #[serde(default, rename = "functionsFound")]
    functions_found: u32,
    #[serde(default, rename = "functionsHit")]
    functions_hit: u32,
}

impl From<CoverageJson> for CoverageReport {
    fn from(j: CoverageJson) -> Self {
        let files = j
            .files
            .into_iter()
            .map(|(path, fc)| {
                (
                    path,
                    FileCoverage {
                        instrumented: fc.instrumented.into_iter().collect::<BTreeSet<u32>>(),
                        covered: fc.covered.into_iter().collect::<BTreeSet<u32>>(),
                        branches_found: fc.branches_found,
                        branches_hit: fc.branches_hit,
                        functions_found: fc.functions_found,
                        functions_hit: fc.functions_hit,
                    },
                )
            })
            .collect();
        CoverageReport { files }
    }
}

#[derive(Deserialize)]
struct TestReportJson {
    #[serde(default)]
    suites: Vec<TestSuiteJson>,
}

#[derive(Deserialize)]
struct TestSuiteJson {
    #[serde(default)]
    name: String,
    #[serde(default)]
    cases: Vec<TestCaseJson>,
}

#[derive(Deserialize)]
struct TestCaseJson {
    #[serde(default)]
    classname: String,
    #[serde(default)]
    name: String,
    status: TestStatusJson,
    #[serde(default, rename = "timeMs")]
    time_ms: Option<u64>,
    /// A failed case's message and location, as the parser read them.
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum TestStatusJson {
    Passed,
    Failed,
    Skipped,
}

impl From<TestStatusJson> for TestStatus {
    fn from(s: TestStatusJson) -> Self {
        match s {
            TestStatusJson::Passed => TestStatus::Passed,
            TestStatusJson::Failed => TestStatus::Failed,
            TestStatusJson::Skipped => TestStatus::Skipped,
        }
    }
}

#[derive(Deserialize)]
struct AnalysisReportJson {
    #[serde(default)]
    findings: Vec<AnalysisFindingJson>,
}

#[derive(Deserialize)]
struct AnalysisFindingJson {
    #[serde(default)]
    path: String,
    #[serde(default)]
    line: Option<u32>,
    #[serde(default)]
    column: Option<u32>,
    severity: SeverityJson,
    #[serde(default)]
    rule: Option<String>,
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SeverityJson {
    Error,
    Warning,
    Info,
    Note,
}

impl From<SeverityJson> for Severity {
    fn from(s: SeverityJson) -> Self {
        match s {
            SeverityJson::Error => Severity::Error,
            SeverityJson::Warning => Severity::Warning,
            SeverityJson::Info => Severity::Info,
            SeverityJson::Note => Severity::Note,
        }
    }
}

impl From<AnalysisReportJson> for AnalysisReport {
    fn from(j: AnalysisReportJson) -> Self {
        let findings = j
            .findings
            .into_iter()
            .map(|f| AnalysisFinding {
                path: f.path,
                line: f.line,
                column: f.column,
                severity: f.severity.into(),
                rule: f.rule,
                message: f.message,
            })
            .collect();
        AnalysisReport { findings }
    }
}

/// The most bytes of one failed case's message a capture keeps; a longer one
/// is cut at a character boundary and ends in `…`.
pub const MAX_MESSAGE_BYTES: usize = 4096;
/// The most message bytes a whole run's capture keeps: failures past it keep
/// their status and lose the message, so a report of a thousand failures (or
/// one huge one) can't grow the capture without bound.
pub const MAX_MESSAGE_BYTES_PER_RUN: usize = 64 * 1024;

/// `message` cut to [`MAX_MESSAGE_BYTES`], saying so.
fn capped(message: &str) -> String {
    if message.len() <= MAX_MESSAGE_BYTES {
        return message.to_string();
    }
    let mut end = MAX_MESSAGE_BYTES;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &message[..end])
}

impl From<TestReportJson> for TestReport {
    fn from(j: TestReportJson) -> Self {
        let mut budget = MAX_MESSAGE_BYTES_PER_RUN;
        let suites = j
            .suites
            .into_iter()
            .map(|s| TestSuite {
                name: s.name,
                cases: s
                    .cases
                    .into_iter()
                    .map(|c| {
                        let failed = matches!(c.status, TestStatusJson::Failed);
                        let message = c
                            .message
                            .filter(|m| failed && !m.is_empty())
                            .map(|m| capped(&m))
                            .filter(|m| m.len() <= budget)
                            .inspect(|m| budget -= m.len());
                        TestCase {
                            classname: c.classname,
                            name: c.name,
                            status: c.status.into(),
                            time_ms: c.time_ms,
                            message,
                        }
                    })
                    .collect(),
            })
            .collect();
        TestReport { suites }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A script may define several functions the host calls by name (an
    /// AI provider's `request` and `response`); the check names what's
    /// missing.
    #[test]
    fn a_script_runs_the_function_the_host_names() {
        let script = "def request(x):\n    return {\"path\": \"/v1/\" + x[\"model\"]}\n\ndef response(x):\n    return {\"text\": x[\"body\"][\"t\"]}\n";
        assert_eq!(
            run_starlark_fn(script, "request", &json!({ "model": "m" })).unwrap(),
            json!({ "path": "/v1/m" })
        );
        assert_eq!(
            run_starlark_fn(script, "response", &json!({ "body": { "t": "hi" } })).unwrap(),
            json!({ "text": "hi" })
        );
        assert_eq!(
            check_starlark_defines("p.star", script, &["request", "response"]),
            Ok(())
        );
        let err = check_starlark_defines("p.star", script, &["request", "decide"]).unwrap_err();
        assert!(err.contains("`decide`"), "{err}");
    }

    /// tsk161: the deadlock. A streaming filter that echoes its input can't be
    /// fed a payload larger than the pipe buffer unless stdout is drained
    /// concurrently — the child blocks writing stdout, so it stops reading
    /// stdin, so the parent blocks writing, forever. 4 MB through `cat` used to
    /// hang indefinitely with no budget to stop it.
    ///
    /// The generous budget here is deliberate: this test must fail by HANGING
    /// (and being killed by the harness) if the drain regresses, not by
    /// tripping a short timeout, which would pass for the wrong reason.
    #[test]
    fn exec_streams_a_payload_larger_than_the_pipe_buffer() {
        let budget = Budget::with_timeout(std::time::Duration::from_secs(60));
        // Multiple MB of JSON — ~50x the ~64 KB pipe buffer, in both directions.
        let big: Vec<i64> = (0..500_000).collect();
        let payload = serde_json::to_string(&json!({ "files": {}, "pad": big })).unwrap();
        assert!(payload.len() > 1_000_000, "payload {} bytes", payload.len());

        let argv = vec!["cat".to_string()];
        let out = run_exec_authorized(&budget, &argv, &payload, &|| Ok(()))
            .expect("cat round-trips the payload");
        assert_eq!(
            out.get("pad").and_then(|p| p.as_array()).map(Vec::len),
            Some(500_000),
            "the whole payload made it through both pipes"
        );
    }

    #[test]
    fn exec_kills_a_child_that_exceeds_its_budget() {
        // Unlike the in-process tiers, which can only abandon an overrunning
        // script, a child process can be killed — so exec gets a real ceiling.
        let budget = Budget::with_timeout(std::time::Duration::from_millis(200));
        let argv = vec!["sleep".to_string(), "30".to_string()];
        let started = std::time::Instant::now();
        let err = run_exec_authorized(&budget, &argv, "{}", &|| Ok(()))
            .expect_err("must not wait for the sleep");
        assert!(
            matches!(&err, CollectError::Exec(m) if m.contains("budget")),
            "got {err:?}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "returned in {:?} — the child was not actually killed",
            started.elapsed()
        );
    }

    #[test]
    fn exec_surfaces_a_failing_child_with_its_stderr() {
        let budget = Budget::with_timeout(std::time::Duration::from_secs(10));
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            "echo boom >&2; exit 3".to_string(),
        ];
        let err = run_exec_authorized(&budget, &argv, "{}", &|| Ok(()))
            .expect_err("non-zero exit is an error");
        assert!(
            matches!(&err, CollectError::Exec(m) if m.contains("boom")),
            "stderr must reach the caller: {err:?}"
        );
    }

    #[test]
    fn jaq_maps_input_to_coverage_output() {
        // Reshape a list of {path,total,hit} into the coverage output shape.
        let input = json!([
            { "path": "src/a.rs", "lines": [1, 2], "covered": [1] }
        ]);
        let program = r#"{ files: ( reduce .[] as $f ({}; .[$f.path] = { instrumented: $f.lines, covered: $f.covered }) ) }"#;
        let out = run_jaq(program, &input).expect("jaq runs");
        let typed = value_to_output(CollectorKind::Coverage, out).expect("typed");
        let cov = typed.as_coverage().expect("coverage");
        let f = cov.files.get("src/a.rs").expect("file");
        assert!(f.instrumented.contains(&1) && f.instrumented.contains(&2));
        assert!(f.covered.contains(&1) && !f.covered.contains(&2));
    }

    /// This repo's type-coverage collector reads `target/type-coverage.json`,
    /// which only exists after `type-coverage` has run. Missing, empty or
    /// garbled, the script must still emit one report with no facts.
    #[test]
    fn jaq_type_coverage_script_tolerates_a_missing_report() {
        let program =
            include_str!("../../../.oxplow/extensions/project/collectors/type_coverage.jq");
        for input in [
            json!({}),
            json!({ "report": null }),
            json!({ "report": "" }),
            json!({ "report": "not json" }),
        ] {
            assert_eq!(
                run_jaq(program, &input).unwrap_or_else(|e| panic!("{input}: {e:?}")),
                json!({ "facts": [] }),
                "{input}"
            );
        }
        let report = json!({ "correctCount": 9, "totalCount": 10, "percent": 90.0 });
        assert_eq!(
            run_jaq(program, &json!({ "report": report.to_string() })).unwrap(),
            json!({ "facts": [{
                "measure": "repo.type_coverage", "value": 90.0, "num": 9, "den": 10
            }] })
        );
    }

    #[test]
    fn jaq_errors_on_malformed_program() {
        assert!(matches!(
            run_jaq(".[", &json!({})),
            Err(CollectError::Runtime(_))
        ));
    }

    #[test]
    fn jaq_impure_builtins_are_unavailable() {
        // Determinism/trust guarantee: env/now/input must not resolve, so a
        // program can't read the host environment or clock (which would make
        // `observed` output non-deterministic/leaky).
        for prog in ["env", "now", "input"] {
            assert!(
                matches!(run_jaq(prog, &json!({})), Err(CollectError::Runtime(_))),
                "expected `{prog}` to be unavailable"
            );
        }
        // A pure builtin still works.
        assert_eq!(run_jaq("length", &json!([1, 2, 3])).unwrap(), json!(3));
    }

    #[test]
    fn starlark_maps_input_to_test_output() {
        let input = json!({ "names": ["t1", "t2"] });
        let script = r#"
def transform(input):
    cases = [{"classname": "m", "name": n, "status": "passed"} for n in input["names"]]
    return {"suites": [{"name": "s", "cases": cases}]}
"#;
        let out = run_starlark(script, &input).expect("starlark runs");
        let typed = value_to_output(CollectorKind::Test, out).expect("typed");
        let report = typed.as_test().expect("test");
        assert_eq!(report.suites.len(), 1);
        assert_eq!(report.suites[0].cases.len(), 2);
        assert_eq!(report.suites[0].cases[1].name, "t2");
        assert_eq!(report.suites[0].cases[0].status, TestStatus::Passed);
    }

    #[test]
    fn starlark_can_self_parse_via_host_helpers() {
        // input is raw text; the script calls the parse_xml host builtin.
        let input = json!("<cov file=\"src/a.rs\"/>");
        let script = r#"
def transform(input):
    doc = parse_xml(input)
    path = doc["attrs"]["file"]
    return {"files": {path: {"instrumented": [1, 2], "covered": [1]}}}
"#;
        let out = run_starlark(script, &input).expect("starlark runs");
        let typed = value_to_output(CollectorKind::Coverage, out).expect("typed");
        let cov = typed.as_coverage().expect("coverage");
        let f = cov.files.get("src/a.rs").expect("file");
        assert_eq!(f.instrumented.len(), 2);
        assert!(f.covered.contains(&1));
    }

    #[test]
    fn starlark_helpers_cover_lines_and_regex() {
        let input = json!("1,3\n2,0\n");
        // lines + regex_find host builtins, mapped into coverage.
        let script = r#"
def transform(input):
    instrumented = []
    covered = []
    for row in regex_find(r"(\d+),(\d+)", input):
        n = int(row[1])
        instrumented.append(n)
        if int(row[2]) > 0:
            covered.append(n)
    return {"files": {"f": {"instrumented": instrumented, "covered": covered}}}
"#;
        let out = run_starlark(script, &input).expect("starlark runs");
        let cov = value_to_output(CollectorKind::Coverage, out)
            .expect("typed")
            .as_coverage()
            .expect("coverage")
            .clone();
        let f = cov.files.get("f").expect("file");
        assert_eq!(
            f.instrumented.iter().copied().collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(f.covered.iter().copied().collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn starlark_errors_on_malformed_script() {
        assert!(matches!(
            run_starlark("def transform(", &json!({})),
            Err(CollectError::Runtime(_))
        ));
    }

    #[test]
    fn jaq_maps_input_to_analysis_output() {
        // Reshape a list of raw diagnostics into the analysis output shape.
        let input = json!([
            { "file": "src/a.rs", "ln": 12, "col": 5, "lvl": "warning", "lint": "needless_return", "msg": "x" }
        ]);
        let program = r#"{ findings: [ .[] | { path: .file, line: .ln, column: .col, severity: .lvl, rule: .lint, message: .msg } ] }"#;
        let out = run_jaq(program, &input).expect("jaq runs");
        let typed = value_to_output(CollectorKind::Analysis, out).expect("typed");
        let report = typed.as_analysis().expect("analysis");
        assert_eq!(report.findings.len(), 1);
        let f = &report.findings[0];
        assert_eq!(f.path, "src/a.rs");
        assert_eq!(f.line, Some(12));
        assert_eq!(f.column, Some(5));
        assert_eq!(f.severity, Severity::Warning);
        assert_eq!(f.rule.as_deref(), Some("needless_return"));
    }

    #[test]
    fn analysis_json_deserializes_with_defaults() {
        // Optional line/column/rule absent → None; missing findings → empty.
        let v = json!({ "findings": [ { "path": "x", "severity": "error", "message": "boom" } ] });
        let out = value_to_output(CollectorKind::Analysis, v).expect("typed");
        let report = out.as_analysis().unwrap();
        assert_eq!(report.findings.len(), 1);
        let f = &report.findings[0];
        assert_eq!(f.severity, Severity::Error);
        assert!(f.line.is_none() && f.column.is_none() && f.rule.is_none());

        let empty = value_to_output(CollectorKind::Analysis, json!({})).expect("typed");
        assert!(empty.as_analysis().unwrap().findings.is_empty());
    }

    #[test]
    fn coverage_json_deserializes_with_defaults() {
        let v = json!({ "files": { "x": { "instrumented": [1, 2, 3] } } });
        let out = value_to_output(CollectorKind::Coverage, v).expect("typed");
        let cov = out.as_coverage().unwrap();
        let f = cov.files.get("x").unwrap();
        assert_eq!(f.instrumented.len(), 3);
        assert!(f.covered.is_empty());
        // Branch/function counts default to 0 when the report omits them (tsk123).
        assert_eq!(f.branches_found, 0);
        assert_eq!(f.functions_found, 0);
    }

    #[test]
    fn coverage_json_carries_branch_and_function_counts() {
        // tsk123: the parsers emit camelCase branch/function counts; they
        // deserialize onto the typed FileCoverage.
        let v = json!({ "files": { "x": {
            "instrumented": [1, 2], "covered": [1],
            "branchesFound": 4, "branchesHit": 3,
            "functionsFound": 2, "functionsHit": 2,
        } } });
        let out = value_to_output(CollectorKind::Coverage, v).expect("typed");
        let f = out.as_coverage().unwrap().files.get("x").unwrap().clone();
        assert_eq!((f.branches_found, f.branches_hit), (4, 3));
        assert_eq!((f.functions_found, f.functions_hit), (2, 2));
    }
}

#[cfg(test)]
mod check_starlark_tests {
    use super::check_starlark;

    /// `transform` is found in the parsed module, not by text: one inside
    /// a string or a block doesn't count; a parse error names the file.
    #[test]
    fn transform_is_a_top_level_def_in_the_ast() {
        assert!(check_starlark("handlers/a.star", "def transform(x):\n    return {}\n").is_ok());
        for script in [
            "X = \"\"\"\ndef transform(x):\n\"\"\"\n",
            "def other(x):\n    def transform(y):\n        return y\n    return transform\n",
            "transform = 1\n",
        ] {
            let err = check_starlark("handlers/a.star", script).unwrap_err();
            assert!(err.contains("must define `transform`"), "{script}: {err}");
        }
        let err = check_starlark("handlers/a.star", "def transform(x:\n").unwrap_err();
        assert!(err.contains("handlers/a.star"), "{err}");
    }
}
