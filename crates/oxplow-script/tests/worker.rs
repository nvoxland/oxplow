//! The script worker contains what it runs: a script past its budget,
//! over its memory or cancelled is stopped and reaped, never left
//! running; runs are bounded in number; a run's calls out are answered by
//! its host with the time spent answering left out of its budget.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxplow_domain::run::Budget;
use oxplow_script::workers::{Cancel, Limits, ScriptRun, ScriptWorkers};
use oxplow_script::CollectError;
use serde_json::{json, Value};

fn workers(limits: Limits) -> ScriptWorkers {
    ScriptWorkers::new(env!("CARGO_BIN_EXE_oxplow-script-worker").into(), limits)
}

fn limits() -> Limits {
    Limits {
        max_concurrent: 4,
        memory_bytes: 256 * 1024 * 1024,
    }
}

fn budget(ms: u64) -> Budget {
    Budget::with_timeout(Duration::from_millis(ms)).with_ceiling(Duration::from_secs(30))
}

/// No process with `pid` is left (it was reaped, not just signalled).
fn gone(pid: u32) -> bool {
    // SAFETY: signal 0 only checks the pid exists.
    unsafe { libc::kill(pid as i32, 0) == -1 }
}

const SPIN: &str = "def transform(x):\n    n = 0\n    for i in range(2000000000):\n        for j in range(2000000000):\n            n += 1\n    return n\n";

#[test]
fn a_run_answers_its_value_and_what_it_cost() {
    let w = workers(limits());
    let ran = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(
                "def transform(x):\n    return {'n': x['n'] + 1}\n",
                json!({ "n": 1 }),
            ),
            None,
        )
        .unwrap();
    assert_eq!(ran.value, json!({ "n": 2 }));
    assert!(ran.peak_bytes > 0);
    let jaq = w
        .run(
            &budget(5_000),
            ScriptRun::jaq(".n * 2", json!({ "n": 21 })),
            None,
        )
        .unwrap();
    assert_eq!(jaq.value, json!(42));
    assert_eq!(w.live(), 0);
}

#[test]
fn a_script_error_is_the_scripts() {
    let w = workers(limits());
    let err = w
        .run(
            &budget(5_000),
            ScriptRun::starlark("def transform(x):\n    return 1 // 0\n", json!({})),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, CollectError::Runtime(_)), "{err:?}");
}

/// A Starlark loop and a jq one (which no in-process limit can stop) both
/// end at their budget, their worker stopped and reaped.
#[test]
fn a_runaway_script_is_stopped_and_reaped_at_its_budget() {
    let w = workers(limits());
    for run in [
        ScriptRun::starlark(SPIN, json!({})),
        ScriptRun::jaq("reduce range(1000000000000) as $i (0; . + 1)", json!(null)),
    ] {
        let started = Instant::now();
        let err = w.run(&budget(200), run, None).unwrap_err();
        assert!(matches!(err, CollectError::Timeout), "{err:?}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(w.live(), 0);
        let pid = *w.recent_pids().last().unwrap();
        assert!(gone(pid), "worker {pid} is still there");
    }
}

/// Repeated overruns leave nothing behind, and the host still runs scripts.
#[test]
fn repeated_overruns_leave_no_workers() {
    let w = workers(limits());
    for _ in 0..20 {
        let err = w
            .run(&budget(20), ScriptRun::starlark(SPIN, json!({})), None)
            .unwrap_err();
        assert!(matches!(err, CollectError::Timeout), "{err:?}");
    }
    assert_eq!(w.live(), 0);
    for pid in w.recent_pids() {
        assert!(gone(pid), "worker {pid} is still there");
    }
    let ran = w
        .run(&budget(5_000), ScriptRun::jaq(".", json!(1)), None)
        .unwrap();
    assert_eq!(ran.value, json!(1));
}

#[test]
fn a_run_past_its_memory_is_stopped() {
    let w = workers(Limits {
        max_concurrent: 1,
        memory_bytes: 64 * 1024 * 1024,
    });
    let bomb = "def transform(x):\n    xs = []\n    for i in range(1000000000):\n        xs.append('x' * 1000000)\n    return len(xs)\n";
    let err = w
        .run(&budget(30_000), ScriptRun::starlark(bomb, json!({})), None)
        .unwrap_err();
    assert!(matches!(err, CollectError::MemoryExceeded(_)), "{err:?}");
    assert_eq!(w.live(), 0);
}

#[test]
fn a_cancelled_run_is_stopped() {
    let w = Arc::new(workers(limits()));
    let cancel = Cancel::default();
    let stop = cancel.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        stop.cancel();
    });
    let started = Instant::now();
    let err = w
        .run(
            &budget(30_000),
            ScriptRun::starlark(SPIN, json!({})),
            Some(&cancel),
        )
        .unwrap_err();
    assert!(matches!(err, CollectError::Cancelled), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(w.live(), 0);
}

