//! A generated lcov report: many files, each with function (`FN`/`FNDA`),
//! branch (`BRDA`) and line (`DA`) records and their totals, the way
//! `cargo llvm-cov --lcov` writes them, with long mangled symbol names.

use std::fmt::Write;

use super::Rng;

/// The size the containment measurements use: about 5 MB.
pub const DEFAULT_LINES: usize = 100_000;

const SEED: u64 = 0x1c0f;
/// What the fewest-line record needs: `TN`, `SF`, `FNF`, `FNH`, `BRF`,
/// `BRH`, `LF`, `LH`, `end_of_record`.
const RECORD_OVERHEAD: usize = 9;

const CRATES: &[&str] = &["app", "config", "db", "domain", "script", "rpc", "git"];
const AREAS: &[&str] = &[
    "collectors",
    "commands",
    "metrics",
    "stores",
    "runtime",
    "hooks",
];
const WORDS: &[&str] = &[
    "collect",
    "resolve",
    "snapshot",
    "render",
    "dispatch",
    "reconcile",
    "decode",
    "persist",
    "observe",
    "project",
];
const GENERICS: &[&str] = &[
    "alloc..vec..Vec$LT$serde_json..value..Value$GT$",
    "core..result..Result$LT$$LP$$RP$$C$oxplow_domain..error..Error$GT$",
    "std..collections..hash..map..HashMap$LT$alloc..string..String$C$alloc..string..String$GT$",
    "core..option..Option$LT$alloc..boxed..Box$LT$dyn$u20$core..any..Any$GT$$GT$",
];

/// An lcov report of exactly `lines` lines (at least 9), the records of
/// several files, each complete. At [`DEFAULT_LINES`] it is about 5 MB.
pub fn report(lines: usize) -> String {
    let lines = lines.max(RECORD_OVERHEAD + 1);
    let mut rng = Rng::new(SEED);
    let mut out = String::with_capacity(lines * 50);
    let mut remaining = lines;
    let mut file = 0;
    while remaining > 0 {
        let mut fns = rng.range(40, 120);
        let mut branches = rng.range(30, 80);
        let mut da = rng.range(100, 250);
        let mut size = RECORD_OVERHEAD + 2 * fns + branches + da;
        // Never leave a remainder too small to be a record of its own.
        if size > remaining || remaining - size < RECORD_OVERHEAD + 1 {
            size = remaining;
            fns = fns.min((size - RECORD_OVERHEAD - 1) / 4);
            branches = branches.min((size - RECORD_OVERHEAD - 1 - 2 * fns) / 2);
            da = size - RECORD_OVERHEAD - 2 * fns - branches;
        }
        record(&mut out, &mut rng, file, fns, branches, da);
        remaining -= size;
        file += 1;
    }
    out
}

fn record(out: &mut String, rng: &mut Rng, file: usize, fns: usize, branches: usize, da: usize) {
    let krate = CRATES[file % CRATES.len()];
    let area = AREAS[(file / CRATES.len()) % AREAS.len()];
    let _ = writeln!(out, "TN:");
    let _ = writeln!(
        out,
        "SF:/home/runner/work/oxplow/oxplow/crates/oxplow-{krate}/src/{area}/unit_{file:04}.rs"
    );
    let mut line = 1;
    let mut names = Vec::with_capacity(fns);
    for i in 0..fns {
        line += rng.range(2, 9);
        let (word, word2) = (rng.pick(WORDS), rng.pick(WORDS));
        // Half the symbols are generic instances, the long ones.
        let generic = if rng.chance(50) {
            format!("$LT${}$GT$", rng.pick(GENERICS))
        } else {
            String::new()
        };
        let name = format!(
            "_ZN12oxplow_{krate}{n}{area}{m}{word}{k}{word2}$u7b$$u7b$closure$u7d$$u7d${generic}17h{hash:016x}E",
            n = area.len(),
            m = word.len(),
            k = word2.len(),
            hash = rng.next_u64(),
        );
        let _ = writeln!(out, "FN:{line},{name}");
        names.push((name, i));
    }
    let mut hit_fns = 0;
    for (name, _) in &names {
        let count = if rng.chance(80) {
            rng.range(1, 5000)
        } else {
            0
        };
        hit_fns += usize::from(count > 0);
        let _ = writeln!(out, "FNDA:{count},{name}");
    }
    let _ = writeln!(out, "FNF:{fns}");
    let _ = writeln!(out, "FNH:{hit_fns}");
    let mut hit_branches = 0;
    let mut line = 1;
    for _ in 0..branches {
        line += rng.range(0, 4);
        let (block, branch) = (rng.range(0, 3), rng.range(0, 2));
        let taken = if rng.chance(70) {
            rng.range(1, 900).to_string()
        } else {
            "-".into()
        };
        hit_branches += usize::from(taken != "-");
        let _ = writeln!(out, "BRDA:{},{block},{branch},{taken}", line + 1);
    }
    let _ = writeln!(out, "BRF:{branches}");
    let _ = writeln!(out, "BRH:{hit_branches}");
    let mut hit_lines = 0;
    let mut line = 0;
    for _ in 0..da {
        line += rng.range(1, 3);
        let count = if rng.chance(85) {
            rng.range(1, 20000)
        } else {
            0
        };
        hit_lines += usize::from(count > 0);
        let _ = writeln!(out, "DA:{line},{count}");
    }
    let _ = writeln!(out, "LF:{da}");
    let _ = writeln!(out, "LH:{hit_lines}");
    let _ = writeln!(out, "end_of_record");
}
