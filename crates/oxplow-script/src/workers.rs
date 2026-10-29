//! The host side of the script worker ([`crate::worker`]): every Starlark
//! and jaq run is one `oxplow-script-worker` process, which this kills and
//! reaps when its budget runs out or its run is cancelled — never a thread
//! left spinning. [`ScriptWorkers`] bounds how many run at once and how
//! much memory each may take (`scripts:` in `.oxplow/project.yaml`).
//!
//! A run's calls out — a command's `scope(…)`, a collector's `ai_*` — are
//! answered on the calling thread, which owns what the answer needs (a
//! command's transaction isn't `Send`); the time spent answering is left
//! out of the budget's `timeout` and counted against its `ceiling`, as it
//! was in-process. Callers block: run them off the async runtime.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ai::AiOracle;
use crate::runtime::RunClock;
use crate::worker::{CallKind, Engine, FromWorker, Given, ToWorker, EXIT_MEMORY, MEMORY_ARG};
use crate::CollectError;
pub use oxplow_domain::run::{Budget, Cancel};

/// The worker binary's name, beside the oxplow executable.
pub const WORKER: &str = "oxplow-script-worker";

/// How long a run waits between looks at its deadline and cancel flag.
const TICK: Duration = Duration::from_millis(20);

/// The host's bounds on script workers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Runs at once; one more waits for a slot.
    pub max_concurrent: usize,
    /// Live bytes one run may allocate.
    pub memory_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_concurrent: std::thread::available_parallelism()
                .map(|n| n.get().min(4))
                .unwrap_or(2),
            memory_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Answers a command script's `scope(id, args)` call.
pub type Serve<'a> = dyn FnMut(&str, Value) -> Result<Value, String> + 'a;

/// The services a run is given (`.context/extensions.md` "Services"):
/// any of them, each answered by what the caller passes.
#[derive(Default)]
pub struct Hosts<'a> {
    /// `tree.read`: the snapshot, shipped to the worker before the run.
    pub tree: Option<Arc<HashMap<String, String>>>,
    /// `ai.compute`: the model, answered on this thread.
    pub ai: Option<Arc<dyn AiOracle>>,
    /// The scopes (`sql.read`), answered on this thread.
    pub scope: Option<&'a mut Serve<'a>>,
}

/// One script run.
pub struct ScriptRun<'a> {
    pub engine: Engine,
    pub script: String,
    pub input: Value,
    pub hosts: Hosts<'a>,
}

impl<'a> ScriptRun<'a> {
    /// A Starlark script's `transform`.
    pub fn starlark(script: &str, input: Value) -> Self {
        Self::starlark_fn(script, "transform", input)
    }

    /// A Starlark script's `func`.
    pub fn starlark_fn(script: &str, func: &str, input: Value) -> Self {
        ScriptRun {
            engine: Engine::Starlark { func: func.into() },
            script: script.into(),
            input,
            hosts: Hosts::default(),
        }
    }

    /// A jq program.
    pub fn jaq(program: &str, input: Value) -> Self {
        ScriptRun {
            engine: Engine::Jaq,
            script: program.into(),
            input,
            hosts: Hosts::default(),
        }
    }

    /// Given the tree (`tree.read`).
    pub fn with_tree(mut self, files: Arc<HashMap<String, String>>) -> Self {
        self.hosts.tree = Some(files);
        self
    }

    /// Given a model (`ai.compute`).
    pub fn with_ai(mut self, oracle: Arc<dyn AiOracle>) -> Self {
        self.hosts.ai = Some(oracle);
        self
    }

    /// Given the scopes, answered by `serve`.
    pub fn with_scope(mut self, serve: &'a mut Serve<'a>) -> Self {
        self.hosts.scope = Some(serve);
        self
    }

    /// Given `hosts`.
    pub fn with_hosts(self, hosts: Hosts<'a>) -> Self {
        ScriptRun { hosts, ..self }
    }
}

/// A finished run: its value and what it cost.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    pub value: Value,
    pub cpu_ms: u64,
    pub peak_bytes: u64,
}

/// The process's workers: the bound on concurrent runs is the host's, so
/// every caller in a process shares one. Its limits follow the project's
/// `scripts:` config ([`ScriptWorkers::set_limits`]).
pub fn shared() -> &'static ScriptWorkers {
    static SHARED: std::sync::OnceLock<ScriptWorkers> = std::sync::OnceLock::new();
    SHARED.get_or_init(|| ScriptWorkers::beside_current_exe(Limits::default()))
}

