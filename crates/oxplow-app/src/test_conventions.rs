//! The test conventions the enabled extensions declare (`test_conventions:`,
//! `.context/language-support.md` "Test conventions"), compiled once and
//! applied by change analysis: whether a path, a container or a function
//! is a test, and how many assertions and skips a file's text holds. Core
//! keeps the engine — the matching, comment stripping and counting — and
//! names no framework or language's rules.
//!
//! Built from the published extensions at each publication and swapped
//! whole (`extension_activation`, beside the language set), so a change
//! analysis reads one consistent set and never a manifest.

use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use oxplow_code_metrics::Languages;

use crate::extension::extensions::test_conventions::{name_glob, path_glob, TestConventionDecl};
use crate::extension::extensions::Extension;

/// Assertion and skip counts for one version of a file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TestSignals {
    pub assertions: i64,
    pub skips: i64,
}

/// One declared convention, compiled.
struct Rule {
    /// Language ids it applies to; empty: any file.
    languages: Vec<String>,
    paths: Option<globset::GlobSet>,
    containers: Vec<globset::GlobMatcher>,
    functions: Vec<globset::GlobMatcher>,
    assertions: Vec<regex::Regex>,
    skips: Vec<regex::Regex>,
}

/// The conventions of every enabled extension, and the language set that
/// says which language a path is.
pub struct TestConventions {
    rules: Vec<Rule>,
    languages: Arc<Languages>,
    /// A hash of the declarations it was built from: an analysis keyed by it
    /// is stale when the conventions change.
    fingerprint: u64,
}

impl Default for TestConventions {
    /// No conventions: nothing is a test until an extension says so.
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            languages: Arc::new(Languages::build(&[]).0),
            fingerprint: 0,
        }
    }
}

impl TestConventions {
    /// The enabled extensions' conventions. A declaration that doesn't
    /// compile was refused at load, so one that gets here is dropped only
    /// if it somehow does not.
    pub fn of(extensions: &[Extension], languages: Arc<Languages>) -> Self {
        let mut enabled: Vec<&Extension> = extensions
            .iter()
            .filter(|e| e.enabled && !e.test_conventions.is_empty())
            .collect();
        enabled.sort_by(|a, b| a.name.cmp(&b.name));
        Self::build(
            enabled.iter().flat_map(|e| e.test_conventions.iter()),
            languages,
        )
    }

