//! The metric extension template (`.context/metrics.md` "Authoring
//! surface"): what a new code metric starts as. One function builds the
//! extension, so `oxplow extension new metric` (the SDK writes the files)
//! and the `oxplow.metric.scaffold` command (it hands them to the agent to
//! write) produce the same thing.
//!
//! It is a private extension: a per-path `measures:` entry on files, a
//! Starlark fact collector on `snapshot.taken` that counts each file's
//! functions over a complexity limit (`source_files()` + `code_metrics()`),
//! the `<namespace>.area` dimension its facts carry (the top folder of the
//! path), the headline `metrics:` spec, a launcher page whose rows link to
//! the file and line, and an intent example with the fixture
//! `oxplow extension test` runs the collector over.

use oxplow_code_metrics::{language_for_path, language_from_name, language_support, Language};
use oxplow_domain::contribution::extension_namespace;
use serde_json::json;

use crate::extensions::{self, EXTENSIONS_DIR};

/// Cyclomatic complexity above which a function counts, in the starter
/// collector's `LIMIT`.
pub const COMPLEXITY_LIMIT: u32 = 10;

/// The language and files a new metric scans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricStarter {
    /// The canonical language name (`rust`, `typescript`, …).
    pub language: Language,
    /// The glob of files the collector reads, `*` and `**` only.
    pub glob: String,
}

impl Default for MetricStarter {
    fn default() -> Self {
        Self::new(None, None).expect("the default starter is valid")
    }
}

/// Languages with a starter sample (the fixture's source), in the order
/// the error lists them.
const SAMPLED: &[Language] = &[
    Language::Rust,
    Language::TypeScript,
    Language::JavaScript,
    Language::Python,
    Language::Go,
    Language::C,
    Language::Cpp,
];

impl MetricStarter {
    /// The starter for `language` (default `rust`) over `glob` (default:
    /// every file with one of the language's extensions).
    pub fn new(language: Option<&str>, glob: Option<&str>) -> Result<Self, String> {
        let language = match language.filter(|l| !l.is_empty()) {
            None => Language::Rust,
            Some(name) => language_from_name(name).ok_or_else(|| {
                format!(
                    "`{name}` isn't a language oxplow analyses; one of {}",
                    names(SAMPLED)
                )
            })?,
        };
        if !SAMPLED.contains(&language) {
            return Err(format!(
                "no starter sample for `{}` yet; scaffold one of {} and change LANGUAGE in the script",
                language.name(),
                names(SAMPLED)
            ));
        }
        let glob = match glob.filter(|g| !g.is_empty()) {
            Some(g) => {
                if g.contains(['{', '[', '?']) {
                    return Err(format!(
                        "--glob `{g}`: use `*` and `**` only (e.g. `src/**/*.{}`)",
                        extension_of(language)
                    ));
                }
                g.to_string()
            }
            None => format!("**/*.{}", extension_of(language)),
        };
        let starter = Self { language, glob };
        // The sample's paths come from the glob: they must still be files
        // of the language, or the fixture could never match.
        for path in starter.sample_paths() {
            if language_for_path(&path) != Some(language) {
                return Err(format!(
                    "--glob `{}` doesn't name {} files (its sample path `{path}` isn't one); \
                     end it in `*.{}`",
                    starter.glob,
                    language.name(),
                    extension_of(language)
                ));
            }
        }
        Ok(starter)
    }

    /// The two sample files' paths, in two folders: the glob with its
    /// first `**` as `core` / `web` and each other `*` as `engine` / `view`.
    fn sample_paths(&self) -> [String; 2] {
        let one = |folder: &str, file: &str| {
            let mut out = String::new();
            let mut rest = self.glob.as_str();
            let mut first = true;
            while let Some(i) = rest.find('*') {
                out.push_str(&rest[..i]);
                if rest[i..].starts_with("**") {
                    out.push_str(if first { folder } else { file });
                    rest = &rest[i + 2..];
                    first = false;
                } else {
                    out.push_str(file);
                    rest = &rest[i + 1..];
                }
            }
            out.push_str(rest);
            out
        };
        [one("core", "engine"), one("web", "view")]
    }
}

