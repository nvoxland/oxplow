use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum DomainError {
    /// A caller-supplied value failed validation. Not retryable;
    /// surface it to the user.
    #[error("invalid value: {0}")]
    Invalid(String),
    #[error("not found")]
    NotFound,
    #[error("invariant violated: {0}")]
    Invariant(String),
    /// A storage constraint rejected the write (UNIQUE, FK, CHECK,
    /// NOT NULL). Usually a duplicate action or a caller bug — not
    /// retryable, but distinguishable from validation errors.
    #[error("constraint violated: {0}")]
    Constraint(String),
    /// The storage layer was busy or locked (`SQLITE_BUSY` /
    /// `SQLITE_LOCKED`). Transient — retrying may succeed.
    #[error("storage busy: {0}")]
    Busy(String),
    /// Any other storage-layer failure (I/O, corruption, pool
    /// exhaustion, a panicked DB task). Neither a validation error
    /// nor user-actionable.
    #[error("storage: {0}")]
    Storage(String),
}

impl DomainError {
    /// Transient storage contention — a retry may succeed. Everything
    /// else is deterministic and retrying just repeats the failure.
    pub fn is_retryable(&self) -> bool {
        matches!(self, DomainError::Busy(_))
    }
}
