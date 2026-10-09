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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::ai::AiOracle;
use crate::runtime::{RunClock, SandboxBudget};
use crate::worker::{CallKind, CallOut, Engine, FromWorker, ToWorker, EXIT_MEMORY, MEMORY_ARG};
use crate::CollectError;

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

/// Asks a run to stop: the worker is killed at the next tick.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Answers a command script's `scope(id, args)` call.
pub type Serve<'a> = dyn FnMut(&str, Value) -> Result<Value, String> + 'a;

/// What a run's builtins may call out to (at most one).
pub enum RunHost<'a> {
    None,
    /// A fact collector's snapshot: shipped to the worker before the run.
    Tree(Arc<HashMap<String, String>>),
    /// A collector's model.
    Ai(Arc<dyn AiOracle>),
    /// A command script's scopes, answered on this thread.
    Scope(&'a mut Serve<'a>),
}

/// One script run.
pub struct ScriptRun<'a> {
    pub engine: Engine,
    pub script: String,
    pub input: Value,
    pub host: RunHost<'a>,
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
            host: RunHost::None,
        }
    }

    /// A jq program.
    pub fn jaq(program: &str, input: Value) -> Self {
        ScriptRun {
            engine: Engine::Jaq,
            script: program.into(),
            input,
            host: RunHost::None,
        }
    }

    pub fn with_host(self, host: RunHost<'a>) -> Self {
        ScriptRun { host, ..self }
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
        budget: &SandboxBudget,
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
            host,
        } = run;
        let (call_out, mut host) = match host {
            RunHost::None => (CallOut::None, Answering::None),
            RunHost::Tree(files) => {
                worker.send_tree(&files);
                (CallOut::Tree, Answering::None)
            }
            RunHost::Ai(oracle) => (CallOut::Ai, Answering::Ai(oracle)),
            RunHost::Scope(serve) => (CallOut::Scope, Answering::Scope(serve)),
        };
        worker.send(&ToWorker::Run {
            engine,
            script,
            input,
            host: call_out,
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
                    let answer = clock.paused(|| host.answer(kind, &id, args));
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
                    "couldn't start the script worker {}: {e}",
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

/// Who answers a run's calls.
enum Answering<'a> {
    None,
    Ai(Arc<dyn AiOracle>),
    Scope(&'a mut Serve<'a>),
}

impl Answering<'_> {
    fn answer(&mut self, kind: CallKind, id: &str, args: Value) -> Result<Value, String> {
        match (kind, self) {
            (CallKind::Scope, Answering::Scope(serve)) => serve(id, args),
            (CallKind::Ai, Answering::Ai(oracle)) => ask(&**oracle, id, &args),
            (CallKind::Scope, _) => Err("scope() is available in command handlers only".into()),
            (CallKind::Ai, _) => Err(format!(
                "ai_{id} is available in entity collectors only: a fact collector or a \
                 report parser can't call a model"
            )),
        }
    }
}

fn ask(oracle: &dyn AiOracle, op: &str, args: &Value) -> Result<Value, String> {
    let text = args["text"].as_str().unwrap_or_default();
    let strings = |key: &str| -> Vec<String> {
        serde_json::from_value(args[key].clone()).unwrap_or_default()
    };
    match op {
        "classify" => oracle.classify(text, &strings("labels")),
        "score" => oracle.score(text, &strings("levels")),
        "summarize" => oracle
            .summarize(text, args["focus"].as_str())
            .map(Value::String),
        "extract" => oracle.extract(
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