/// One slot: a second run waits for the first, so no more than one
/// worker is ever alive.
#[test]
fn runs_are_bounded_in_number() {
    let w = Arc::new(workers(Limits {
        max_concurrent: 1,
        memory_bytes: 256 * 1024 * 1024,
    }));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (w, peak, done) = (w.clone(), peak.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::SeqCst) {
                peak.fetch_max(w.live(), std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    let runs: Vec<_> = (0..3)
        .map(|_| {
            let w = w.clone();
            std::thread::spawn(move || {
                w.run(&budget(150), ScriptRun::starlark(SPIN, json!({})), None)
            })
        })
        .collect();
    for r in runs {
        assert!(matches!(r.join().unwrap(), Err(CollectError::Timeout)));
    }
    done.store(true, std::sync::atomic::Ordering::SeqCst);
    watcher.join().unwrap();
    assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 1);
}

/// A command script's `scope` calls are answered on this thread; the time
/// answering doesn't count against the budget.
#[test]
fn scope_calls_are_answered_by_the_host_outside_the_budget() {
    let w = workers(limits());
    let mut asked = Vec::new();
    let mut serve = |id: &str, args: Value| {
        asked.push(format!("{id} {args}"));
        std::thread::sleep(Duration::from_millis(300));
        Ok(json!([{ "n": asked.len() }]))
    };
    let script = "def transform(x):\n    return {'rows': scope('sql.read', {'sql': 'SELECT 1'}), 'x': x['uri']}\n";
    let ran = w
        .run(
            &budget(200),
            ScriptRun::starlark(script, json!({ "uri": "oxplow:oxplow/effort?id=eff1" }))
                .with_scope(&mut serve),
            None,
        )
        .unwrap();
    assert_eq!(
        ran.value,
        json!({ "rows": [{ "n": 1 }], "x": "oxplow:oxplow/effort?id=eff1" })
    );
    assert_eq!(asked, [r#"sql.read {"sql":"SELECT 1"}"#]);
    // A refusal fails the script with the host's reason.
    let mut refuse = |id: &str, _: Value| Err(format!("`{id}` isn't in the command's needs"));
    let err = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(script, json!({ "uri": "x" })).with_scope(&mut refuse),
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("`sql.read` isn't in the command's needs"),
        "{err}"
    );
}

struct Oracle;

impl oxplow_script::AiOracle for Oracle {
    fn classify(&self, role: &str, text: &str, labels: &[String]) -> Result<Value, String> {
        Ok(json!({ "label": labels[0], "text": text, "role": role }))
    }
    fn score(&self, _: &str, _: &str, _: &[String]) -> Result<Value, String> {
        Err("no scores today".into())
    }
    fn summarize(&self, role: &str, text: &str, _: Option<&str>) -> Result<String, String> {
        Ok(format!("short {text} ({role})"))
    }
    fn extract(&self, _: &str, _: &str, _: &str, schema: &Value) -> Result<Value, String> {
        Ok(json!({ "schema": schema }))
    }
}

#[test]
fn model_calls_are_answered_by_the_hosts_oracle() {
    let w = workers(limits());
    let script = "def transform(x):\n    return {'c': ai_classify('hi', ['a', 'b'], role = 'sort'), 's': ai_summarize('long')}\n";
    let ran = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(script, json!({})).with_ai(Arc::new(Oracle)),
            None,
        )
        .unwrap();
    assert_eq!(
        ran.value,
        json!({ "c": { "label": "a", "text": "hi", "role": "sort" }, "s": "short long (summarize)" })
    );
    let err = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(
                "def transform(x):\n    return ai_score('x', ['l'])\n",
                json!({}),
            )
            .with_ai(Arc::new(Oracle)),
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("no scores today"), "{err}");
    // A script with no model can't reach one.
    let err = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(
                "def transform(x):\n    return ai_summarize('x')\n",
                json!({}),
            ),
            None,
        )
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("`ai_summarize()` needs `ai.compute` in this script's `needs`"),
        "{err}"
    );
}

