//! Serving the host capabilities (`oxplow_domain::host_capability`) to a
//! command's handler (`.context/commands.md` "Host capabilities"): each
//! call checked against the command's `needs` — one it didn't declare is
//! refused — counted in the run's [`CapabilityTrace`], and answered.
//!
//! The same [`Calls`] serves a run inside the bus's transaction (reading
//! on its connection) and a dry run of an example (reading through the
//! SQL gateway, or answered from the example's `answers`).

use std::collections::{BTreeMap, VecDeque};

use oxplow_domain::DomainError;
use serde::Deserialize;
use serde_json::Value;

/// The most rows one `sql.read` answers.
pub const SQL_READ_ROW_CAP: usize = 1_000;

/// What a run called, per capability: the per-run summary its audit row
/// records. One per run, shared by the runs nested in it; a retried run
/// starts a fresh one.
#[derive(Debug, Default)]
pub struct CapabilityTrace(parking_lot::Mutex<BTreeMap<String, u32>>);

impl CapabilityTrace {
    fn count(&self, id: &str) {
        *self.0.lock().entry(id.to_string()).or_default() += 1;
    }

    /// Calls made before the run, counted with it (an effect's script
    /// runs before its reaction does).
    pub fn add(&self, counts: &BTreeMap<String, u32>) {
        add_counts(&mut self.0.lock(), counts.clone());
    }

    /// Each capability called, and how often.
    pub fn summary(&self) -> BTreeMap<String, u32> {
        self.0.lock().clone()
    }
}

/// `from`'s counts added to `into`'s.
pub fn add_counts(into: &mut BTreeMap<String, u32>, from: BTreeMap<String, u32>) {
    for (id, n) in from {
        *into.entry(id).or_default() += n;
    }
}

/// How `sql.read` is answered: the query's result.
pub type Reader<'a> =
    dyn FnMut(oxplow_db::SqlQuery) -> Result<oxplow_db::SqlQueryResult, DomainError> + 'a;

/// One handler run's capability calls.
pub struct Calls<'a> {
    needs: &'a [String],
    trace: &'a CapabilityTrace,
    read: Box<Reader<'a>>,
    /// Answers standing in for a capability's own, in call order (an
    /// example's); a capability without any is served for real.
    answers: BTreeMap<String, VecDeque<Value>>,
    /// The database was busy answering: the run is retried, not failed.
    busy: Option<String>,
}

impl<'a> Calls<'a> {
    pub fn new(needs: &'a [String], trace: &'a CapabilityTrace, read: Box<Reader<'a>>) -> Self {
        Self {
            needs,
            trace,
            read,
            answers: BTreeMap::new(),
            busy: None,
        }
    }

    /// These calls answered from `answers` (per capability, in call order).
    pub fn with_answers(mut self, answers: &BTreeMap<String, Vec<Value>>) -> Self {
        self.answers = answers
            .iter()
            .map(|(id, a)| (id.clone(), a.iter().cloned().collect()))
            .collect();
        self
    }

    /// The handler's call of `id` with `args`: its answer, or why not.
    pub fn serve(&mut self, id: &str, args: Value) -> Result<Value, String> {
        if oxplow_domain::host_capability::host_capability(id).is_none() {
            return Err(format!("no host capability `{id}`"));
        }
        if !self.needs.iter().any(|n| n == id) {
            return Err(format!(
                "`{id}` isn't in the command's `needs` — declare it to call it"
            ));
        }
        self.trace.count(id);
        if let Some(answers) = self.answers.get_mut(id) {
            return answers
                .pop_front()
                .ok_or_else(|| format!("the example's `answers` for `{id}` ran out"));
        }
        match id {
            "sql.read" => self.sql_read(args),
            other => Err(format!("`{other}` can't be called from a handler yet")),
        }
    }

    /// Whether answering found the database busy (the run is retried).
    pub fn take_busy(&mut self) -> Option<String> {
        self.busy.take()
    }