/// Runs scripts in worker processes, a bounded number at once.
pub struct ScriptWorkers {
    exe: PathBuf,
    limits: Mutex<Limits>,
    running: Mutex<usize>,
    freed: Condvar,
    live: AtomicUsize,
    recent: Mutex<VecDeque<u32>>,
    /// Parse results shared by every run this serves.
    parses: ParseCache,
}

/// The most JSON text the shared parse results hold.
const PARSE_CACHE_BYTES: usize = 256 * 1024 * 1024;

/// What a worker parsed, kept by the host while it lives: workers are one
/// process per run, so the results outlive them here. Keyed by
/// [`crate::runtime::parse_key`] (the builtin, the language and the
/// content's hash), so an edited file is a new entry and an unchanged one
/// is read, not parsed, by every script of a sweep. Oldest entries go
/// first once the bound is reached.
#[derive(Default)]
struct ParseCache {
    entries: Mutex<ParseEntries>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

#[derive(Default)]
struct ParseEntries {
    by_key: HashMap<String, Arc<str>>,
    order: VecDeque<String>,
    bytes: usize,
}

impl ParseCache {
    fn answer(&self, id: &str, args: &Value) -> Result<Value, String> {
        let key = args["key"].as_str().ok_or("a parse call needs its key")?;
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        match id {
            "get" => Ok(match entries.by_key.get(key) {
                Some(json) => {
                    self.hits.fetch_add(1, Ordering::Relaxed);
                    Value::String(json.to_string())
                }
                None => {
                    self.misses.fetch_add(1, Ordering::Relaxed);
                    Value::Null
                }
            }),
            "put" => {
                let value = args["value"]
                    .as_str()
                    .ok_or("a parse put needs its value")?;
                if entries.by_key.contains_key(key) || value.len() > PARSE_CACHE_BYTES / 16 {
                    return Ok(Value::Null);
                }
                entries.bytes += value.len();
                entries.by_key.insert(key.to_string(), value.into());
                entries.order.push_back(key.to_string());
                while entries.bytes > PARSE_CACHE_BYTES {
                    let Some(oldest) = entries.order.pop_front() else {
                        break;
                    };
                    if let Some(gone) = entries.by_key.remove(&oldest) {
                        entries.bytes -= gone.len();
                    }
                }
                Ok(Value::Null)
            }
            other => Err(format!("no parse call `{other}`")),
        }
    }
}

/// How the shared parse results have done so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseStats {
    pub entries: usize,
    pub hits: usize,
    pub misses: usize,
}

impl ScriptWorkers {
    pub fn new(exe: PathBuf, limits: Limits) -> Self {
        ScriptWorkers {
            exe,
            limits: Mutex::new(limits),
            running: Mutex::new(0),
            freed: Condvar::new(),
            live: AtomicUsize::new(0),
            recent: Mutex::new(VecDeque::new()),
            parses: ParseCache::default(),
        }
    }

    /// What the shared parse results hold and how often they answered.
    pub fn parse_stats(&self) -> ParseStats {
        ParseStats {
            entries: self
                .parses
                .entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .by_key
                .len(),
            hits: self.parses.hits.load(Ordering::Relaxed),
            misses: self.parses.misses.load(Ordering::Relaxed),
        }
    }

    /// Workers from the binary beside this executable: the app's sidecar,
    /// `target/<profile>/` in development, and the same directory for a
    /// test binary in `target/<profile>/deps/`.
    pub fn beside_current_exe(limits: Limits) -> Self {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
            .map(|dir| {
                if dir.file_name().is_some_and(|n| n == "deps") {
                    dir.parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or(dir)
                } else {
                    dir
                }
            })
            .map(|dir| dir.join(WORKER))
            .unwrap_or_else(|| PathBuf::from(WORKER));
        Self::new(exe, limits)
    }

    /// The worker binary this runs.
    pub fn exe(&self) -> &std::path::Path {
        &self.exe
    }

    /// New bounds, for runs that start from now.
    pub fn set_limits(&self, limits: Limits) {
        *self.limits.lock().unwrap_or_else(|e| e.into_inner()) = limits;
        self.freed.notify_all();
    }

