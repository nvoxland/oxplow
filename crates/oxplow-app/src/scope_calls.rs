//! Serving the scopes (`oxplow_domain::scope`) to a command's handler
//! (`.context/commands.md` "Scopes"): each call checked against the
//! command's `needs` — one it didn't declare is refused — counted in
//! the run's trace (`RunContext::trace`), and answered.
//!
//! The same [`Calls`] serves a run inside the bus's transaction (reading
//! on its connection) and a dry run of an example (reading through the
//! SQL gateway, or answered from the example's `answers`).

use oxplow_domain::run::RunContext;
use std::collections::{BTreeMap, VecDeque};

use oxplow_domain::DomainError;
use serde::Deserialize;
use serde_json::Value;

/// The scopes a script — a command's handler, an effect, a provider's
/// `host/call` — can call: what [`Calls::serve`] answers.
pub const CALLABLE: &[&str] = &["sql.read"];

/// A script's need of scope `id`: one it can call, or why not — the one
/// rule a command's `entry:` and an effect meet at load.
pub fn check_callable(id: &str) -> Result<(), String> {
    if CALLABLE.contains(&id) {
        return Ok(());
    }
    let what = if oxplow_domain::scope::scope(id).is_some() {
        "a scope a script can't call"
    } else {
        "not a scope"
    };
    Err(format!(
        "`needs`: `{id}` is {what} — a script calls only {}",
        CALLABLE
            .iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// The most rows one `sql.read` answers. A result over this cap is
/// refused: the row-only answer cannot represent truncation.
pub const SQL_READ_ROW_CAP: usize = 1_000;

/// How `sql.read` is answered: the query's result.
pub type Reader<'a> =
    dyn FnMut(oxplow_db::SqlQuery) -> Result<oxplow_db::SqlQueryResult, DomainError> + 'a;

/// One run's scope calls, counted on its context's trace.
pub struct Calls<'a> {
    needs: &'a [String],
    run: &'a RunContext,
    read: Box<Reader<'a>>,
    /// Answers standing in for a scope's own, in call order (an
    /// example's); a scope without any is served for real.
    answers: BTreeMap<String, VecDeque<Value>>,
    /// The database was busy answering: the run is retried, not failed.
    busy: Option<String>,
}

impl<'a> Calls<'a> {
    pub fn new(run: &'a RunContext, needs: &'a [String], read: Box<Reader<'a>>) -> Self {
        Self {
            needs,
            run,
            read,
            answers: BTreeMap::new(),
            busy: None,
        }
    }

    /// These calls answered from `answers` (per scope, in call order).
    pub fn with_answers(mut self, answers: &BTreeMap<String, Vec<Value>>) -> Self {
        self.answers = answers
            .iter()
            .map(|(id, a)| (id.clone(), a.iter().cloned().collect()))
            .collect();
        self
    }

    /// The handler's call of `id` with `args`: its answer, or why not.
    pub fn serve(&mut self, id: &str, args: Value) -> Result<Value, String> {
        if oxplow_domain::scope::scope(id).is_none() {
            return Err(format!("no scope `{id}`"));
        }
        // Decided and recorded on the run either way.
        let decision = self
            .run
            .decide(oxplow_domain::grant::Need::Scope(id.to_string()));
        if !self.needs.iter().any(|n| n == id) {
            return Err(format!(
                "`{id}` isn't in the command's `needs` — declare it to call it"
            ));
        }
        if let oxplow_domain::grant::Decision::Denied(why) = decision {
            return Err(format!("`{id}`: {why}"));
        }
        self.run.trace().count(id);
        self.run.used_scope(id);
        if let Some(answers) = self.answers.get_mut(id) {
            return answers
                .pop_front()
                .ok_or_else(|| format!("the example's `answers` for `{id}` ran out"));
        }
        match id {
            "sql.read" => self.sql_read(args),
            other => Err(check_callable(other).err().unwrap_or_default()),
        }
    }

    /// The run these calls are part of.
    pub fn run(&self) -> &RunContext {
        self.run
    }

    /// Whether answering found the database busy (the run is retried).
    pub fn take_busy(&mut self) -> Option<String> {
        self.busy.take()
    }

    fn sql_read(&mut self, args: Value) -> Result<Value, String> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Page {
            limit: usize,
            #[serde(default)]
            offset: usize,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Args {
            sql: String,
            #[serde(default)]
            params: serde_json::Map<String, Value>,
            #[serde(default)]
            page: Option<Page>,
        }
        let args: Args = serde_json::from_value(args)
            .map_err(|e| format!("`sql.read` takes {{ sql, params?, page? }}: {e}"))?;
        let mut params: Vec<(String, oxplow_db::SqlCell)> = args
            .params
            .into_iter()
            .map(|(k, v)| (k, oxplow_db::SqlCell::from(v)))
            .collect();
        // A page: this slice of the query's rows, one more asked for to
        // tell whether any follow.
        let (sql, cap) = match &args.page {
            None => (args.sql.clone(), SQL_READ_ROW_CAP),
            Some(page) if page.limit == 0 || page.limit > SQL_READ_ROW_CAP => {
                return Err(format!(
                    "`sql.read`'s `page.limit` is at least 1 and at most {SQL_READ_ROW_CAP}"
                ))
            }
            Some(page) => {
                params.push((
                    "__page_limit".into(),
                    oxplow_db::SqlCell::Int(page.limit as i64 + 1),
                ));
                params.push((
                    "__page_offset".into(),
                    oxplow_db::SqlCell::Int(page.offset as i64),
                ));
                (
                    format!(
                        "SELECT * FROM ({}) LIMIT :__page_limit OFFSET :__page_offset",
                        args.sql.trim().trim_end_matches(';')
                    ),
                    page.limit + 1,
                )
            }
        };
        let query = oxplow_db::SqlQuery::new(&sql)
            .named(params)
            .limit(Some(cap));
        let result = match (self.read)(query) {
            Ok(result) => result,
            Err(DomainError::Busy(m)) => {
                self.busy = Some(m.clone());
                return Err(m);
            }
            Err(e) => {
                return Err(format!(
                    "`sql.read`: {}",
                    e.to_string().replacen("invalid value: ", "", 1)
                ))
            }
        };
        let Some(page) = args.page else {
            if result.truncated {
                return Err(format!(
                    "`sql.read` returned more than {SQL_READ_ROW_CAP} rows; aggregate in SQL, \
                     explicitly narrow the query, or ask for it a `page` at a time; partial \
                     results are not returned"
                ));
            }
            return Ok(Value::Array(rows_json(&result)));
        };
        let mut rows = rows_json(&result);
        let more = rows.len() > page.limit;
        rows.truncate(page.limit);
        let mut answer = serde_json::json!({ "rows": rows, "more": more });
        if more {
            answer["next_offset"] = (page.offset + page.limit).into();
        }
        if !args.sql.to_ascii_lowercase().contains("order by") {
            answer["warning"] = Value::String(
                "the query has no ORDER BY, so its pages may overlap or skip rows".into(),
            );
        }
        Ok(answer)
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
                generation: None,
                coverage: Vec::new(),
            })
        })
    }

    #[test]
    fn a_call_is_checked_against_needs_counted_and_answered() {
        let (needs, run, mut asked) = (
            needs(&["sql.read"]),
            crate::run_context::for_dry_run(),
            vec![],
        );
        let mut calls = Calls::new(&run, &needs, reader(&mut asked));
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
        assert!(
            err.contains("`sql.read` takes { sql, params?, page? }"),
            "{err}"
        );
        drop(calls);
        assert_eq!(asked, vec!["SELECT 1 AS n", "SELECT 2"]);
        assert_eq!(run.trace().summary(), [("sql.read".to_string(), 3)].into());
    }

    #[test]
    fn a_scope_not_declared_or_not_known_is_refused_uncounted() {
        let (none, run, mut asked) = (
            needs(&["work_items"]),
            crate::run_context::for_dry_run(),
            vec![],
        );
        let mut calls = Calls::new(&run, &none, reader(&mut asked));
        let err = calls
            .serve("sql.read", json!({ "sql": "SELECT 1" }))
            .unwrap_err();
        assert!(
            err.contains("`sql.read` isn't in the command's `needs`"),
            "{err}"
        );
        let err = calls.serve("fs.erase", json!({})).unwrap_err();
        assert!(err.contains("no scope `fs.erase`"), "{err}");
        drop(calls);
        assert!(asked.is_empty());
        assert!(run.trace().summary().is_empty());
    }

    #[test]
    fn an_examples_answers_stand_in_until_they_run_out() {
        let (needs, run, mut asked) = (
            needs(&["sql.read"]),
            crate::run_context::for_dry_run(),
            vec![],
        );
        let answers = [("sql.read".to_string(), vec![json!([{ "a": 1 }])])].into();
        let mut calls = Calls::new(&run, &needs, reader(&mut asked)).with_answers(&answers);
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
        let (needs, run) = (needs(&["sql.read"]), crate::run_context::for_dry_run());
        let mut calls = Calls::new(
            &run,
            &needs,
            Box::new(|_| Err(DomainError::Busy("locked".into()))),
        );
        assert!(calls.serve("sql.read", json!({ "sql": "x" })).is_err());
        assert_eq!(calls.take_busy().as_deref(), Some("locked"));
    }

    const NUMBER_ROWS: &str = "WITH RECURSIVE numbers(n) AS (
        SELECT 1 WHERE :total > 0
        UNION ALL SELECT n + 1 FROM numbers WHERE n < :total
    ) SELECT n FROM numbers ORDER BY n";

    /// The row-only scope must never present a capped prefix as the full
    /// input. The gateway probes one extra row, so exactly the cap is OK.
    #[tokio::test]
    async fn sql_read_requires_complete_results_at_the_row_cap() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        for total in [
            0,
            SQL_READ_ROW_CAP - 1,
            SQL_READ_ROW_CAP,
            SQL_READ_ROW_CAP + 1,
        ] {
            let result = crate::effects::run_over(
                &layer,
                &crate::run_context::for_dry_run(),
                &needs(&["sql.read"]),
                &format!(
                    r#"def transform(x):
    rows = scope("sql.read", {{"sql": {sql:?}, "params": {{"total": {total}}}}})
    return {{"skip": str(len(rows))}}
"#,
                    sql = NUMBER_ROWS,
                ),
                json!({}),
                None,
            )
            .await;
            if total > SQL_READ_ROW_CAP {
                let err = result.unwrap_err();
                assert!(
                    err.contains("`sql.read` returned more than 1000 rows"),
                    "{err}"
                );
            } else {
                assert_eq!(
                    result.unwrap(),
                    crate::effects::Reaction::Skip(total.to_string())
                );
            }
        }
    }

    /// Asking for a page reads past the cap a page at a time: each page
    /// says whether more rows follow and where the next starts, and a
    /// query with no `ORDER BY` is answered with a warning (its pages may
    /// overlap or skip).
    #[tokio::test]
    async fn sql_read_pages_through_a_result_on_request() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        let script = format!(
            r#"def transform(x):
    total = 0
    pages = 0
    offset = 0
    for i in range(10):
        page = scope("sql.read", {{"sql": {sql:?}, "params": {{"total": 2500}}, "page": {{"limit": 1000, "offset": offset}}}})
        total += len(page["rows"])
        pages += 1
        if not page["more"]:
            break
        offset = page["next_offset"]
    return {{"skip": "%d rows in %d pages, last %d" % (total, pages, page["rows"][-1]["n"])}}
"#,
            sql = NUMBER_ROWS,
        );
        let out = crate::effects::run_over(
            &layer,
            &crate::run_context::for_dry_run(),
            &needs(&["sql.read"]),
            &script,
            json!({}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            out,
            crate::effects::Reaction::Skip("2500 rows in 3 pages, last 2500".into())
        );
        let unordered = r#"def transform(x):
    page = scope("sql.read", {"sql": "SELECT 1 AS n", "page": {"limit": 10}})
    return {"skip": page.get("warning", "")}
"#;
        let out = crate::effects::run_over(
            &layer,
            &crate::run_context::for_dry_run(),
            &needs(&["sql.read"]),
            unordered,
            json!({}),
            None,
        )
        .await
        .unwrap();
        assert!(
            matches!(&out, crate::effects::Reaction::Skip(w) if w.contains("ORDER BY")),
            "{out:?}"
        );
        // A page is at most the cap.
        let big = r#"def transform(x):
    return scope("sql.read", {"sql": "SELECT 1 ORDER BY 1", "page": {"limit": 5000}})
"#;
        let err = crate::effects::run_over(
            &layer,
            &crate::run_context::for_dry_run(),
            &needs(&["sql.read"]),
            big,
            json!({}),
            None,
        )
        .await
        .unwrap_err();
        assert!(err.contains("at most 1000"), "{err}");
    }

    /// Aggregate in SQL to keep all source rows in the calculation while
    /// returning a complete result that fits the scope's cap.
    #[tokio::test]
    async fn sql_read_aggregates_all_rows_before_applying_the_cap() {
        let db = oxplow_db::Database::in_memory();
        db.read(|conn| {
            let (needs, run) = (needs(&["sql.read"]), crate::run_context::for_dry_run());
            let mut calls = Calls::new(
                &run,
                &needs,
                Box::new(|q| oxplow_db::semantic_layer::read_on(conn, &q)),
            );
            let sql = format!("SELECT count(*) AS n, sum(n) AS total FROM ({NUMBER_ROWS})");
            assert_eq!(
                calls
                    .serve(
                        "sql.read",
                        json!({ "sql": sql, "params": { "total": 1001 } })
                    )
                    .unwrap(),
                json!([{ "n": 1001, "total": 501501 }]),
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}
