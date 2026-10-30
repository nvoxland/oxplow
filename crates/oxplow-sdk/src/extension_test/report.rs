use super::*;

/// What `extension test` found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestReport {
    pub name: String,
    /// No errors.
    pub ok: bool,
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
    /// What wasn't run here (an example its host withholds, `ui` steps
    /// with no app or no drawing window): never a pass, so its case is
    /// skipped.
    pub skipped: Vec<Diagnostic>,
    /// The folder a person edits (`Extension::root`): what the
    /// diagnostics are shown under (`Diagnostic::render`).
    pub root: String,
    /// Golden transcripts written by `--bless`.
    pub blessed: Vec<String>,
    /// What ran: `check`, `provider <id>`, `<capability> suite`.
    pub ran: Vec<String>,
    /// How each of its `imports:` resolved here (`crate::import_lines`).
    pub imports: Vec<String>,
    /// What serves each of its `ai_profiles:` (`crate::profile_lines`).
    pub profiles: Vec<String>,
    /// The throwaway's activation generation the examples ran against.
    pub generation: Option<String>,
    /// What the extension contributes, by public id
    /// (its ref (`<kind>:<extension>/<id>`, `command:<bus id>`)).
    pub contributes: Vec<String>,
    /// What it contributes that this host can't run — a person sees it in
    /// the app (pages, panels, slots, decorators, replacements, skills).
    pub unsupported: Vec<String>,
    /// What the run made in a provider's own system and didn't remove —
    /// the refs a provider example's `create` returned, and the items the
    /// conformance suite filed and couldn't delete. Against a real
    /// service, a person (or a live test) cleans up exactly these.
    pub left: Vec<String>,
    /// Each thing that ran and how it went: `check`, `example <name>`,
    /// `answers <page>`, `provider <id>`, … — a test case each in the JUnit
    /// form.
    pub cases: Vec<TestCase>,
    /// `--require-ui`: `ui` steps that can't draw are errors, not skips.
    #[serde(skip)]
    pub(crate) require_ui: bool,
}

/// One thing `extension test` ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestCase {
    pub name: String,
    pub outcome: CaseOutcome,
    /// The errors it added as the lines a person reads, one per line, or
    /// why it was skipped; `None` when it passed.
    pub message: Option<String>,
    pub duration_ms: u64,
}

/// How a [`TestCase`] went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CaseOutcome {
    Passed,
    Failed,
    /// Not run here: what it asserts is unknown, so it never counts as a pass.
    Skipped,
}

/// Where a report stood when a case started: what [`TestReport::finish_case`]
/// counts the case's errors and skips from.
#[derive(Clone, Copy)]
pub(super) struct Mark {
    pub(super) errors: usize,
    pub(super) skipped: usize,
    pub(super) started: std::time::Instant,
}

impl TestReport {
    /// The errors as the lines a person reads (`<folder>/<file>:<line>: …`).
    pub fn error_lines(&self) -> Vec<String> {
        self.errors.iter().map(|d| d.render(&self.root)).collect()
    }

    /// The warnings as the lines a person reads.
    pub fn warning_lines(&self) -> Vec<String> {
        self.warnings.iter().map(|d| d.render(&self.root)).collect()
    }

    /// What wasn't run, as the lines a person reads.
    pub fn skipped_lines(&self) -> Vec<String> {
        self.skipped.iter().map(|d| d.render(&self.root)).collect()
    }

    /// An empty report for part of extension `name`'s run, to [`Self::absorb`].
    pub(super) fn part(name: &str, root: &str) -> TestReport {
        TestReport {
            name: name.to_string(),
            ok: false,
            errors: Vec::new(),
            warnings: Vec::new(),
            skipped: Vec::new(),
            root: root.to_string(),
            blessed: Vec::new(),
            ran: Vec::new(),
            imports: Vec::new(),
            profiles: Vec::new(),
            generation: None,
            contributes: Vec::new(),
            unsupported: Vec::new(),
            left: Vec::new(),
            cases: Vec::new(),
            require_ui: false,
        }
    }

    /// What `part` found and ran, after what this report has.
    pub(super) fn absorb(&mut self, part: TestReport) {
        self.errors.extend(part.errors);
        self.warnings.extend(part.warnings);
        self.skipped.extend(part.skipped);
        self.blessed.extend(part.blessed);
        self.ran.extend(part.ran);
        self.left.extend(part.left);
        self.cases.extend(part.cases);
    }

    /// Where this report stands, for a case starting now.
    pub(super) fn mark(&self) -> Mark {
        Mark {
            errors: self.errors.len(),
            skipped: self.skipped.len(),
            started: std::time::Instant::now(),
        }
    }

