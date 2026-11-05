//! The **script worker**: a process that runs one Starlark or jaq script
//! and exits, so the host can contain it (`.context/collection.md`
//! "Containing scripts"). A thread can only be abandoned; a process can be
//! killed and reaped. The host side is [`crate::workers::ScriptWorkers`].
//!
//! The binary is `oxplow-script-worker` (`src/bin`), shipped beside the
//! oxplow executable as a sidecar. It speaks NDJSON on stdin/stdout, one
//! run per process:
//!
//! - host → worker: an optional `{"tree": {files}}` (the run's
//!   snapshot, when it was given `tree.read`), then `{"run": {runtime,
//!   func, script, input, given}}`;
//! - worker → host: `{"call": {seq, kind, id, args}}` for each `scope(…)`
//!   or `ai_*` call, answered by host → worker `{"answer": {seq, ok | err}}`;
//!   finally `{"done": {ok | err, cpu_ms, peak_bytes}}`.
//!
//! Memory is capped by [`MemoryCap`], the binary's global allocator: past
//! the cap the worker exits with [`EXIT_MEMORY`] before allocating more.
//! A worker whose host is gone (its stdin closed while it waits on an
//! answer, or its parent replaced by init) exits on its own.

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::CollectError;

/// The worker exited because a run asked for more memory than its cap.
pub const EXIT_MEMORY: i32 = 3;

/// The argument the host passes the cap in bytes as: `--memory-bytes N`.
pub const MEMORY_ARG: &str = "--memory-bytes";

/// The argument the host passes its own process id as: `--parent-pid N`.
/// Windows has no cheap "my parent changed" check, so the worker waits on
/// a handle to this process; Unix workers ignore it.
pub const PARENT_ARG: &str = "--parent-pid";

/// Which engine runs the script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "runtime")]
pub enum Engine {
    Jaq,
    /// Starlark, calling `func` (`transform` unless a provider names
    /// another) with the input.
    Starlark {
        func: String,
    },
}

/// The services a run was given (`.context/extensions-permissions.md` "Services"):
/// the tree (shipped as `tree` before the run), a model and the scopes,
/// any of them. A builtin whose service wasn't given says which need it
/// wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Given {
    #[serde(default)]
    pub tree: bool,
    #[serde(default)]
    pub ai: bool,
    #[serde(default)]
    pub scope: bool,
}

/// What the host sends.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToWorker {
    Tree {
        files: HashMap<String, String>,
    },
    Run {
        #[serde(flatten)]
        engine: Engine,
        script: String,
        input: Value,
        #[serde(default)]
        given: Given,
    },
    Answer {
        seq: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        err: Option<String>,
    },
}

/// Which host a call goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallKind {
    /// `scope(id, args)`.
    Scope,
    /// `ai_<id>(…)`: `classify`, `score`, `summarize`, `extract`.
    Ai,
    /// The host's parse results, shared across runs: `get { key }` answers
    /// the JSON text stored under `key` (or null), `put { key, value }`
    /// stores it. Answered by the host itself, never by the caller.
    Parse,
}

/// A run's failure, as the worker reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// `runtime`, `shape`, `parse`, `container`, … (a [`CollectError`]'s).
    pub kind: String,
    pub message: String,
}

/// What the worker sends.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FromWorker {
    Call {
        seq: u64,
        kind: CallKind,
        id: String,
        args: Value,
    },
    Done {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ok: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        err: Option<Failure>,
        cpu_ms: u64,
        peak_bytes: u64,
    },
}

