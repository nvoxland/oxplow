//! Architectural zone classification for repo files.
//!
//! A zone is a coarse architectural bucket — `ui`, `store`, `docs` —
//! that every repo file belongs to, and the unit the Change-analysis
//! surfaces summarize churn and cross-boundary imports by.
//!
//! **The rules are the project's, not oxplow's** (tsk251). Oxplow ships
//! no built-in table: what makes a file "the store layer" follows from
//! how a particular repo is laid out, and a general-purpose tool can't
//! assume that. Rules come from the `zones:` block in
//! `.oxplow/project.yaml` (see [`oxplow_config::ZoneRuleConfig`]) — an
//! ordered list where the FIRST matching rule wins, so specific
//! patterns precede catch-alls. A project with no `zones:` block
//! classifies every file as [`ZONE_OTHER`]; the zone surfaces stay
//! empty rather than showing a guess.

use globset::GlobBuilder;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ImportEdge;

pub use oxplow_config::{ZONE_EXTERNAL, ZONE_OTHER};

/// One compiled rule: the globs, kept alongside their source patterns
/// (module resolution reads the patterns as text).
struct CompiledRule {
    matchers: Vec<globset::GlobMatcher>,
    patterns: Vec<String>,
    zone: String,
}

/// The project's compiled zone table.
///
/// Cheap to build (a few dozen globs) and cheap to query, so callers
/// construct one per request from the live config rather than caching a
/// copy that could go stale against a `set_zones` edit.
pub struct ZoneRules {
    rules: Vec<CompiledRule>,
}

impl ZoneRules {
    /// No rules — every path classifies as [`ZONE_OTHER`]. This is what
    /// an unconfigured project gets.
    pub fn empty() -> Self {
        Self { rules: Vec::new() }
    }

    /// Compile the project's `zones:` table, preserving order.
    ///
    /// A pattern that fails to compile is skipped rather than fatal:
    /// config load already rejects bad globs (`validate_zones`), so
    /// reaching one here means the config was built in memory, and
    /// dropping the rule beats panicking inside an analysis request.
    pub fn from_config(rules: &[oxplow_config::ZoneRuleConfig]) -> Self {
        let rules = rules
            .iter()
            .map(|rule| {
                let mut matchers = Vec::with_capacity(rule.patterns.len());
                for pattern in &rule.patterns {
                    match GlobBuilder::new(pattern).literal_separator(true).build() {
                        Ok(glob) => matchers.push(glob.compile_matcher()),
                        Err(e) => tracing_skip(pattern, &e.to_string()),
                    }
                }
                CompiledRule {
                    matchers,
                    patterns: rule.patterns.clone(),
                    zone: rule.zone.clone(),
                }
            })
            .collect();
        Self { rules }
    }

    /// True when the project declared no zones.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Classify a repo-relative path. First matching rule wins;
    /// [`ZONE_OTHER`] when none match.
    pub fn classify(&self, path: &str) -> String {
        let normalized = path.replace('\\', "/");
        for rule in &self.rules {
            if rule.matchers.iter().any(|m| m.is_match(&normalized)) {
                return rule.zone.clone();
            }
        }
        ZONE_OTHER.to_string()
    }

    /// Resolve a MODULE name (a Rust crate in `use foo::bar`, say) to a
    /// zone.
    ///
    /// An import names a module, not a file, so there is no path to
    /// classify. The heuristic: normalize `_` to `-` and look for the
    /// name as a whole path SEGMENT of some rule's pattern — module
    /// `oxplow_db` hits the rule `crates/oxplow-db/**`. That is enough
    /// to resolve first-party imports from the project's own zone table,
    /// without oxplow having to read Cargo/npm manifests to learn which
    /// packages are in-repo. `None` means "not one of ours", which the
    /// caller reports as [`ZONE_EXTERNAL`].
    pub fn zone_for_module(&self, name: &str) -> Option<String> {
        let dashed = name.replace('_', "-");
        for rule in &self.rules {
            for pattern in &rule.patterns {
                if pattern
                    .split('/')
                    .any(|seg| seg == name || seg == dashed.as_str())
                {
                    return Some(rule.zone.clone());
                }
            }
        }
        None
    }

    /// Classify an [`ImportEdge`] whose target resolved to a repo file.
    pub fn zone_for_resolved_edge(
        &self,
        edge: ImportEdge,
        resolved_target: &str,
    ) -> ZonedImportEdge {
        let from_zone = self.classify(&edge.from_path);
        let to_zone = Some(self.classify(resolved_target));
        ZonedImportEdge {
            edge,
            from_zone,
            to_zone,
        }
    }

    /// Classify an [`ImportEdge`] whose target couldn't be resolved to a
    /// repo file. `to_zone` stays `None` — we don't pretend to know.
    pub fn zone_for_unresolved_edge(&self, edge: ImportEdge) -> ZonedImportEdge {
        let from_zone = self.classify(&edge.from_path);
        ZonedImportEdge {
            edge,
            from_zone,
            to_zone: None,
        }
    }
}

/// Log-and-drop for a pattern that survived config validation but won't
/// compile here. Split out so the closure above stays readable.
fn tracing_skip(pattern: &str, error: &str) {
    // No `tracing` dep in this crate (it is a pure analysis library);
    // an eprintln keeps the signal without pulling one in.
    eprintln!("zones: skipping uncompilable glob {pattern:?}: {error}");
}

/// A directed edge between two zones, with the originating
/// [`ImportEdge`] for hover/drill-down.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ZonedImportEdge {
    pub edge: ImportEdge,
    pub from_zone: String,
    /// The target zone if we could classify it, else None. None
    /// indicates a target we couldn't resolve (external package, path
    /// the resolver doesn't know how to walk).
    pub to_zone: Option<String>,
}