    pub fn limits(&self) -> Limits {
        *self.limits.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Workers spawned and not yet reaped.
    pub fn live(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }

    /// The pids of the most recent workers (diagnostics and tests).
    pub fn recent_pids(&self) -> Vec<u32> {
        self.recent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .copied()
            .collect()
    }

    /// Run `run` within `budget`, stopping and reaping its worker when the
    /// budget runs out ([`CollectError::Timeout`]), `cancel` is set
    /// ([`CollectError::Cancelled`]) or it exceeds its memory
    /// ([`CollectError::MemoryExceeded`]). Blocks.
    pub fn run(
        &self,
        budget: &Budget,
        run: ScriptRun<'_>,
        cancel: Option<&Cancel>,
    ) -> Result<Ran, CollectError> {
        let called = Instant::now();
        let ceiling = called + budget.ceiling;
        let _slot = self.slot(ceiling, cancel)?;
        let limits = self.limits();
        let mut worker = self.spawn(limits.memory_bytes)?;
        let ScriptRun {
            engine,
            script,
            input,
            hosts,
        } = run;
        let given = Given {
            tree: hosts.tree.is_some(),
            ai: hosts.ai.is_some(),
            scope: hosts.scope.is_some(),
        };
        if let Some(files) = &hosts.tree {
            worker.send_tree(files);
        }
        let mut host = Answering {
            ai: hosts.ai,
            scope: hosts.scope,
        };
        worker.send(&ToWorker::Run {
            engine,
            script,
            input,
            given,
        });
        let clock = RunClock::default();
        let started = Instant::now();
        loop {
            if cancel.is_some_and(Cancel::is_cancelled) {
                worker.stop();
                return Err(CollectError::Cancelled);
            }
            let deadline = (started + budget.timeout + clock.total()).min(ceiling);
            let now = Instant::now();
            if now >= deadline {
                worker.stop();
                return Err(CollectError::Timeout);
            }
            match worker.lines.recv_timeout((deadline - now).min(TICK)) {
                Ok(Ok(FromWorker::Call {
                    seq,
                    kind,
                    id,
                    args,
                })) => {
                    let answer = match kind {
                        // The host's own, cheap: not the caller's, and not
                        // time the script's clock should stop for.
                        CallKind::Parse => self.parses.answer(&id, &args),
                        _ => clock.paused(|| host.answer(kind, &id, args)),
                    };
                    let (ok, err) = match answer {
                        Ok(v) => (Some(v), None),
                        Err(e) => (None, Some(e)),
                    };
                    worker.send(&ToWorker::Answer { seq, ok, err });
                }
                Ok(Ok(FromWorker::Done {
                    ok,
                    err,
                    cpu_ms,
                    peak_bytes,
                })) => {
                    worker.finish();
                    return match (ok, err) {
                        (_, Some(e)) => Err(e.into_error()),
                        (value, None) => Ok(Ran {
                            value: value.unwrap_or(Value::Null),
                            cpu_ms,
                            peak_bytes,
                        }),
                    };
                }
                Ok(Err(bad)) => {
                    worker.stop();
                    return Err(CollectError::WorkerDied(bad));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(worker.died(limits.memory_bytes));
                }
            }
        }
    }

    /// Wait for a slot, until `ceiling` or a cancel.
    fn slot(&self, ceiling: Instant, cancel: Option<&Cancel>) -> Result<Slot<'_>, CollectError> {
        let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if *running < self.limits().max_concurrent.max(1) {
                *running += 1;
                return Ok(Slot(self));
            }
            if cancel.is_some_and(Cancel::is_cancelled) {
                return Err(CollectError::Cancelled);
            }
            let now = Instant::now();
            if now >= ceiling {
                return Err(CollectError::Timeout);
            }
            running = self
                .freed
                .wait_timeout(running, (ceiling - now).min(TICK))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn spawn(&self, memory_bytes: u64) -> Result<Worker<'_>, CollectError> {
        let mut child = Command::new(&self.exe)
            .arg(MEMORY_ARG)
            .arg(memory_bytes.to_string())
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                CollectError::WorkerDied(format!(
                    "couldn't start the script worker {}: {e} (it is built with the sidecars: `bun run build:dev`)",
                    self.exe.display()
                ))
            })?;
        self.live.fetch_add(1, Ordering::SeqCst);
        {
            let mut recent = self.recent.lock().unwrap_or_else(|e| e.into_inner());
            recent.push_back(child.id());
            while recent.len() > 64 {
                recent.pop_front();
            }
        }
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let msg = serde_json::from_str::<FromWorker>(&line)
                    .map_err(|e| format!("the script worker said something else: {e}"));
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        let errors = Arc::new(Mutex::new(Vec::new()));
        let tail = errors.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = BufReader::new(stderr).take(64 * 1024).read_to_end(&mut buf);
            *tail.lock().unwrap_or_else(|e| e.into_inner()) = buf;
        });
        Ok(Worker {
            owner: self,
            child: Some(child),
            stdin,
            lines,
            errors,
        })
    }
}