impl Failure {
    pub fn of(e: &CollectError) -> Self {
        let kind = match e {
            CollectError::UnknownFormat(_) => "unknown_format",
            CollectError::Parse(_) => "parse",
            CollectError::Container(_) => "container",
            CollectError::Runtime(_) => "runtime",
            CollectError::Shape(_) => "shape",
            CollectError::Exec(_) => "exec",
            _ => "runtime",
        };
        let message = match e {
            CollectError::UnknownFormat(m)
            | CollectError::Parse(m)
            | CollectError::Container(m)
            | CollectError::Runtime(m)
            | CollectError::Shape(m)
            | CollectError::Exec(m) => m.clone(),
            other => other.to_string(),
        };
        Failure {
            kind: kind.into(),
            message,
        }
    }

    pub fn into_error(self) -> CollectError {
        match self.kind.as_str() {
            "unknown_format" => CollectError::UnknownFormat(self.message),
            "parse" => CollectError::Parse(self.message),
            "container" => CollectError::Container(self.message),
            "shape" => CollectError::Shape(self.message),
            "exec" => CollectError::Exec(self.message),
            _ => CollectError::Runtime(self.message),
        }
    }
}

// ---- the memory cap -------------------------------------------------

static LIMIT: AtomicUsize = AtomicUsize::new(usize::MAX);
static USED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);

/// The worker binary's global allocator: [`System`], counting live bytes.
/// An allocation that would take the count past the cap ends the process
/// with [`EXIT_MEMORY`] instead of being made, so a runaway script can't
/// take the machine's memory with it.
pub struct MemoryCap;

impl MemoryCap {
    /// Cap live allocations at `bytes`.
    pub fn arm(bytes: usize) {
        LIMIT.store(bytes, Ordering::SeqCst);
    }

    /// The most bytes live at once so far.
    pub fn peak() -> u64 {
        PEAK.load(Ordering::Relaxed)
    }

    fn take(size: usize) {
        let now = USED.fetch_add(size, Ordering::Relaxed) + size;
        if now > LIMIT.load(Ordering::Relaxed) {
            // Nothing may allocate from here on: leave at once.
            exit_now(EXIT_MEMORY);
        }
        PEAK.fetch_max(now as u64, Ordering::Relaxed);
    }

    fn give(size: usize) {
        USED.fetch_sub(size, Ordering::Relaxed);
    }
}

// SAFETY: every call forwards to `System` with the same layout; the
// counting only reads and writes atomics.
unsafe impl GlobalAlloc for MemoryCap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        Self::take(layout.size());
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        Self::take(layout.size());
        System.alloc_zeroed(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        Self::give(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            Self::take(new_size - layout.size());
        } else {
            Self::give(layout.size() - new_size);
        }
        System.realloc(ptr, layout, new_size)
    }
}

// ---- the worker -----------------------------------------------------

/// The worker's line to its host: what a `scope(…)` or `ai_*` call writes
/// and waits on.
struct Line {
    out: std::io::Stdout,
    input: std::io::Stdin,
    seq: u64,
}

impl Line {
    fn send(&mut self, msg: &FromWorker) {
        let text = serde_json::to_string(msg).expect("a worker message serializes");
        let mut out = self.out.lock();
        // A host that's gone can't be told anything: leave.
        if writeln!(out, "{text}").and_then(|()| out.flush()).is_err() {
            std::process::exit(0);
        }
    }

    fn read(&mut self) -> ToWorker {
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            // The host is gone.
            Ok(0) | Err(_) => std::process::exit(0),
            Ok(_) => serde_json::from_str(&line)
                .unwrap_or_else(|e| fail_hard(&format!("a host message isn't one: {e}"))),
        }
    }

    fn call(&mut self, kind: CallKind, id: &str, args: Value) -> Result<Value, String> {
        self.seq += 1;
        let seq = self.seq;
        self.send(&FromWorker::Call {
            seq,
            kind,
            id: id.to_string(),
            args,
        });
        match self.read() {
            ToWorker::Answer { seq: s, ok, err } if s == seq => match (ok, err) {
                (_, Some(e)) => Err(e),
                (Some(v), None) => Ok(v),
                (None, None) => Ok(Value::Null),
            },
            _ => fail_hard("the host answered something other than this call"),
        }
    }
}

