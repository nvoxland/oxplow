//! Captures that store only what changed: complete-scope chains (V39) and
//! per-path holds (V43).
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
//! A `per-path` code gauge restates each file it rescans, and a rescan
//! mostly repeats the file's last one but for lines (an edit shifts the
//! functions below it: 60% of facts identical, 99% identical but for the
//! line). So a finished delta or full scan holds a file's unchanged facts
//! the same way, per file:
//!
//! - a `fact_path_hold (capture, measure, path)` row marks a capture as
//!   holding that file's facts stored since `from_capture_id` and not closed
//!   before it; its own rows for the file are what's new;
//! - a fact that only moved stays one fact, and `fact_line (fact, capture,
//!   line)` says where it is from that capture on — a holder reads the
//!   newest one at or before it ([`LINE_AT_HOLD`]), else the fact's own.
//!
//! A capture drops a fact by closing it (`last_capture_id`). It stores a
//! measure — or a file — whole when there's nothing to stand on, when more
//! than half of what it restates changed, or after [`MAX_CHAIN_DEPTH`]
//! captures, which bounds how far back a read looks.
//!
//! Every read of facts goes through [`held_facts_sql`] (and `v_fact` and the
//! tree fold through the same rule), so a reader sees each capture's whole
//! set, labelled with that capture, at its lines. Pruning moves a fact a
//! doomed capture stored up to the next capture that holds it ([`rehome`]).

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, OptionalExtension};

use crate::fact_store::{fact_row_mapper, insert_fact, FactRow, NewFact, FACT_ROW_COLS};

/// The most captures a chain (or a file's holds) runs before one is stored
/// whole.
pub const MAX_CHAIN_DEPTH: i64 = 32;

/// The facts chain captures hold: `ch` is the holder's `fact_chain` row,
/// `c` the holder, `f` a fact it holds and `fc` the capture that stored it.
/// The CROSS JOINs pin that order: from the chain rows a read wants, never
/// from every fact (the planner, guessing at `fact_chain`, went fact-first
/// and scanned the stream's later captures for each fact). The bounds are
/// spelled out because with `BETWEEN` here SQLite drops the upper one, and
/// the index is named because inside a larger read (a UNION with an ORDER
/// BY) the planner picked another and scanned the measure's whole range
/// per chain row: a per-path history read went 2 s → 21 s.
const CHAIN_HELD: &str = "fact_chain ch
       CROSS JOIN metric_capture c ON c.id = ch.capture_id
       CROSS JOIN fact f INDEXED BY idx_fact_measure_capture ON f.measure_id = ch.measure_id
                  AND f.capture_id >= ch.from_capture_id AND f.capture_id <= ch.capture_id
                  AND (f.last_capture_id IS NULL OR f.last_capture_id >= ch.capture_id)
       CROSS JOIN metric_capture fc ON fc.id = f.capture_id
                             AND fc.stream_id = c.stream_id AND fc.producer = c.producer
                             AND fc.status = 'done'";

/// The facts path holds hold, the same way: `ph` is the holder's
/// `fact_path_hold` row, `c` the holder, `f` a fact of that file it holds.
pub(crate) const PATH_HELD: &str = "fact_path_hold ph
       CROSS JOIN metric_capture c ON c.id = ph.capture_id
       CROSS JOIN fact f INDEXED BY idx_fact_measure_path
                  ON f.measure_id = ph.measure_id AND f.path_id = ph.path_id
                  AND f.capture_id >= ph.from_capture_id AND f.capture_id <= ph.capture_id
                  AND (f.last_capture_id IS NULL OR f.last_capture_id >= ph.capture_id)
       CROSS JOIN metric_capture fc ON fc.id = f.capture_id
                             AND fc.stream_id = c.stream_id AND fc.producer = c.producer
                             AND fc.status = 'done'";

/// A held fact `f`'s line as its holder `ph` sees it: the newest line move
/// at or before the holder, else the fact's own.
pub(crate) const LINE_AT_HOLD: &str = "CAST(coalesce((SELECT fl.line FROM fact_line fl
                  WHERE fl.fact_id = f.id AND fl.capture_id <= ph.capture_id
                  ORDER BY fl.capture_id DESC LIMIT 1), f.line) AS INTEGER)";

