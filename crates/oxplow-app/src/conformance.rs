//! What every capability's conformance suite reports: a failed check is a
//! [`Finding`]; a run is its findings and what it left behind
//! ([`SuiteRun`]). An empty list passes.

/// A failed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub check: &'static str,
    pub message: String,
}

/// What a run of a suite came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuiteRun {
    /// The failed checks; none passes.
    pub findings: Vec<Finding>,
    /// What it made and didn't delete (a provider without `delete`, or a
    /// run stopped by a failed delete): what is left in the
    /// implementation's own system for a person to clean up.
    pub left: Vec<String>,
}
