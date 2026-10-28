//! Admission of loader results into a page's combined result budget.
//! Count the wire JSON through a bounded writer; never build a serialized
//! copy just to measure a potentially large cell. A result retains a
//! prefix of whole rows, with its read/coverage provenance intact.

use std::io::{self, Write};

use oxplow_db::SqlQueryResult;
use oxplow_domain::DomainError;
use serde::Serialize;

pub(super) struct ResultBudget {
    remaining: usize,
}

impl ResultBudget {
    pub(super) fn new(bytes: usize) -> Self {
        Self { remaining: bytes }
    }

    pub(super) fn remaining(&self) -> usize {
        self.remaining
    }

    /// Restore a checkpoint when an embed failed and none of its results
    /// is retained in the returned tree.
    pub(super) fn restore(&mut self, remaining: usize) {
        self.remaining = remaining;
    }

    /// Admit the result's metadata and a prefix of its rows. The returned
    /// boolean says that data was cropped. If the columns alone exceed
    /// the remaining budget, omit columns and rows together. Read and
    /// coverage metadata is never silently discarded; an unrepresentable
    /// metadata envelope fails the page with a fixed, bounded diagnostic.
    pub(super) fn admit(&mut self, result: &mut SqlQueryResult) -> Result<bool, DomainError> {
        let rows = std::mem::take(&mut result.rows);
        let mut cropped = false;
        let mut base = serialized_bytes(result, self.remaining);
        if base.is_none() {
            result.columns.clear();
            result.truncated = true;
            cropped = true;
            base = serialized_bytes(result, self.remaining);
        }
        let Some(base) = base else {
            return Err(DomainError::Invalid(
                "page result metadata exceeds the combined result budget".into(),
            ));
        };
        self.remaining -= base;
        if cropped {
            return Ok(true);
        }
        for row in rows {
            // The empty rows array's brackets are already in `base`.
            let comma = usize::from(!result.rows.is_empty());
            let Some(available) = self.remaining.checked_sub(comma) else {
                cropped = true;
                break;
            };
            let Some(bytes) = serialized_bytes(&row, available) else {
                cropped = true;
                break;
            };
            self.remaining -= bytes + comma;
            result.rows.push(row);
        }
        result.truncated |= cropped;
        Ok(cropped)
    }
}

/// Exact serialized size up to `limit`, without allocating output. An
/// overflow stops serialization immediately, including inside a cell.
fn serialized_bytes(value: &impl Serialize, limit: usize) -> Option<usize> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let size = self.bytes.saturating_add(bytes.len());
            if size > self.limit {
                return Err(io::Error::other("page result budget exhausted"));
            }
            self.bytes = size;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value).ok()?;
    Some(counter.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::SqlCell;

    #[test]
    fn admission_counts_json_escaping_and_retains_whole_rectangular_rows() {
        let row = vec![SqlCell::Text("\"\\\n\u{0001}é".into()), SqlCell::Int(7)];
        let mut result = SqlQueryResult {
            columns: vec!["text".into(), "n".into()],
            ..Default::default()
        };
        let base = serde_json::to_vec(&result).unwrap().len();
        let row_size = serde_json::to_vec(&row).unwrap().len();
        result.rows = vec![row.clone(), row.clone()];
        let mut budget = ResultBudget::new(base + row_size);
        assert!(budget.admit(&mut result).unwrap());
        assert_eq!(result.rows, [row]);
        assert!(result.truncated);
        assert!(serde_json::to_vec(&result).unwrap().len() <= base + row_size);
        assert_eq!(result.columns.len(), result.rows[0].len());
    }

    #[test]
    fn oversized_columns_drop_the_rows_together_without_losing_reads() {
        let mut result = SqlQueryResult {
            columns: vec!["x".repeat(4096)],
            rows: vec![vec![SqlCell::Int(1)]],
            ..Default::default()
        };
        result.reads.datasets.push("v_work_item".into());
        assert!(ResultBudget::new(512).admit(&mut result).unwrap());
        assert!(result.columns.is_empty());
        assert!(result.rows.is_empty());
        assert!(result.truncated);
        assert_eq!(result.reads.datasets, ["v_work_item"]);
    }

    #[test]
    fn metadata_overflow_fails_with_a_bounded_diagnostic() {
        let mut result = SqlQueryResult::default();
        result.reads.datasets.push("private-".repeat(1024));
        let error = ResultBudget::new(512).admit(&mut result).unwrap_err();
        assert_eq!(
            error.message(),
            "page result metadata exceeds the combined result budget"
        );
        assert_eq!(result.reads.datasets.len(), 1);
    }

    #[test]
    fn an_existing_truncation_and_provenance_survive_admission() {
        let mut result = SqlQueryResult {
            columns: vec!["n".into()],
            rows: vec![vec![SqlCell::Int(1)]],
            truncated: true,
            generation: Some("generation-a".into()),
            ..Default::default()
        };
        assert!(!ResultBudget::new(512).admit(&mut result).unwrap());
        assert!(result.truncated);
        assert_eq!(result.generation.as_deref(), Some("generation-a"));
    }
}