fn fail_hard(message: &str) -> ! {
    eprintln!("oxplow-script-worker: {message}");
    std::process::exit(2)
}

/// The host's parse results, as the worker's builtins reach them: a miss
/// is computed here and stored there, so the next run (of another script
/// over the same file) finds it.
struct RemoteParses(Arc<Mutex<Line>>);

impl crate::runtime::ParseMemo for RemoteParses {
    fn get(&self, key: &str) -> Option<String> {
        match self.0.lock().unwrap_or_else(|e| e.into_inner()).call(
            CallKind::Parse,
            "get",
            serde_json::json!({ "key": key }),
        ) {
            Ok(Value::String(json)) => Some(json),
            _ => None,
        }
    }

    fn put(&self, key: &str, value: &str) {
        let _ = self.0.lock().unwrap_or_else(|e| e.into_inner()).call(
            CallKind::Parse,
            "put",
            serde_json::json!({ "key": key, "value": value }),
        );
    }
}

/// The model, as the worker's `ai_*` builtins see it: each call crosses to
/// the host, which asks its oracle (with the run's clock paused).
struct RemoteOracle(Arc<Mutex<Line>>);

impl RemoteOracle {
    fn ask(&self, id: &str, args: Value) -> Result<Value, String> {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .call(CallKind::Ai, id, args)
    }
}

impl crate::ai::AiOracle for RemoteOracle {
    fn classify(&self, role: &str, text: &str, labels: &[String]) -> Result<Value, String> {
        self.ask(
            "classify",
            serde_json::json!({ "role": role, "text": text, "labels": labels }),
        )
    }

    fn score(&self, role: &str, text: &str, levels: &[String]) -> Result<Value, String> {
        self.ask(
            "score",
            serde_json::json!({ "role": role, "text": text, "levels": levels }),
        )
    }

    fn summarize(&self, role: &str, text: &str, focus: Option<&str>) -> Result<String, String> {
        match self.ask(
            "summarize",
            serde_json::json!({ "role": role, "text": text, "focus": focus }),
        )? {
            Value::String(s) => Ok(s),
            other => Err(format!("a summary should be text, not {other}")),
        }
    }

    fn extract(
        &self,
        role: &str,
        instructions: &str,
        text: &str,
        schema: &Value,
    ) -> Result<Value, String> {
        self.ask(
            "extract",
            serde_json::json!({
                "role": role,
                "instructions": instructions,
                "text": text,
                "schema": schema,
            }),
        )
    }
}

/// The worker's main: arm the cap from argv, leave when the parent goes,
/// read the run, run it, report. Never returns.
pub fn main() -> ! {
    let mut args = std::env::args().skip(1);
    let mut parent_pid = None;
    while let Some(arg) = args.next() {
        if arg == MEMORY_ARG {
            if let Some(n) = args.next().and_then(|n| n.parse::<usize>().ok()) {
                MemoryCap::arm(n);
            }
        } else if arg == PARENT_ARG {
            parent_pid = args.next().and_then(|n| n.parse::<u32>().ok());
        }
    }
    watch_parent(parent_pid);
    let line = Arc::new(Mutex::new(Line {
        out: std::io::stdout(),
        input: std::io::stdin(),
        seq: 0,
    }));
    let mut tree = None;
    let (engine, script, input, given) = loop {
        let msg = line.lock().unwrap_or_else(|e| e.into_inner()).read();
        match msg {
            ToWorker::Tree { files } => tree = Some(crate::runtime::TreeHost::new(files)),
            ToWorker::Run {
                engine,
                script,
                input,
                given,
            } => break (engine, script, input, given),
            ToWorker::Answer { .. } => fail_hard("an answer before any call"),
        }
    };
    let out = run(&line, engine, &script, &input, given, tree);
    let (ok, err) = match out {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(Failure::of(&e))),
    };
    let done = FromWorker::Done {
        ok,
        err,
        cpu_ms: cpu_ms(),
        peak_bytes: MemoryCap::peak(),
    };
    line.lock().unwrap_or_else(|e| e.into_inner()).send(&done);
    std::process::exit(0)
}