#[test]
fn a_fact_collector_reads_the_tree_it_was_shipped() {
    let w = workers(limits());
    let files: HashMap<String, String> = [
        ("src/a.rs".to_string(), "fn a() {}\n".to_string()),
        ("docs/b.md".to_string(), "# b\n".to_string()),
    ]
    .into_iter()
    .collect();
    let script = "def transform(x):\n    return [f['path'] for f in files('src/**')]\n";
    let ran = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(script, json!({})).with_tree(Arc::new(files)),
            None,
        )
        .unwrap();
    assert_eq!(ran.value, json!(["src/a.rs"]));
}

/// One run may read the tree, ask a model and call a scope: the hosts
/// are a set, each answered by what the caller gave.
#[test]
fn one_run_reads_the_tree_asks_a_model_and_calls_a_scope() {
    let w = workers(limits());
    let files: HashMap<String, String> = [("src/a.rs".to_string(), "fn a() {}\n".to_string())]
        .into_iter()
        .collect();
    let mut serve = |id: &str, _: Value| Ok(json!([{ "scope": id }]));
    let script = "def transform(x):\n    return {\n        'files': [f['path'] for f in files('**')],\n        'label': ai_classify('hi', ['a'])['label'],\n        'rows': scope('sql.read', {'sql': 'SELECT 1'}),\n    }\n";
    let ran = w
        .run(
            &budget(5_000),
            ScriptRun::starlark(script, json!({}))
                .with_tree(Arc::new(files))
                .with_ai(Arc::new(Oracle))
                .with_scope(&mut serve),
            None,
        )
        .unwrap();
    assert_eq!(
        ran.value,
        json!({ "files": ["src/a.rs"], "label": "a", "rows": [{ "scope": "sql.read" }] })
    );
}

/// A run given no tree can't read one: `files()` says which need it
/// wants rather than answering an empty tree.
#[test]
fn a_run_without_the_tree_is_told_its_need() {
    let w = workers(limits());
    for (call, builtin, need) in [
        ("files('**')", "files", "tree.read"),
        ("source_files()", "source_files", "tree.read"),
        ("duplicate_blocks(3)", "duplicate_blocks", "tree.read"),
        (
            "scope('sql.read', {'sql': 'SELECT 1'})",
            "scope",
            "sql.read",
        ),
    ] {
        let err = w
            .run(
                &budget(5_000),
                ScriptRun::starlark(
                    &format!("def transform(x):\n    return {call}\n"),
                    json!({}),
                )
                .with_ai(Arc::new(Oracle)),
                None,
            )
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(&format!(
                "`{builtin}()` needs `{need}` in this script's `needs`"
            )),
            "{err}"
        );
    }
}

/// A missing worker binary is an error the caller sees, not a hang.
#[test]
fn a_missing_worker_is_reported() {
    let w = ScriptWorkers::new("/nonexistent/oxplow-script-worker".into(), limits());
    let err = w
        .run(&budget(5_000), ScriptRun::jaq(".", json!(1)), None)
        .unwrap_err();
    assert!(matches!(err, CollectError::WorkerDied(_)), "{err:?}");
    assert_eq!(w.live(), 0);
}

/// A model that takes `delay` to answer, counting what it's asked.
#[derive(Default)]
struct Slow {
    delay: Duration,
    asked: std::sync::Mutex<Vec<String>>,
}

impl oxplow_script::AiOracle for Slow {
    fn classify(&self, _: &str, text: &str, labels: &[String]) -> Result<Value, String> {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(text.to_string());
        std::thread::sleep(self.delay);
        Ok(json!({ "label": labels[0] }))
    }
    fn score(&self, _: &str, _: &str, _: &[String]) -> Result<Value, String> {
        Err("no".into())
    }
    fn summarize(&self, _: &str, _: &str, _: Option<&str>) -> Result<String, String> {
        Err("no".into())
    }
    fn extract(&self, _: &str, _: &str, _: &str, _: &Value) -> Result<Value, String> {
        Err("no".into())
    }
}

/// The time a model takes to answer is left out of the budget.
#[test]
fn model_time_is_left_out_of_the_budget() {
    let w = workers(limits());
    let oracle = Arc::new(Slow {
        delay: Duration::from_millis(300),
        ..Slow::default()
    });
    let ran = w
        .run(
            &budget(150),
            ScriptRun::starlark(
                "def transform(x):\n    return ai_classify('t', ['a'])\n",
                json!({}),
            )
            .with_ai(oracle),
            None,
        )
        .unwrap();
    assert_eq!(ran.value["label"], "a");
}