/// [`FACT_ROW_COLS`] for a fact a path hold holds, at the holder's line.
pub(crate) fn path_held_cols() -> String {
    FACT_ROW_COLS.replace("f.line", LINE_AT_HOLD)
}

/// A capture's own rows of `f`'s file and measure aren't its whole set
/// when it holds that file (its own rows come through the hold).
pub(crate) const NOT_PATH_HELD: &str = "NOT EXISTS (SELECT 1 FROM fact_path_hold y
                 WHERE y.capture_id = c.id AND y.measure_id = f.measure_id
                   AND y.path_id = f.path_id)";

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
/// capture columns and the holder's line.
pub(crate) fn held_facts_sql(measure: &str, holders: Holders) -> String {
    let (own, chained, pathed) = match holders {
        Holders::All => ("1".to_string(), "1".to_string(), "1".to_string()),
        Holders::Stream(p) => (
            format!("c.stream_id = {p}"),
            format!("c.stream_id = {p}"),
            format!("c.stream_id = {p}"),
        ),
        Holders::Ids(p) => (
            format!("c.id IN (SELECT value FROM json_each({p}))"),
            format!("ch.capture_id IN (SELECT value FROM json_each({p}))"),
            format!("ph.capture_id IN (SELECT value FROM json_each({p}))"),
        ),
    };
    let path_cols = path_held_cols();
    format!(
        "SELECT {FACT_ROW_COLS} FROM fact f JOIN metric_capture c ON c.id = f.capture_id
          WHERE f.measure_id = {measure} AND {own}
            AND NOT EXISTS (SELECT 1 FROM fact_chain x
                             WHERE x.capture_id = c.id AND x.measure_id = f.measure_id)
            AND {NOT_PATH_HELD}
         UNION ALL
         SELECT {FACT_ROW_COLS} FROM {CHAIN_HELD}
          WHERE ch.measure_id = {measure} AND {chained}
         UNION ALL
         SELECT {path_cols} FROM {PATH_HELD}
          WHERE ph.measure_id = {measure} AND {pathed}
         ORDER BY 14, 2, 1"
    )
}

/// What makes two facts the same fact, for the diff — the line left out
/// (`None`) where a move isn't a change.
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

fn new_key(f: &NewFact, with_line: bool) -> Key {
    (
        f.subject_kind.clone(),
        f.subject_ref.clone(),
        f.path.clone(),
        if with_line { f.line } else { None },
        f.value.to_bits(),
        f.numerator.map(f64::to_bits),
        f.denominator.map(f64::to_bits),
        f.severity.clone(),
        f.rule.clone(),
        f.detail.clone(),
        f.dims_json.clone(),
    )
}

fn row_key(f: &FactRow, with_line: bool) -> Key {
    (
        f.subject_kind.clone(),
        f.subject_ref.clone(),
        f.path.clone(),
        if with_line { f.line } else { None },
        f.value.to_bits(),
        f.numerator.map(f64::to_bits),
        f.denominator.map(f64::to_bits),
        f.severity.clone(),
        f.rule.clone(),
        f.detail.clone(),
        f.dims_json.clone(),
    )
}

/// Write `facts` for the just-inserted `capture_id`. A finished capture
/// extends its producer's chain for each complete-scope measure, and holds
/// each file of a per-path measure it scanned (a delta or full scan);
/// anything else is stored as is, held by its own capture only.
pub(crate) fn write(
    conn: &rusqlite::Connection,
    capture_id: i64,
    capture: &crate::fact_store::NewMetricCapture,
    facts: &[NewFact],
) -> rusqlite::Result<()> {
    let done = capture.status == "done";
    let scanned = matches!(capture.scan_kind.as_str(), "delta" | "full");
    let mut chained: BTreeMap<i64, Vec<&NewFact>> = BTreeMap::new();
    let mut pathed: BTreeMap<(i64, String), Vec<&NewFact>> = BTreeMap::new();
    for f in facts {
        match (done, scope(conn, f.measure_id)?.as_deref(), &f.path) {
            (true, Some("complete"), _) => chained.entry(f.measure_id).or_default().push(f),
            (true, Some("per-path"), Some(path)) if scanned => pathed
                .entry((f.measure_id, path.clone()))
                .or_default()
                .push(f),
            _ => {
                insert_fact(conn, f, capture_id)?;
            }
        }
    }
    if !done {
        return Ok(());
    }
    write_chains(
        conn,
        capture_id,
        capture.stream_id,
        &capture.producer,
        chained,
    )?;
    for ((m, path), new) in pathed {
        write_path(
            conn,
            capture_id,
            capture.stream_id,
            &capture.producer,
            m,
            &path,
            new,
        )?;
    }
    Ok(())
}