    fn sql_read(&mut self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            sql: String,
            #[serde(default)]
            params: serde_json::Map<String, Value>,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| format!("`sql.read` takes {{ sql, params? }}: {e}"))?;
        let query = oxplow_db::SqlQuery::new(&args.sql)
            .named(
                args.params
                    .into_iter()
                    .map(|(k, v)| (k, oxplow_db::SqlCell::from(v)))
                    .collect(),
            )
            .limit(Some(SQL_READ_ROW_CAP));
        match (self.read)(query) {
            Ok(result) => Ok(Value::Array(rows_json(&result))),
            Err(DomainError::Busy(m)) => {
                self.busy = Some(m.clone());
                Err(m)
            }
            Err(e) => Err(format!(
                "`sql.read`: {}",
                e.to_string().replacen("invalid value: ", "", 1)
            )),
        }
    }
}

/// A query result as a handler sees it: one object per row.
pub fn rows_json(result: &oxplow_db::SqlQueryResult) -> Vec<Value> {
    result
        .rows
        .iter()
        .map(|row| {
            Value::Object(
                result
                    .columns
                    .iter()
                    .zip(row)
                    .map(|(c, v)| (c.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn needs(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    fn reader(asked: &mut Vec<String>) -> Box<Reader<'_>> {
        Box::new(move |q: oxplow_db::SqlQuery| {
            asked.push(q.sql.clone());
            Ok(oxplow_db::SqlQueryResult {
                columns: vec!["n".into()],
                rows: vec![vec![oxplow_db::SqlCell::from(json!(1))]],
                truncated: false,
                reads: Default::default(),
                freshness: Vec::new(),
            })
        })
    }

    #[test]
    fn a_call_is_checked_against_needs_counted_and_answered() {
        let (needs, trace, mut asked) = (needs(&["sql.read"]), CapabilityTrace::default(), vec![]);
        let mut calls = Calls::new(&needs, &trace, reader(&mut asked));
        let rows = calls
            .serve(
                "sql.read",
                json!({ "sql": "SELECT 1 AS n", "params": { "x": 1 } }),
            )
            .unwrap();
        assert_eq!(rows, json!([{ "n": 1 }]));
        calls
            .serve("sql.read", json!({ "sql": "SELECT 2" }))
            .unwrap();
        let err = calls
            .serve("sql.read", json!({ "query": "x" }))
            .unwrap_err();
        assert!(err.contains("`sql.read` takes { sql, params? }"), "{err}");
        drop(calls);
        assert_eq!(asked, vec!["SELECT 1 AS n", "SELECT 2"]);
        assert_eq!(trace.summary(), [("sql.read".to_string(), 3)].into());
    }

    #[test]
    fn a_capability_not_declared_or_not_known_is_refused_uncounted() {
        let (none, trace, mut asked) = (needs(&["work_items"]), CapabilityTrace::default(), vec![]);
        let mut calls = Calls::new(&none, &trace, reader(&mut asked));
        let err = calls
            .serve("sql.read", json!({ "sql": "SELECT 1" }))
            .unwrap_err();
        assert!(
            err.contains("`sql.read` isn't in the command's `needs`"),
            "{err}"
        );
        let err = calls.serve("fs.erase", json!({})).unwrap_err();
        assert!(err.contains("no host capability `fs.erase`"), "{err}");
        drop(calls);
        assert!(asked.is_empty());
        assert!(trace.summary().is_empty());
    }

    #[test]
    fn an_examples_answers_stand_in_until_they_run_out() {
        let (needs, trace, mut asked) = (needs(&["sql.read"]), CapabilityTrace::default(), vec![]);
        let answers = [("sql.read".to_string(), vec![json!([{ "a": 1 }])])].into();
        let mut calls = Calls::new(&needs, &trace, reader(&mut asked)).with_answers(&answers);
        assert_eq!(
            calls.serve("sql.read", json!({ "sql": "x" })).unwrap(),
            json!([{ "a": 1 }])
        );
        let err = calls.serve("sql.read", json!({ "sql": "x" })).unwrap_err();
        assert!(err.contains("`answers` for `sql.read` ran out"), "{err}");
        drop(calls);
        assert!(asked.is_empty());
    }

    #[test]
    fn a_busy_database_is_kept_for_a_retry() {
        let (needs, trace) = (needs(&["sql.read"]), CapabilityTrace::default());
        let mut calls = Calls::new(
            &needs,
            &trace,
            Box::new(|_| Err(DomainError::Busy("locked".into()))),
        );
        assert!(calls.serve("sql.read", json!({ "sql": "x" })).is_err());
        assert_eq!(calls.take_busy().as_deref(), Some("locked"));
    }
}