/// …but not out of the ceiling: a per-row loop of model calls stops at
/// the ceiling, and the stopped worker asks nothing more.
#[test]
fn a_run_past_its_ceiling_stops_asking_the_model() {
    let w = workers(limits());
    let oracle = Arc::new(Slow {
        delay: Duration::from_millis(50),
        ..Slow::default()
    });
    let budget =
        Budget::with_timeout(Duration::from_secs(5)).with_ceiling(Duration::from_millis(300));
    let script = "def transform(x):\n    for i in range(100):\n        ai_classify(str(i), ['a'])\n    return {}\n";
    let started = Instant::now();
    let err = w
        .run(
            &budget,
            ScriptRun::starlark(script, json!({})).with_ai(oracle.clone()),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, CollectError::Timeout), "{err:?}");
    assert!(started.elapsed() < Duration::from_secs(2));
    let at_timeout = oracle.asked.lock().unwrap().len();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(oracle.asked.lock().unwrap().len(), at_timeout);
    assert!(at_timeout < 100);
}

fn rust_tree() -> Arc<HashMap<String, String>> {
    Arc::new(HashMap::from([
        (
            "a.rs".to_string(),
            "// TODO: one\nfn a(x: i32) -> i32 {\n    if x > 1 { x } else { 0 }\n}\n".to_string(),
        ),
        (
            "b.rs".to_string(),
            "fn b() {}\nfn c() { let _ = Some(1).unwrap(); }\n".to_string(),
        ),
        ("d.rs".to_string(), "// FIXME: two\nfn d() {}\n".to_string()),
    ]))
}

/// Every source file's `code_metrics`, `markers` and `ast_query`, summed up
/// differently by each script of a sweep.
fn sweep_script(reduce: &str) -> String {
    format!(
        "def transform(x):\n    out = []\n    for f in source_files():\n        fns = code_metrics(f['text'], f['language'], f['path'])\n        marks = markers(f['text'], f['language'])\n        calls = ast_query(f['text'], f['language'], '(call_expression) @c')\n        out.append({{'path': f['path'], 'fns': {reduce}, 'marks': len(marks), 'calls': len(calls)}})\n    return out\n"
    )
}

/// A file is parsed once however many scripts of a sweep read it: the
/// workers are one process per run, so the host keeps what they parsed,
/// keyed by the builtin, the language and the content's hash.
#[test]
fn a_file_is_parsed_once_across_a_sweeps_scripts() {
    let w = workers(limits());
    let tree = rust_tree();
    let run = |script: &str| {
        w.run(
            &budget(10_000),
            ScriptRun::starlark(script, json!({})).with_tree(tree.clone()),
            None,
        )
        .unwrap()
        .value
    };
    let first = run(&sweep_script("len(fns)"));
    let cold = w.parse_stats();
    assert_eq!((cold.hits, cold.misses), (0, 9), "3 files x 3 builtins");
    assert_eq!(cold.entries, 9);

    // Another script over the same files reads, and parses nothing.
    let again = run(&sweep_script("len(fns)"));
    let other = run(&sweep_script("[g['name'] for g in fns]"));
    let warm = w.parse_stats();
    assert_eq!((warm.hits, warm.misses), (18, 9), "{warm:?}");
    assert_eq!(first, again, "the same facts come out");
    assert_ne!(first, other, "a different reduction still reads the cache");

    // The facts are what a fresh worker computes without anything cached.
    let fresh = workers(limits());
    let uncached = fresh
        .run(
            &budget(10_000),
            ScriptRun::starlark(&sweep_script("len(fns)"), json!({})).with_tree(tree.clone()),
            None,
        )
        .unwrap()
        .value;
    assert_eq!(first, uncached);

    // An edit is a new key: only that file is parsed again.
    let mut edited = (*tree).clone();
    edited.insert("d.rs".into(), "fn d() {}\nfn e() {}\n".into());
    let after = w
        .run(
            &budget(10_000),
            ScriptRun::starlark(&sweep_script("len(fns)"), json!({})).with_tree(Arc::new(edited)),
            None,
        )
        .unwrap()
        .value;
    let edited_stats = w.parse_stats();
    assert_eq!(
        (edited_stats.hits, edited_stats.misses),
        (warm.hits + 6, warm.misses + 3),
        "{edited_stats:?}"
    );
    assert_ne!(after, first);
}