/// A held concurrency slot.
struct Slot<'a>(&'a ScriptWorkers);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut running = self.0.running.lock().unwrap_or_else(|e| e.into_inner());
        *running = running.saturating_sub(1);
        self.0.freed.notify_one();
    }
}

/// Who answers a run's calls: the worker only calls out to what it was
/// given, so a call to anything else is refused as the worker would.
struct Answering<'a> {
    ai: Option<Arc<dyn AiOracle>>,
    scope: Option<&'a mut Serve<'a>>,
}

impl Answering<'_> {
    fn answer(&mut self, kind: CallKind, id: &str, args: Value) -> Result<Value, String> {
        match kind {
            CallKind::Scope => match &mut self.scope {
                Some(serve) => serve(id, args),
                None => Err(oxplow_domain::service::undeclared("scope")),
            },
            CallKind::Parse => Err("a parse call is answered by the workers".into()),
            CallKind::Ai => match &self.ai {
                Some(oracle) => ask(&**oracle, id, &args),
                None => Err(oxplow_domain::service::undeclared(&format!("ai_{id}"))),
            },
        }
    }
}

fn ask(oracle: &dyn AiOracle, op: &str, args: &Value) -> Result<Value, String> {
    let text = args["text"].as_str().unwrap_or_default();
    let role = args["role"].as_str().unwrap_or_default();
    let strings = |key: &str| -> Vec<String> {
        serde_json::from_value(args[key].clone()).unwrap_or_default()
    };
    match op {
        "classify" => oracle.classify(role, text, &strings("labels")),
        "score" => oracle.score(role, text, &strings("levels")),
        "summarize" => oracle
            .summarize(role, text, args["focus"].as_str())
            .map(Value::String),
        "extract" => oracle.extract(
            role,
            args["instructions"].as_str().unwrap_or_default(),
            text,
            &args["schema"],
        ),
        other => Err(format!("no model call `{other}`")),
    }
}

/// A running worker; dropping it kills and reaps the process.
struct Worker<'a> {
    owner: &'a ScriptWorkers,
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<Result<FromWorker, String>>,
    errors: Arc<Mutex<Vec<u8>>>,
}

impl Worker<'_> {
    /// The tree, written straight from the shared map (no copy of it).
    fn send_tree(&mut self, files: &HashMap<String, String>) {
        #[derive(serde::Serialize)]
        struct Tree<'a> {
            tree: Files<'a>,
        }
        #[derive(serde::Serialize)]
        struct Files<'a> {
            files: &'a HashMap<String, String>,
        }
        let text = serde_json::to_string(&Tree {
            tree: Files { files },
        })
        .expect("a tree serializes");
        self.write(&text);
    }

    fn send(&mut self, msg: &ToWorker) {
        let text = serde_json::to_string(msg).expect("a host message serializes");
        self.write(&text);
    }

    fn write(&mut self, text: &str) {
        // A worker that already died is found by its closed stdout.
        if let Some(stdin) = self.stdin.as_mut() {
            if writeln!(stdin, "{text}")
                .and_then(|()| stdin.flush())
                .is_err()
            {
                self.stdin = None;
            }
        }
    }

    /// Kill it and wait for it.
    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            self.owner.live.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// It said it's done: close its stdin and wait for it to exit.
    fn finish(&mut self) {
        self.stdin = None;
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) if Instant::now() < deadline => std::thread::sleep(TICK),
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                }
            }
            self.owner.live.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// It ended without saying it was done: why.
    fn died(&mut self, memory_bytes: u64) -> CollectError {
        let status = self.child.take().map(|mut child| {
            let status = child.wait();
            self.owner.live.fetch_sub(1, Ordering::SeqCst);
            status
        });
        match status {
            Some(Ok(s)) if s.code() == Some(EXIT_MEMORY) => {
                CollectError::MemoryExceeded(memory_bytes)
            }
            other => {
                // The stderr thread finishes once the pipe closes.
                std::thread::sleep(Duration::from_millis(20));
                let tail =
                    String::from_utf8_lossy(&self.errors.lock().unwrap_or_else(|e| e.into_inner()))
                        .trim()
                        .chars()
                        .rev()
                        .take(2000)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<String>();
                let how = match other {
                    Some(Ok(s)) => s.to_string(),
                    Some(Err(e)) => e.to_string(),
                    None => "it was already stopped".into(),
                };
                CollectError::WorkerDied(if tail.is_empty() {
                    how
                } else {
                    format!("{how}: {tail}")
                })
            }
        }
    }
}

impl Drop for Worker<'_> {
    fn drop(&mut self) {
        self.stop();
    }
}