impl ZonedImportEdge {
    /// True when this edge crosses an *internal* architectural
    /// boundary worth flagging. Specifically:
    ///
    /// - both zones must be known,
    /// - the target must not be [`ZONE_EXTERNAL`] (reaching into a
    ///   third-party crate / npm package isn't a layer violation),
    /// - the two zones must differ.
    ///
    /// Unknown targets (None) never trip cross-zone — better to
    /// underflag than overflag.
    pub fn is_cross_zone(&self) -> bool {
        match self.to_zone.as_deref() {
            None => false,
            Some(ZONE_EXTERNAL) => false,
            Some(target) => target != self.from_zone,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImportKind;

    fn rules(pairs: &[(&str, &[&str])]) -> ZoneRules {
        let cfg: Vec<oxplow_config::ZoneRuleConfig> = pairs
            .iter()
            .map(|(zone, patterns)| oxplow_config::ZoneRuleConfig {
                patterns: patterns.iter().map(|p| p.to_string()).collect(),
                zone: zone.to_string(),
                color: None,
            })
            .collect();
        ZoneRules::from_config(&cfg)
    }

    /// tsk251: with no project config there is no rule table at all, so
    /// every path is the `other` sentinel. Oxplow ships no assumptions
    /// about how a project lays out its files.
    #[test]
    fn no_rules_classifies_everything_as_other() {
        let z = ZoneRules::empty();
        assert!(z.is_empty());
        assert_eq!(z.classify("crates/oxplow-db/src/lib.rs"), ZONE_OTHER);
        assert_eq!(z.classify("anything/at/all.ts"), ZONE_OTHER);
    }

    /// The table is ordered and the FIRST match wins — that is what lets
    /// a project put `**/*_test.rs` above `crates/**` so a test file
    /// inside a crate reads as `test`, and a catch-all last.
    #[test]
    fn first_matching_rule_wins() {
        let z = rules(&[
            ("test", &["**/*_test.rs"]),
            ("store", &["crates/db/**"]),
            ("meta", &["**/*.toml"]),
        ]);
        assert_eq!(z.classify("crates/db/src/thing_test.rs"), "test");
        assert_eq!(z.classify("crates/db/src/thing.rs"), "store");
        assert_eq!(z.classify("crates/db/Cargo.toml"), "store");
        assert_eq!(z.classify("tools/build.toml"), "meta");
        assert_eq!(z.classify("scripts/deploy.sh"), ZONE_OTHER);
    }

    #[test]
    fn a_rule_matches_any_of_its_patterns() {
        let z = rules(&[("test", &["**/*_test.rs", "**/tests/**"])]);
        assert_eq!(z.classify("src/a_test.rs"), "test");
        assert_eq!(z.classify("crates/db/tests/it.rs"), "test");
        assert_eq!(z.classify("src/a.rs"), ZONE_OTHER);
    }

    #[test]
    fn windows_separators_normalize_before_matching() {
        let z = rules(&[("ui", &["apps/desktop/**"])]);
        assert_eq!(z.classify("apps\\desktop\\src\\App.tsx"), "ui");
    }

    /// A Rust `use oxplow_db::…` has no path to classify. The module
    /// name is matched against the rule patterns as a path SEGMENT —
    /// the one heuristic in the design, and the reason a project's own
    /// zone table is enough to resolve first-party imports without
    /// oxplow reading the build system.
    #[test]
    fn module_names_resolve_through_the_rule_patterns() {
        let z = rules(&[
            ("store", &["crates/oxplow-db/**"]),
            ("ui", &["apps/desktop/src/**"]),
        ]);
        assert_eq!(z.zone_for_module("oxplow_db").as_deref(), Some("store"));
        assert_eq!(z.zone_for_module("oxplow-db").as_deref(), Some("store"));
        // Not a segment of any pattern → not first-party.
        assert_eq!(z.zone_for_module("serde"), None);
        // A partial segment must not match.
        assert_eq!(z.zone_for_module("oxplow"), None);
    }

    #[test]
    fn cross_zone_only_counts_known_distinct_in_repo_targets() {
        let edge = ImportEdge {
            from_path: "a.rs".into(),
            raw: "use x;".into(),
            module: "x".into(),
            kind: ImportKind::Use,
            start_line: 1,
            end_line: 1,
        };
        let mk = |from: &str, to: Option<&str>| ZonedImportEdge {
            edge: edge.clone(),
            from_zone: from.to_string(),
            to_zone: to.map(|t| t.to_string()),
        };
        assert!(mk("ui", Some("store")).is_cross_zone());
        assert!(!mk("ui", Some("ui")).is_cross_zone());
        assert!(!mk("ui", None).is_cross_zone());
        assert!(!mk("ui", Some(ZONE_EXTERNAL)).is_cross_zone());
    }

    /// Glob semantics are pinned by a fixture shared with the TS
    /// matcher (`apps/desktop/src/components/ChangeAnalysis/zones.ts`),
    /// so the two implementations cannot drift apart.
    #[test]
    fn glob_semantics_match_the_shared_fixture() {
        #[derive(serde::Deserialize)]
        struct Case {
            pattern: String,
            path: String,
            matches: bool,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            cases: Vec<Case>,
        }
        let raw = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/zone-globs.json"
        ));
        let fixture: Fixture = serde_json::from_str(raw).expect("fixture parses");
        assert!(!fixture.cases.is_empty());
        for case in fixture.cases {
            let z = rules(&[("hit", &[case.pattern.as_str()])]);
            let got = z.classify(&case.path) == "hit";
            assert_eq!(
                got, case.matches,
                "pattern {:?} vs path {:?}",
                case.pattern, case.path
            );
        }
    }
}
