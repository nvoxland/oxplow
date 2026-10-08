//! Complete-scope captures that store only what changed (V39).
//!
//! A `complete`-scope capture restates its measure's whole population, and
//! consecutive scans of one worktree mostly repeat each other (a duplicate
//! scan restated ~19k blocks ~44 times a day, 0.3-7% of them changed). So a
//! finished capture of such a measure stores only the facts that are new or
//! changed since its producer's previous finished capture in the same
//! stream, and **holds** the ones it repeats:
//!
//! - a fact is held by the captures of its chain from its own capture up to
//!   its `last_capture_id` (NULL while the newest capture still holds it);
//! - a `fact_chain (capture, measure)` row marks a capture as holding the
//!   measure's facts that way; `from_capture_id` is where the chain was last
//!   stored whole, so every fact it holds is at or after it.
//!
//! A capture drops a fact by closing it: `last_capture_id` becomes the
//! previous capture. It stores the measure whole — closing everything,
//! inserting everything — when there's nothing to stand on, when more than
//! half of what it restates changed, or after [`MAX_CHAIN_DEPTH`] captures,
//! which bounds how far back a read looks.
//!
//! Every read of facts goes through [`held_facts_sql`] (and `v_fact` through
//! the same rule), so a reader sees each capture's whole set, labelled with
//! that capture. Pruning moves a fact a doomed capture stored up to the
//! next capture that holds it ([`rehome`]).

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, OptionalExtension};

use crate::fact_store::{fact_row_mapper, insert_fact, FactRow, NewFact, FACT_ROW_COLS};

/// The most captures a chain runs before one is stored whole.
pub const MAX_CHAIN_DEPTH: i64 = 32;

/// The facts chain captures hold: `ch` is the holder's `fact_chain` row,
/// `c` the holder, `f` a fact it holds and `fc` the capture that stored it.
/// The CROSS JOINs pin that order: from the chain rows a read wants, never
/// from every fact (the planner, guessing at `fact_chain`, went fact-first
/// and scanned the stream's later captures for each fact). The bounds are
/// spelled out because with `BETWEEN` here SQLite drops the upper one.
const CHAIN_HELD: &str = "fact_chain ch
       CROSS JOIN metric_capture c ON c.id = ch.capture_id
       CROSS JOIN fact f ON f.measure_id = ch.measure_id
                  AND f.capture_id >= ch.from_capture_id AND f.capture_id <= ch.capture_id
                  AND (f.last_capture_id IS NULL OR f.last_capture_id >= ch.capture_id)
       CROSS JOIN metric_capture fc ON fc.id = f.capture_id
                             AND fc.stream_id = c.stream_id AND fc.producer = c.producer
                             AND fc.status = 'done'";