    /// Record case `name`, which started at `mark`: failed when it added
    /// errors, else skipped when it added skips, else passed.
    pub(super) fn finish_case(&mut self, name: String, mark: Mark) {
        let errors = &self.errors[mark.errors.min(self.errors.len())..];
        let skipped = &self.skipped[mark.skipped.min(self.skipped.len())..];
        let lines = |found: &[Diagnostic]| {
            found
                .iter()
                .map(|d| d.render(&self.root))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let (outcome, message) = if !errors.is_empty() {
            (CaseOutcome::Failed, Some(lines(errors)))
        } else if !skipped.is_empty() {
            (CaseOutcome::Skipped, Some(lines(skipped)))
        } else {
            (CaseOutcome::Passed, None)
        };
        self.cases.push(TestCase {
            name,
            outcome,
            message,
            duration_ms: u64::try_from(mark.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
    }
}

/// The report as text (`error:` / `warning:` lines, then a summary) or
/// JSON.
pub fn render(report: &TestReport, format: crate::Format) -> String {
    match format {
        crate::Format::Json => serde_json::to_string_pretty(report).expect("report serializes"),
        crate::Format::Junit => junit(report),
        crate::Format::Text => {
            let mut out = String::new();
            for e in &report.errors {
                out.push_str(&format!("error: {}\n", e.render(&report.root)));
            }
            for w in &report.warnings {
                out.push_str(&format!("warning: {}\n", w.render(&report.root)));
            }
            for k in &report.skipped {
                out.push_str(&format!("skipped: {}\n", k.render(&report.root)));
            }
            for b in &report.blessed {
                out.push_str(&format!(
                    "blessed: {}\n",
                    crate::shown_under(&report.root, b)
                ));
            }
            for l in &report.left {
                out.push_str(&format!("left in the provider: {l}\n"));
            }
            for i in &report.imports {
                out.push_str(&format!("import {i}\n"));
            }
            for line in &report.profiles {
                out.push_str(&format!("{line}\n"));
            }
            if !report.unsupported.is_empty() {
                out.push_str(&format!(
                    "not run here (the app shows them): {}\n",
                    report.unsupported.join(", ")
                ));
            }
            if let Some(generation) = &report.generation {
                out.push_str(&format!("generation: {generation}\n"));
            }
            let skipped = if report.skipped.is_empty() {
                String::new()
            } else {
                format!(", {} skipped", report.skipped.len())
            };
            out.push_str(&format!(
                "{}: {} error{}, {} warning{}{skipped}; ran {}\n",
                report.name,
                report.errors.len(),
                if report.errors.len() == 1 { "" } else { "s" },
                report.warnings.len(),
                if report.warnings.len() == 1 { "" } else { "s" },
                ran_summary(&report.ran)
            ));
            out
        }
    }
}

/// What ran, in order, with the examples as one count where the first
/// stood (the JUnit form names each).
pub(super) fn ran_summary(ran: &[String]) -> String {
    let examples = ran.iter().filter(|r| r.starts_with("example ")).count();
    let (mut parts, mut counted) = (Vec::new(), false);
    for r in ran {
        if !r.starts_with("example ") {
            parts.push(r.clone());
        } else if !counted {
            counted = true;
            parts.push(format!(
                "{examples} example{}",
                if examples == 1 { "" } else { "s" }
            ));
        }
    }
    parts.join(", ")
}

/// `report` as one JUnit `<testsuite>` (no XML prolog, so a caller can
/// put several in one `<testsuites>`): a `<testcase>` per case, a
/// `<failure>` holding the errors it added, a `<skipped>` saying why it
/// wasn't run.
pub(super) fn junit(report: &TestReport) -> String {
    fn escape(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
    let count = |outcome| report.cases.iter().filter(|c| c.outcome == outcome).count();
    let (failures, skipped) = (count(CaseOutcome::Failed), count(CaseOutcome::Skipped));
    let total_ms: u64 = report.cases.iter().map(|c| c.duration_ms).sum();
    let seconds = |ms: u64| format!("{:.3}", ms as f64 / 1000.0);
    let name = escape(&report.name);
    let mut out = format!(
        "<testsuite name=\"{name}\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\" time=\"{}\">\n",
        report.cases.len(),
        seconds(total_ms)
    );
    for case in &report.cases {
        out.push_str(&format!(
            "  <testcase name=\"{}\" classname=\"{name}\" time=\"{}\"",
            escape(&case.name),
            seconds(case.duration_ms)
        ));
        let element = match case.outcome {
            CaseOutcome::Passed => None,
            CaseOutcome::Failed => Some("failure"),
            CaseOutcome::Skipped => Some("skipped"),
        };
        match (element, &case.message) {
            (Some(element), Some(message)) => out.push_str(&format!(
                ">\n    <{element} message=\"{}\">{}</{element}>\n  </testcase>\n",
                escape(message.lines().next().unwrap_or_default()),
                escape(message)
            )),
            _ => out.push_str("/>\n"),
        }
    }
    out.push_str("</testsuite>\n");
    out
}