    fn build<'a>(
        decls: impl Iterator<Item = &'a TestConventionDecl>,
        languages: Arc<Languages>,
    ) -> Self {
        let decls: Vec<&TestConventionDecl> = decls.collect();
        // FNV-1a over the declarations as written.
        let fingerprint = decls
            .iter()
            .flat_map(|d| {
                serde_json::to_string(d)
                    .unwrap_or_default()
                    .into_bytes()
                    .into_iter()
                    .chain(std::iter::once(0xff))
            })
            .fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| {
                (acc ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        let rules = decls
            .into_iter()
            .filter_map(|d| {
                let paths = if d.paths.is_empty() {
                    None
                } else {
                    let mut set = globset::GlobSetBuilder::new();
                    for g in &d.paths {
                        set.add(path_glob(g).ok()?);
                    }
                    Some(set.build().ok()?)
                };
                let names = |list: &[String]| -> Option<Vec<globset::GlobMatcher>> {
                    list.iter().map(|g| name_glob(g).ok()).collect()
                };
                let regexes = |list: &[String]| -> Option<Vec<regex::Regex>> {
                    list.iter().map(|r| regex::Regex::new(r).ok()).collect()
                };
                Some(Rule {
                    languages: d.languages.clone(),
                    paths,
                    containers: names(&d.containers)?,
                    functions: names(&d.functions)?,
                    assertions: regexes(&d.assertions)?,
                    skips: regexes(&d.skips)?,
                })
            })
            .collect();
        Self {
            rules,
            languages,
            fingerprint,
        }
    }

    /// A hash of the declarations it holds.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Every convention oxplow ships (foundation's and each language pack's,
    /// enabled or not) over its shipped languages: what a unit test of the
    /// analysis reads.
    #[cfg(test)]
    pub fn shipped() -> Self {
        let decls: Vec<TestConventionDecl> = crate::bundled_extensions::BUNDLED
            .iter()
            .filter_map(|b| {
                let text = b.file("extension.yaml")?;
                let v: serde_yaml::Value = serde_yaml::from_str(text).ok()?;
                let list = v.get("test_conventions")?;
                let (declared, errors) =
                    crate::extension::extensions::test_conventions::parse_test_conventions(
                        list,
                        "extension.yaml",
                        text,
                    );
                assert!(errors.is_empty(), "{}: {errors:?}", b.name);
                Some(declared)
            })
            .flatten()
            .collect();
        Self::build(
            decls.iter(),
            Arc::new(crate::extension::language_set::shipped()),
        )
    }

    /// The rules that apply to `path`: the ones for its language, and the
    /// ones for any.
    fn applicable<'a>(&'a self, path: &str) -> impl Iterator<Item = &'a Rule> + 'a {
        let language = self
            .languages
            .declared_for_path(path)
            .map(|l| l.name().to_string());
        self.rules.iter().filter(move |r| {
            r.languages.is_empty()
                || language
                    .as_deref()
                    .is_some_and(|l| r.languages.iter().any(|x| x == l))
        })
    }

    /// A test file by the conventions' `paths`.
    pub fn is_test_path(&self, path: &str) -> bool {
        let path = path.replace('\\', "/");
        self.applicable(&path)
            .any(|r| r.paths.as_ref().is_some_and(|set| set.is_match(&path)))
    }

    /// A test container (class, module) by the conventions' `containers`.
    pub fn is_test_container(&self, path: &str, name: &str) -> bool {
        self.applicable(path)
            .any(|r| r.containers.iter().any(|m| m.is_match(name)))
    }

    /// A test function by the conventions' `functions` (its name alone).
    pub fn is_test_function_name(&self, path: &str, name: &str) -> bool {
        self.applicable(path)
            .any(|r| r.functions.iter().any(|m| m.is_match(name)))
    }

    /// Count assertions and skip markers in `content`, a file at `path`.
    /// Line comments (`//`, and `#` lines that aren't `#[attribute]`s) are
    /// ignored, so commenting an assertion out lowers the count. A span two
    /// conventions both match counts once.
    pub fn count(&self, path: &str, content: &str) -> TestSignals {
        let rules: Vec<&Rule> = self.applicable(path).collect();
        let mut out = TestSignals::default();
        if rules
            .iter()
            .all(|r| r.assertions.is_empty() && r.skips.is_empty())
        {
            return out;
        }
        for raw in content.lines() {
            let line = code_part(raw);
            if line.is_empty() {
                continue;
            }
            let spans = |pick: fn(&Rule) -> &Vec<regex::Regex>| -> i64 {
                let mut starts = HashSet::new();
                for re in rules.iter().flat_map(|r| pick(r).iter()) {
                    starts.extend(re.find_iter(line).map(|m| m.start()));
                }
                starts.len() as i64
            };
            out.assertions += spans(|r| &r.assertions);
            out.skips += spans(|r| &r.skips);
        }
        out
    }
}

/// The line without a trailing `//` comment; empty for a `#` comment line
/// (Python, shell) — but not for a `#[attribute]` or `#!`.
fn code_part(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') && !trimmed.starts_with("#[") && !trimmed.starts_with("#!") {
        return "";
    }
    match line.find("//") {
        Some(i) => &line[..i],
        None => line,
    }
}

/// The set the app reads, swapped whole when what runs changes.
#[derive(Clone, Default)]
pub struct TestConventionsHandle(Arc<RwLock<Arc<TestConventions>>>);

impl TestConventionsHandle {
    pub fn current(&self) -> Arc<TestConventions> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn swap(&self, conventions: TestConventions) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(conventions);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assertions and skips of `src` as a file at `path`, by every
    /// convention oxplow ships.
    fn c(path: &str, src: &str) -> (i64, i64) {
        let s = TestConventions::shipped().count(path, src);
        (s.assertions, s.skips)
    }