fn names(languages: &[Language]) -> String {
    languages
        .iter()
        .map(|l| format!("`{}`", l.name()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn extension_of(language: Language) -> &'static str {
    language_support(language).extensions[0]
}

/// One file of the extension: its path relative to the project root, and
/// its content.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ScaffoldFile {
    pub path: String,
    pub content: String,
}

/// Every file of the extension `name`, relative to the project root:
/// `extension.yaml`, the collector script, the page and the fixture.
/// `origin` is the thread or effort ref that asked for it.
pub fn extension_files(
    name: &str,
    origin: Option<&str>,
    starter: &MetricStarter,
) -> Vec<ScaffoldFile> {
    let ns = extension_namespace(name);
    let dir = format!("{EXTENSIONS_DIR}/{name}");
    let measure = format!("{ns}.complex_functions");
    let area = format!("{ns}.area");
    let collector = format!("{ns}.scan");
    let title = title_case(name);
    let sample = Sample::new(starter, &measure, &area);

    let mut manifest = extensions::scaffold_manifest(&extensions::ManifestScaffold {
        name,
        description: "TODO: one line on what this metric tracks",
        purpose: "TODO: the question this metric answers",
        origin,
        example_name: "basic",
        example_input: &sample.input(&collector),
        example_expect: "one fact per file with functions over the limit, in its area",
    });
    manifest.push_str(&format!(
        "# Its collector reads the project's code.\n\
         permissions: [files.read]\n\
         measures:\n\
         \x20 # One fact per file: how many of its functions are over the limit. `per-path`\n\
         \x20 # because a snapshot rescans only the files it changed; `semi-additive`\n\
         \x20 # because the latest count is the answer, not a sum over time.\n\
         \x20 - key: {measure}\n\
         \x20   title: Functions over the complexity limit\n\
         \x20   unit: count\n\
         \x20   subjectKind: file\n\
         \x20   temporalSemantics: semi-additive\n\
         \x20   fold: last\n\
         \x20   captureScope: per-path\n\
         dimensions:\n\
         \x20 # Facts carry it in `dims`; metrics slice by it (`metric_grid('day', '{area}')`).\n\
         \x20 - key: {area}\n\
         \x20   label: Area\n\
         \x20   valueType: categorical\n\
         collectors:\n\
         \x20 # Runs after every snapshot that recorded files: all of the changed ones in a\n\
         \x20 # delta, the whole tree for the first baseline. Starlark, so no I/O.\n\
         \x20 - id: {collector}\n\
         \x20   doc: \"Functions over the complexity limit, per file\"\n\
         \x20   runtime: starlark\n\
         \x20   entry: collectors/complex_functions.star\n\
         \x20   on: [snapshot.taken]\n\
         \x20   facts: [{measure}]\n\
         metrics:\n\
         \x20 # The headline: the per-file facts, added up.\n\
         \x20 - key: {measure}\n\
         \x20   title: \"{title}: functions over the limit\"\n\
         \x20   sourceMeasure: {measure}\n\
         \x20   aggregation: sum\n\
         \x20   direction: lower-better\n\
         \x20   unit: count\n\
         \x20   displayKind: findings  # its facts are located, so the drill-in lists the files\n\
         \x20   sliceableDims: [{area}]\n"
    ));
    let page = format!(
        "title: {title}\n\
         description: \"TODO: what this page answers. The files with functions over the limit, worst first.\"\n\
         category: Code\n\
         data:\n\
         \x20 # The current facts behind the metric: one row per file, its worst function's line.\n\
         \x20 findings:\n\
         \x20   sql: |\n\
         \x20     SELECT path, line, value AS functions\n\
         \x20     FROM metric_findings('{measure}')\n\
         \x20     ORDER BY value DESC, path\n\
         view:\n\
         \x20 viz: table\n\
         \x20 from: findings\n\
         \x20 columns:\n\
         \x20   - {{ key: path, label: File, link: {{ kind: file, line: line }} }}\n\
         \x20   - {{ key: line, label: Worst function at line }}\n\
         \x20   - {{ key: functions, label: Functions over the limit }}\n\
         \x20 empty: No functions over the limit, or no snapshot has been taken yet.\n"
    );
    let script = collector_script(starter, &measure, &area);
    let fixture = format!(
        "# The acceptance example from extension.yaml as a fixture for `oxplow extension test`\n\
         # (`when` it does this, `expect` that). Keep the two in step.\n\
         when: {}\nexpect: {}\n",
        sample.input(&collector),
        sample.expect()
    );
    vec![
        ScaffoldFile {
            path: format!("{dir}/extension.yaml"),
            content: manifest,
        },
        ScaffoldFile {
            path: format!("{dir}/collectors/complex_functions.star"),
            content: script,
        },
        ScaffoldFile {
            path: format!("{dir}/pages/{name}.yaml"),
            content: page,
        },
        ScaffoldFile {
            path: format!("{dir}/fixtures/basic.yaml"),
            content: fixture,
        },
    ]
}

/// The starter collector: `LIMIT`, `LANGUAGE` and `GLOB` at the top, one
/// fact per file with functions over the limit.
fn collector_script(starter: &MetricStarter, measure: &str, area: &str) -> String {
    format!(
        "# Functions over the complexity limit, per file. One FACT per file (never a\n\
         # total): the metric adds them up. Reads the snapshot's files, so it is\n\
         # sandboxed and deterministic.\n\
         LIMIT = {COMPLEXITY_LIMIT}  # cyclomatic complexity above this counts\n\
         LANGUAGE = \"{language}\"\n\
         GLOB = \"{glob}\"\n\
         \n\
         def area(path):\n\
         \x20   # The top folder; a file at the root is in \"(root)\".\n\
         \x20   parts = path.split(\"/\")\n\
         \x20   return parts[0] if len(parts) > 1 else \"(root)\"\n\
         \n\
         def transform(input):\n\
         \x20   # source_files() leaves out generated code and unsupported languages.\n\
         \x20   languages = {{f[\"path\"]: f[\"language\"] for f in source_files()}}\n\
         \x20   facts = []\n\
         \x20   for f in files(GLOB):\n\
         \x20       if languages.get(f[\"path\"]) != LANGUAGE:\n\
         \x20           continue\n\
         \x20       over = [m for m in code_metrics(f[\"text\"], LANGUAGE, f[\"path\"]) if m[\"complexity\"] > LIMIT]\n\
         \x20       if len(over) == 0:\n\
         \x20           continue  # a clean file records nothing; its old fact is replaced\n\
         \x20       worst = over[0]\n\
         \x20       for m in over:\n\
         \x20           if m[\"complexity\"] > worst[\"complexity\"]:\n\
         \x20               worst = m\n\
         \x20       facts.append({{\n\
         \x20           \"measure\": \"{measure}\",\n\
         \x20           \"value\": len(over),\n\
         \x20           \"subject\": uri.build(\"oxplow/file\", path = f[\"path\"]),\n\
         \x20           \"path\": f[\"path\"],\n\
         \x20           \"line\": worst[\"start_line\"],  # where the page links to\n\
         \x20           \"dims\": {{\"{area}\": area(f[\"path\"])}},\n\
         \x20       }})\n\
         \x20   return {{\"facts\": facts}}\n",
        language = starter.language.name(),
        glob = starter.glob,
    )
}

/// The fixture's corpus and the facts it must produce: two files in two
/// folders, the first with one function over the limit, the second with two.
struct Sample {
    paths: [String; 2],
    texts: [String; 2],
    /// (value, line of the worst function) per file.
    facts: [(usize, usize); 2],
    measure: String,
    area: String,
}

impl Sample {
    fn new(starter: &MetricStarter, measure: &str, area: &str) -> Self {
        let over = COMPLEXITY_LIMIT as usize + 1;
        // Complexity is 1 + the decisions: `over` branches is over the limit.
        let (engine, engine_starts) = source(starter.language, &[("calm", 0), ("busy", over)]);
        let (view, view_starts) = source(
            starter.language,
            &[("busy_a", over), ("busy_b", over + 1), ("calm", 0)],
        );
        Self {
            paths: starter.sample_paths(),
            texts: [engine, view],
            facts: [(1, engine_starts[1]), (2, view_starts[1])],
            measure: measure.to_string(),
            area: area.to_string(),
        }
    }

    /// The example's input, as YAML flow text.
    fn input(&self, collector: &str) -> String {
        let files: Vec<String> = self
            .paths
            .iter()
            .zip(&self.texts)
            .map(|(path, text)| format!("{}: {}", json!(path), json!(text)))
            .collect();
        format!(
            "{{ collector: {collector}, files: {{ {} }} }}",
            files.join(", ")
        )
    }

    /// The facts the collector must record, as YAML flow text.
    fn expect(&self) -> String {
        let facts: Vec<String> = self
            .paths
            .iter()
            .zip(&self.facts)
            .map(|(path, (value, line))| {
                format!(
                    "{{ measure: {}, value: {value}, subject: {}, path: {}, line: {line}, dims: {{ {}: {} }} }}",
                    self.measure,
                    json!(oxplow_domain::uri::build::file_uri(path)),
                    json!(path),
                    json!(self.area),
                    json!(path.split('/').next().unwrap_or_default()),
                )
            })
            .collect();
        format!("{{ facts: [{}] }}", facts.join(", "))
    }
}

/// A source file in `language` defining each `(name, branches)` function
/// with that many `if` branches; the line each function starts on.
pub(crate) fn source(language: Language, functions: &[(&str, usize)]) -> (String, Vec<usize>) {
    let mut lines: Vec<String> = Vec::new();
    let mut starts = Vec::new();
    for (name, branches) in functions {
        starts.push(lines.len() + 1);
        lines.extend(function(language, name, *branches));
    }
    (lines.join("\n") + "\n", starts)
}

fn function(language: Language, name: &str, branches: usize) -> Vec<String> {
    let mut out = Vec::new();
    let i = 1..=branches;
    match language {
        Language::Rust => {
            out.push(format!("fn {name}(x: i32) -> i32 {{"));
            out.extend(i.map(|n| format!("    if x == {n} {{ return {n}; }}")));
            out.push("    0".into());
            out.push("}".into());
        }
        Language::TypeScript | Language::JavaScript => {
            let params = if language == Language::TypeScript {
                "x: number): number"
            } else {
                "x)"
            };
            out.push(format!("function {name}({params} {{"));
            out.extend(i.map(|n| format!("    if (x === {n}) {{ return {n}; }}")));
            out.push("    return 0;".into());
            out.push("}".into());
        }
        Language::Python => {
            out.push(format!("def {name}(x):"));
            for n in i {
                out.push(format!("    if x == {n}:"));
                out.push(format!("        return {n}"));
            }
            out.push("    return 0".into());
        }
        Language::Go => {
            out.push(format!("func {name}(x int) int {{"));
            out.extend(i.map(|n| format!("    if x == {n} {{ return {n} }}")));
            out.push("    return 0".into());
            out.push("}".into());
        }
        _ => {
            out.push(format!("int {name}(int x) {{"));
            out.extend(i.map(|n| format!("    if (x == {n}) {{ return {n}; }}")));
            out.push("    return 0;".into());
            out.push("}".into());
        }
    }
    out
}

fn title_case(name: &str) -> String {
    name.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every sampled language's "busy" function really is over the limit
    /// and its "calm" one isn't, at the line the fixture expects.
    #[test]
    fn every_starter_sample_has_one_function_over_the_limit_at_the_expected_line() {
        for language in SAMPLED {
            let (text, starts) = source(*language, &[("calm", 0), ("busy", 11)]);
            let fns = oxplow_code_metrics::analyze_with_language("", &text, *language);
            let busy = fns
                .iter()
                .find(|f| f.name == "busy")
                .unwrap_or_else(|| panic!("{}: busy found in {fns:?}", language.name()));
            assert!(busy.complexity > COMPLEXITY_LIMIT, "{}", language.name());
            assert_eq!(busy.start_line as usize, starts[1], "{}", language.name());
            let calm = fns.iter().find(|f| f.name == "calm").expect("calm");
            assert!(calm.complexity <= COMPLEXITY_LIMIT, "{}", language.name());
        }
    }

    #[test]
    fn the_starter_defaults_to_rust_over_its_files_and_refuses_what_cannot_be_sampled() {
        let s = MetricStarter::default();
        assert_eq!((s.language, s.glob.as_str()), (Language::Rust, "**/*.rs"));
        let ts = MetricStarter::new(Some("ts"), None).unwrap();
        assert_eq!(ts.glob, "**/*.ts");
        let src = MetricStarter::new(Some("rust"), Some("src/**/*.rs")).unwrap();
        assert_eq!(
            src.sample_paths(),
            [
                "src/core/engine.rs".to_string(),
                "src/web/view.rs".to_string()
            ]
        );
        assert!(MetricStarter::new(Some("klingon"), None)
            .unwrap_err()
            .contains("isn't a language"));
        assert!(MetricStarter::new(Some("java"), None)
            .unwrap_err()
            .contains("no starter sample"));
        assert!(MetricStarter::new(None, Some("**/*.{rs,toml}"))
            .unwrap_err()
            .contains("`*` and `**` only"));
        assert!(MetricStarter::new(None, Some("**/*.md"))
            .unwrap_err()
            .contains("doesn't name rust files"));
    }
}