fn run(
    line: &Arc<Mutex<Line>>,
    engine: Engine,
    script: &str,
    input: &Value,
    given: Given,
    tree: Option<crate::runtime::TreeHost>,
) -> Result<Value, CollectError> {
    use crate::runtime::{run_jaq, run_starlark_inner, Hosts};
    let func = match engine {
        Engine::Jaq => return run_jaq(script, input),
        Engine::Starlark { func } => func,
    };
    let hosts = Hosts {
        tree: given.tree.then(|| tree.unwrap_or_default()),
        ai: given
            .ai
            .then(|| crate::ai::AiHost::new(Arc::new(RemoteOracle(line.clone())))),
        parses: Some(Arc::new(RemoteParses(line.clone()))),
        scope: given.scope.then(|| {
            let line = line.clone();
            crate::scope::ScopeHost::new(Box::new(move |id, args| {
                line.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .call(CallKind::Scope, id, args)
            }))
        }),
    };
    run_starlark_inner(script, &func, input, &hosts)
}

/// End the process at once, running nothing that could allocate.
fn exit_now(code: i32) -> ! {
    #[cfg(unix)]
    // SAFETY: `_exit` ends the process without running anything that
    // could allocate.
    unsafe {
        libc::_exit(code)
    }
    #[cfg(windows)]
    // SAFETY: `TerminateProcess` on the current process ends it at once.
    unsafe {
        windows_sys::Win32::System::Threading::TerminateProcess(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            code as u32,
        );
        // Not reached: the process is gone.
        loop {}
    }
}

/// The process's CPU time (user + system) so far, in milliseconds.
#[cfg(unix)]
fn cpu_ms() -> u64 {
    // SAFETY: `getrusage` fills the zeroed struct it's given.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let ms = |t: libc::timeval| t.tv_sec as u64 * 1000 + t.tv_usec as u64 / 1000;
    ms(usage.ru_utime) + ms(usage.ru_stime)
}

/// The process's CPU time (user + system) so far, in milliseconds.
#[cfg(windows)]
fn cpu_ms() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: `GetProcessTimes` fills the four structs it's given.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    if ok == 0 {
        return 0;
    }
    // FILETIME counts 100 ns ticks.
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    (ticks(kernel) + ticks(user)) / 10_000
}

/// Leave when the host process is gone (the worker re-parented to init),
/// even mid-computation: nothing would ever read its answer.
#[cfg(unix)]
fn watch_parent(_parent_pid: Option<u32>) {
    // SAFETY: `getppid` has no preconditions.
    let parent = unsafe { libc::getppid() };
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(250));
        // SAFETY: as above.
        if unsafe { libc::getppid() } != parent {
            std::process::exit(0);
        }
    });
}

/// Leave when the host process is gone: wait on a handle to it (its pid is
/// the host's `--parent-pid`), even mid-computation. Without the pid, or
/// when the host can't be opened, the worker leaves at once rather than
/// outlive a host it can't watch.
#[cfg(windows)]
fn watch_parent(parent_pid: Option<u32>) {
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };
    // SAFETY: `OpenProcess` takes plain values; a null handle is checked.
    let handle = parent_pid.map(|pid| unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) });
    match handle {
        Some(h) if !h.is_null() => {
            // Handles are plain pointers; move it as an integer.
            let h = h as usize;
            std::thread::spawn(move || {
                // SAFETY: the handle stays open for the process's life.
                if unsafe { WaitForSingleObject(h as _, INFINITE) } == WAIT_OBJECT_0 {
                    std::process::exit(0);
                }
            });
        }
        _ => std::process::exit(0),
    }
}