    #[test]
    fn rust_assertions_and_ignored_tests() {
        let src = "#[test]\nfn a() {\n    assert_eq!(1, 1);\n    assert!(true);\n    debug_assert!(x);\n}\n#[test]\n#[ignore]\nfn b() { assert_ne!(1, 2) }\n";
        assert_eq!(c("a.rs", src), (4, 1));
    }

    #[test]
    fn js_expectations_and_skips() {
        let src = "it('a', () => { expect(x).toBe(1); expect(y).toEqual(2); });\nit.skip('b', () => {});\nxit('c', () => {});\ndescribe.skip('d', () => {});\ntest.todo('e');\n";
        assert_eq!(c("a.test.ts", src), (2, 4));
    }

    #[test]
    fn python_go_and_java() {
        let py = "def test_a():\n    assert x == 1\n    self.assertEqual(a, b)\n\n@pytest.mark.skip(reason='later')\ndef test_b():\n    pytest.skip('no')\n";
        assert_eq!(c("test_a.py", py), (2, 2));
        let go = "func TestA(t *testing.T) {\n\tif x != 1 { t.Errorf(\"bad\") }\n\tt.Fatal(\"x\")\n\tt.Skip(\"later\")\n\trequire.Equal(t, 1, x)\n}\n";
        assert_eq!(c("a_test.go", go), (3, 1));
        let java = "@Test void a() { assertEquals(1, x); assertThat(y).isTrue(); }\n@Disabled @Test void b() {}\n";
        assert_eq!(c("A.java", java), (2, 1));
    }

    #[test]
    fn words_that_merely_contain_assert_or_skip_do_not_count() {
        assert_eq!(
            c("a.rs", "let reassertion = skipper(); // not an assertion\n"),
            (0, 0)
        );
    }

    #[test]
    fn commented_out_assertions_do_not_count() {
        // Commenting an assertion out is a weakening, so it must drop the count.
        assert_eq!(c("a.py", "// assert!(x);\n    # assert x == 1\n"), (0, 0));
        // `#[ignore]` is an attribute, not a comment.
        assert_eq!(c("a.rs", "#[ignore]\n"), (0, 1));
    }

    /// A convention for one language doesn't apply to another's files, and
    /// two conventions on one match count it once.
    #[test]
    fn a_language_convention_applies_to_its_files_only() {
        // `#[ignore` is Rust's; `t.Skip` is Go's.
        assert_eq!(c("a.py", "#[ignore]\n"), (0, 0));
        assert_eq!(c("a.rs", "t.Skip(\"x\")\n"), (0, 0));
        assert_eq!(c("a_test.go", "t.Skip(\"x\")\n"), (0, 1));
        let both = |decls: &[(&str, &str)]| {
            let decls: Vec<TestConventionDecl> = decls
                .iter()
                .map(|(id, re)| TestConventionDecl {
                    id: id.to_string(),
                    languages: vec![],
                    paths: vec![],
                    containers: vec![],
                    functions: vec![],
                    assertions: vec![re.to_string()],
                    skips: vec![],
                })
                .collect();
            TestConventions::build(decls.iter(), Arc::new(Languages::default()))
                .count("a.rs", "check(x)\n")
                .assertions
        };
        assert_eq!(both(&[("a", r"check\("), ("b", r"check")]), 1);
    }

    /// The conventions follow what is enabled: a pack's apply while it is on
    /// and stop when it is turned off; foundation's stay.
    #[tokio::test]
    async fn a_disabled_packs_conventions_do_not_apply() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let go = "x/a_test.go";
        let now = svc.test_conventions.current();
        assert!(now.is_test_path(go) && now.is_test_path("x/a.test.ts"));
        assert_eq!(now.count(go, "t.Skip(\"x\")\n").skips, 1);
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.extension.disable",
                serde_json::json!({ "name": "oxplow-go" }),
                true,
            )
            .await
            .unwrap();
        let now = svc.test_conventions.current();
        assert!(!now.is_test_path(go), "Go's rule is off with its pack");
        assert_eq!(now.count(go, "t.Skip(\"x\")\n").skips, 0);
        assert!(now.is_test_path("x/a.test.ts"), "foundation's stay");
        assert!(now.is_test_function_name("a.go", "TestThing"));
    }
}