/// Which captures' facts a read wants.
pub(crate) enum Holders<'a> {
    All,
    /// The captures of the stream bound at this parameter.
    Stream(&'a str),
    /// The captures whose ids are the JSON array bound at this parameter.
    Ids(&'a str),
}

/// A read of measure `measure` (a bound parameter) as held by `holders`,
/// oldest first: [`FACT_ROW_COLS`] per held fact, with the holder's
/// capture columns.
pub(crate) fn held_facts_sql(measure: &str, holders: Holders) -> String {
    let (own, chained) = match holders {
        Holders::All => ("1".to_string(), "1".to_string()),
        Holders::Stream(p) => (format!("c.stream_id = {p}"), format!("c.stream_id = {p}")),
        Holders::Ids(p) => (
            format!("c.id IN (SELECT value FROM json_each({p}))"),
            format!("ch.capture_id IN (SELECT value FROM json_each({p}))"),
        ),
    };
    format!(
        "SELECT {FACT_ROW_COLS} FROM fact f JOIN metric_capture c ON c.id = f.capture_id
          WHERE f.measure_id = {measure} AND {own}
            AND NOT EXISTS (SELECT 1 FROM fact_chain x
                             WHERE x.capture_id = c.id AND x.measure_id = f.measure_id)
         UNION ALL
         SELECT {FACT_ROW_COLS} FROM {CHAIN_HELD}
          WHERE ch.measure_id = {measure} AND {chained}
         ORDER BY 14, 2, 1"
    )
}

/// What makes two facts the same fact, for the diff.
type Key = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<i64>,
    u64,
    Option<u64>,
    Option<u64>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn new_key(f: &NewFact) -> Key {
    (
        f.subject_kind.clone(),
        f.subject_ref.clone(),
        f.path.clone(),
        f.line,
        f.value.to_bits(),
        f.numerator.map(f64::to_bits),
        f.denominator.map(f64::to_bits),
        f.severity.clone(),
        f.rule.clone(),
        f.detail.clone(),
        f.dims_json.clone(),
    )
}

fn row_key(f: &FactRow) -> Key {
    (
        f.subject_kind.clone(),
        f.subject_ref.clone(),
        f.path.clone(),
        f.line,
        f.value.to_bits(),
        f.numerator.map(f64::to_bits),
        f.denominator.map(f64::to_bits),
        f.severity.clone(),
        f.rule.clone(),
        f.detail.clone(),
        f.dims_json.clone(),
    )
}

/// Write `facts` for the just-inserted `capture_id` of `producer` on
/// `stream_id`. A complete-scope measure of a finished capture extends its
/// chain from the producer's previous finished capture; anything else is
/// stored as is, held by its own capture only.
pub(crate) fn write(
    conn: &rusqlite::Connection,
    capture_id: i64,
    stream_id: i64,
    producer: &str,
    done: bool,
    facts: &[NewFact],
) -> rusqlite::Result<()> {
    let mut chained: BTreeMap<i64, Vec<&NewFact>> = BTreeMap::new();
    for f in facts {
        if done && is_complete(conn, f.measure_id)? {
            chained.entry(f.measure_id).or_default().push(f);
        } else {
            insert_fact(conn, f, capture_id)?;
        }
    }
    if !done {
        return Ok(());
    }
    let base: Option<i64> = conn
        .prepare_cached(
            "SELECT id FROM metric_capture
              WHERE stream_id = ?1 AND producer = ?2 AND status = 'done' AND id < ?3
              ORDER BY id DESC LIMIT 1",
        )?
        .query_row(params![stream_id, producer, capture_id], |r| r.get(0))
        .optional()?;
    // The base's chains too: a scan that found none of a measure's
    // population restates it as empty.
    let mut links: HashMap<i64, (i64, i64)> = HashMap::new();
    if let Some(base) = base {
        let mut stmt = conn.prepare_cached(
            "SELECT measure_id, from_capture_id, depth FROM fact_chain WHERE capture_id = ?1",
        )?;
        for row in stmt.query_map([base], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))? {
            let (m, from, depth): (i64, i64, i64) = row?;
            links.insert(m, (from, depth));
            chained.entry(m).or_default();
        }
    }
    for (m, new) in chained {
        let link = links.get(&m).copied();
        let had = match (base, link) {
            (Some(b), Some(_)) => held(conn, b, m)?,
            _ => Vec::new(),
        };
        let mut unmatched: HashMap<Key, Vec<i64>> = HashMap::new();
        for row in &had {
            unmatched.entry(row_key(row)).or_default().push(row.id);
        }
        let mut added: Vec<&NewFact> = Vec::new();
        for f in &new {
            if unmatched.get_mut(&new_key(f)).and_then(Vec::pop).is_none() {
                added.push(f);
            }
        }
        let dropped: Vec<i64> = unmatched.into_values().flatten().collect();
        let whole = match link {
            None => true,
            Some((_, depth)) => {
                depth + 1 >= MAX_CHAIN_DEPTH
                    || 2 * (added.len() + dropped.len()) > new.len().max(had.len())
            }
        };
        let (closed, stored, from, depth) = if whole {
            let all: Vec<i64> = had.iter().map(|r| r.id).collect();
            (all, new, capture_id, 0)
        } else {
            let (from, depth) = link.expect("a chain to extend");
            (dropped, added, from, depth + 1)
        };
        if let Some(b) = base {
            let mut close =
                conn.prepare_cached("UPDATE fact SET last_capture_id = ?1 WHERE id = ?2")?;
            for id in closed {
                close.execute(params![b, id])?;
            }
        }
        // A whole capture that stores nothing holds nothing.
        if whole && stored.is_empty() {
            continue;
        }
        for f in stored {
            insert_fact(conn, f, capture_id)?;
        }
        conn.prepare_cached(
            "INSERT INTO fact_chain (capture_id, measure_id, from_capture_id, depth)
             VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![capture_id, m, from, depth])?;
    }
    Ok(())
}