fn write_chains(
    conn: &rusqlite::Connection,
    capture_id: i64,
    stream_id: i64,
    producer: &str,
    mut chained: BTreeMap<i64, Vec<&NewFact>>,
) -> rusqlite::Result<()> {
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
            (Some(b), Some(_)) => chain_held(conn, b, m)?,
            _ => Vec::new(),
        };
        let mut unmatched: HashMap<Key, Vec<i64>> = HashMap::new();
        for row in &had {
            unmatched
                .entry(row_key(row, true))
                .or_default()
                .push(row.id);
        }
        let mut added: Vec<&NewFact> = Vec::new();
        for f in &new {
            if unmatched
                .get_mut(&new_key(f, true))
                .and_then(Vec::pop)
                .is_none()
            {
                added.push(f);
            }
        }
        let dropped: Vec<i64> = unmatched.into_values().flatten().collect();
        // Stored whole when there's nothing to stand on or nothing left.
        let whole = match link {
            None => true,
            Some(_) if had.is_empty() || new.is_empty() => true,
            Some((_, depth)) => {
                depth + 1 >= MAX_CHAIN_DEPTH
                    // A changed fact is one added and one dropped: more than
                // half changed is more adds and drops than facts.
                || added.len() + dropped.len() > new.len().max(had.len())
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
            close(conn, &closed, b)?;
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

/// Hold file `path` of measure `m` for `capture_id`: diff what it scanned
/// against what the file's last hold holds, matching on everything but the
/// line — a match that moved records the move.
fn write_path(
    conn: &rusqlite::Connection,
    capture_id: i64,
    stream_id: i64,
    producer: &str,
    m: i64,
    path: &str,
    new: Vec<&NewFact>,
) -> rusqlite::Result<()> {
    let path_id = path_id(conn, path)?;
    let base: Option<(i64, i64, i64)> = conn
        .prepare_cached(
            "SELECT h.capture_id, h.from_capture_id, h.depth FROM fact_path_hold h
               JOIN metric_capture hc ON hc.id = h.capture_id
              WHERE h.measure_id = ?1 AND h.path_id = ?2 AND h.capture_id < ?5
                AND hc.stream_id = ?3 AND hc.producer = ?4
              ORDER BY h.capture_id DESC LIMIT 1",
        )?
        .query_row(params![m, path_id, stream_id, producer, capture_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    let had = match base {
        Some((b, _, _)) => path_held(conn, b, m, path_id)?,
        None => Vec::new(),
    };
    // Match each scanned fact to a held one, the same line first.
    let mut unmatched: HashMap<Key, Vec<(i64, Option<i64>)>> = HashMap::new();
    for row in &had {
        unmatched
            .entry(row_key(row, false))
            .or_default()
            .push((row.id, row.line));
    }
    let mut added: Vec<&NewFact> = Vec::new();
    let mut moved: Vec<(i64, Option<i64>)> = Vec::new();
    for f in &new {
        let Some(held) = unmatched.get_mut(&new_key(f, false)) else {
            added.push(f);
            continue;
        };
        let at = held
            .iter()
            .position(|(_, line)| *line == f.line)
            .or(if held.is_empty() { None } else { Some(0) });
        match at {
            Some(i) => {
                let (id, line) = held.remove(i);
                if line != f.line {
                    moved.push((id, f.line));
                }
            }
            None => added.push(f),
        }
    }
    let dropped: Vec<i64> = unmatched
        .into_values()
        .flatten()
        .map(|(id, _)| id)
        .collect();
    let whole = match base {
        None => true,
        Some(_) if had.is_empty() => true,
        Some((_, _, depth)) => {
            depth + 1 >= MAX_CHAIN_DEPTH
                // A changed fact is one added and one dropped: more than
                // half changed is more adds and drops than facts.
                || added.len() + dropped.len() > new.len().max(had.len())
        }
    };
    // Nothing between the base and this capture holds the file, so a fact
    // this capture drops was last held just before it.
    let (closed, stored, from, depth) = if whole {
        let all: Vec<i64> = had.iter().map(|r| r.id).collect();
        (all, new, capture_id, 0)
    } else {
        let (_, from, depth) = base.expect("a hold to extend");
        let mut line = conn.prepare_cached(
            "INSERT INTO fact_line (fact_id, capture_id, line) VALUES (?1, ?2, ?3)",
        )?;
        for (id, at) in &moved {
            line.execute(params![id, capture_id, at])?;
        }
        (dropped, added, from, depth + 1)
    };
    close(conn, &closed, capture_id - 1)?;
    for f in stored {
        insert_fact(conn, f, capture_id)?;
    }
    conn.prepare_cached(
        "INSERT INTO fact_path_hold (capture_id, measure_id, path_id, from_capture_id, depth)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![capture_id, m, path_id, from, depth])?;
    Ok(())
}

/// Close `ids`: the last capture that holds them is `last`.
fn close(conn: &rusqlite::Connection, ids: &[i64], last: i64) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare_cached("UPDATE fact SET last_capture_id = ?1 WHERE id = ?2")?;
    for id in ids {
        stmt.execute(params![last, id])?;
    }
    Ok(())
}

fn scope(conn: &rusqlite::Connection, m: i64) -> rusqlite::Result<Option<String>> {
    conn.prepare_cached("SELECT capture_scope FROM measure WHERE id = ?1")?
        .query_row([m], |r| r.get(0))
        .optional()
}

/// `path`'s id in `fact_path`, added when new.
fn path_id(conn: &rusqlite::Connection, path: &str) -> rusqlite::Result<i64> {
    let found = conn
        .prepare_cached("SELECT id FROM fact_path WHERE path = ?1")?
        .query_row([path], |r| r.get(0))
        .optional()?;
    if let Some(id) = found {
        return Ok(id);
    }
    conn.prepare_cached("INSERT INTO fact_path (path) VALUES (?1)")?
        .execute([path])?;
    Ok(conn.last_insert_rowid())
}

/// The facts of measure `m` chain capture `capture` holds.
fn chain_held(conn: &rusqlite::Connection, capture: i64, m: i64) -> rusqlite::Result<Vec<FactRow>> {
    conn.prepare_cached(&format!(
        "SELECT {FACT_ROW_COLS} FROM {CHAIN_HELD}
          WHERE ch.capture_id = ?1 AND ch.measure_id = ?2"
    ))?
    .query_map(params![capture, m], fact_row_mapper(conn)?)?
    .collect()
}

/// The facts of measure `m`'s file `path_id` capture `capture` holds, at
/// its lines.
fn path_held(
    conn: &rusqlite::Connection,
    capture: i64,
    m: i64,
    path_id: i64,
) -> rusqlite::Result<Vec<FactRow>> {
    conn.prepare_cached(&format!(
        "SELECT {} FROM {PATH_HELD}
          WHERE ph.capture_id = ?1 AND ph.measure_id = ?2 AND ph.path_id = ?3",
        path_held_cols()
    ))?
    .query_map(params![capture, m, path_id], fact_row_mapper(conn)?)?
    .collect()
}

/// Before `doomed` captures are deleted: move each fact one of them stored
/// that a surviving capture still holds up to the first such capture, so
/// it outlives the one that stored it. A moved fact takes the line it has
/// there, and the line moves before it go. The rest go with their capture.
pub(crate) fn rehome(conn: &rusqlite::Connection, doomed: &[i64]) -> rusqlite::Result<()> {
    let ids = serde_json::to_string(doomed).expect("ids serialize");
    let moves: Vec<(i64, i64)> = conn
        .prepare(
            "SELECT id, home FROM (
               SELECT f.id,
                      CASE WHEN EXISTS (SELECT 1 FROM fact_chain bch
                                         WHERE bch.capture_id = f.capture_id
                                           AND bch.measure_id = f.measure_id)
                      THEN (SELECT min(ch.capture_id) FROM fact_chain ch
                              JOIN metric_capture hc ON hc.id = ch.capture_id
                             WHERE ch.measure_id = f.measure_id
                               AND hc.stream_id = bc.stream_id AND hc.producer = bc.producer
                               AND ch.capture_id > f.capture_id
                               AND ch.from_capture_id <= f.capture_id
                               AND ch.capture_id <= coalesce(f.last_capture_id, 9223372036854775807)
                               AND ch.capture_id NOT IN (SELECT value FROM json_each(?1)))
                      WHEN EXISTS (SELECT 1 FROM fact_path_hold bph
                                    WHERE bph.capture_id = f.capture_id
                                      AND bph.measure_id = f.measure_id
                                      AND bph.path_id = f.path_id)
                      THEN (SELECT min(h.capture_id) FROM fact_path_hold h
                              JOIN metric_capture hc ON hc.id = h.capture_id
                             WHERE h.measure_id = f.measure_id AND h.path_id = f.path_id
                               AND hc.stream_id = bc.stream_id AND hc.producer = bc.producer
                               AND h.capture_id > f.capture_id
                               AND h.from_capture_id <= f.capture_id
                               AND h.capture_id <= coalesce(f.last_capture_id, 9223372036854775807)
                               AND h.capture_id NOT IN (SELECT value FROM json_each(?1)))
                      END AS home
                 FROM fact f
                 JOIN metric_capture bc ON bc.id = f.capture_id
                WHERE f.capture_id IN (SELECT value FROM json_each(?1))
                  AND (f.last_capture_id IS NULL OR f.last_capture_id > f.capture_id))
             WHERE home IS NOT NULL",
        )?
        .query_map([&ids], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut update = conn.prepare(
        "UPDATE fact SET capture_id = ?2,
                line = coalesce((SELECT fl.line FROM fact_line fl
                                  WHERE fl.fact_id = ?1 AND fl.capture_id <= ?2
                                  ORDER BY fl.capture_id DESC LIMIT 1), line)
          WHERE id = ?1",
    )?;
    let mut drop_moves =
        conn.prepare("DELETE FROM fact_line WHERE fact_id = ?1 AND capture_id <= ?2")?;
    for (id, home) in moves {
        update.execute(params![id, home])?;
        drop_moves.execute(params![id, home])?;
    }
    Ok(())
}

/// For prune's keep rules: the capture that last holds fact `f` — a row
/// with its `capture_id`, `measure_id`, `path_id`, `last_capture_id` and
/// its capture's `stream_id` and `producer`. That's its own capture unless
/// a chain or a file's holds carry it on.
pub(crate) const LAST_HOLDER: &str = "CASE
       WHEN EXISTS (SELECT 1 FROM fact_chain x
                     WHERE x.capture_id = f.capture_id AND x.measure_id = f.measure_id)
         THEN coalesce(f.last_capture_id,
                (SELECT max(ch.capture_id) FROM fact_chain ch
                   JOIN metric_capture hc ON hc.id = ch.capture_id
                  WHERE ch.measure_id = f.measure_id AND ch.from_capture_id <= f.capture_id
                    AND hc.stream_id = f.stream_id AND hc.producer = f.producer))
       WHEN EXISTS (SELECT 1 FROM fact_path_hold y
                     WHERE y.capture_id = f.capture_id AND y.measure_id = f.measure_id
                       AND y.path_id = f.path_id)
         THEN coalesce(f.last_capture_id,
                (SELECT max(h.capture_id) FROM fact_path_hold h
                   JOIN metric_capture hc ON hc.id = h.capture_id
                  WHERE h.measure_id = f.measure_id AND h.path_id = f.path_id
                    AND h.from_capture_id <= f.capture_id
                    AND hc.stream_id = f.stream_id AND hc.producer = f.producer))
       ELSE f.capture_id
     END";