fn is_complete(conn: &rusqlite::Connection, m: i64) -> rusqlite::Result<bool> {
    Ok(conn
        .prepare_cached("SELECT capture_scope = 'complete' FROM measure WHERE id = ?1")?
        .query_row([m], |r| r.get(0))
        .optional()?
        .unwrap_or(false))
}

/// The facts of measure `m` chain capture `capture` holds.
fn held(conn: &rusqlite::Connection, capture: i64, m: i64) -> rusqlite::Result<Vec<FactRow>> {
    conn.prepare_cached(&format!(
        "SELECT {FACT_ROW_COLS} FROM {CHAIN_HELD}
          WHERE ch.capture_id = ?1 AND ch.measure_id = ?2"
    ))?
    .query_map(params![capture, m], fact_row_mapper(conn)?)?
    .collect()
}

/// Before `doomed` captures are deleted: move each fact one of them stored
/// that a surviving capture still holds up to the first such capture, so
/// it outlives the one that stored it. The rest go with their capture.
pub(crate) fn rehome(conn: &rusqlite::Connection, doomed: &[i64]) -> rusqlite::Result<()> {
    let ids = serde_json::to_string(doomed).expect("ids serialize");
    let moves: Vec<(i64, i64)> = conn
        .prepare(
            "SELECT id, home FROM (
               SELECT f.id,
                      (SELECT min(ch.capture_id) FROM fact_chain ch
                         JOIN metric_capture hc ON hc.id = ch.capture_id
                        WHERE ch.measure_id = f.measure_id
                          AND hc.stream_id = bc.stream_id AND hc.producer = bc.producer
                          AND ch.capture_id > f.capture_id
                          AND ch.capture_id <= coalesce(f.last_capture_id, 9223372036854775807)
                          AND ch.capture_id NOT IN (SELECT value FROM json_each(?1))) AS home
                 FROM fact f
                 JOIN metric_capture bc ON bc.id = f.capture_id
                 JOIN fact_chain bch ON bch.capture_id = f.capture_id
                                    AND bch.measure_id = f.measure_id
                WHERE f.capture_id IN (SELECT value FROM json_each(?1))
                  AND (f.last_capture_id IS NULL OR f.last_capture_id > f.capture_id))
             WHERE home IS NOT NULL",
        )?
        .query_map([&ids], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = conn.prepare("UPDATE fact SET capture_id = ?2 WHERE id = ?1")?;
    for (id, home) in moves {
        update.execute(params![id, home])?;
    }
    Ok(())
}

/// For prune's keep rules: the capture that last holds fact `f` — a row
/// with its `capture_id`, `measure_id`, `last_capture_id` and its capture's
/// `stream_id` and `producer`. That's its own capture unless a chain
/// carries it on.
pub(crate) const LAST_HOLDER: &str = "CASE
       WHEN NOT EXISTS (SELECT 1 FROM fact_chain x
                         WHERE x.capture_id = f.capture_id AND x.measure_id = f.measure_id)
         THEN f.capture_id
       WHEN f.last_capture_id IS NOT NULL THEN f.last_capture_id
       ELSE (SELECT max(ch.capture_id) FROM fact_chain ch
               JOIN metric_capture hc ON hc.id = ch.capture_id
              WHERE ch.measure_id = f.measure_id
                AND hc.stream_id = f.stream_id AND hc.producer = f.producer)
     END";
