//! Running an extension's `build:` (`.context/extensions-deploy.md` "Builds"):
//! its command in the source folder, as the person, before a deploy
//! copies. Not confined — a build is the person's own tool, which is what
//! the approval at deploy says.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::providers::host::HostEnv;

/// How long a build may run.
pub const TIMEOUT: Duration = Duration::from_secs(600);

/// How much of a failed build's output its error keeps: the end.
const TAIL: usize = 4 * 1024;

/// Run `run` with `/bin/sh -c` in `source`, with this process's
/// environment and the person's `PATH` and `HOME` from `env`. Its output
/// (stdout and stderr together) is kept only for a failure's message.
pub fn run(source: &Path, run: &str, env: &HostEnv, timeout: Duration) -> Result<(), String> {
    let (mut reader, writer) = std::io::pipe().map_err(|e| e.to_string())?;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(run)
        .current_dir(source)
        .stdin(Stdio::null())
        .stdout(writer.try_clone().map_err(|e| e.to_string())?)
        .stderr(writer);
    for name in ["PATH", "HOME"] {
        if let Some(v) = env(name) {
            cmd.env(name, v);
        }
    }
    crate::external_process::ProcessGroup::prepare(&mut cmd);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("couldn't start `{run}`: {e}"))?;
    // The pipe's write ends are the child's now: dropping ours lets the
    // read end see the end of its output.
    drop(cmd);
    let group = crate::external_process::ProcessGroup::new(child.id());
    let output = std::thread::spawn(move || {
        let mut kept: Vec<u8> = Vec::new();
        let mut buffer = [0; 16 * 1024];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&buffer[..n]);
            if kept.len() > 2 * TAIL {
                kept.drain(..kept.len() - TAIL);
            }
        }
        kept
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                group.stop();
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("`{run}` didn't finish in {}", minutes(timeout)));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(e) => {
                group.stop();
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    };
    // Whatever it left running in its group goes with it.
    group.stop();
    let output = output.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let start = output.len().saturating_sub(TAIL);
    let tail = String::from_utf8_lossy(&output[start..]).trim().to_string();
    let how = match status.code() {
        Some(code) => format!("exited {code}"),
        None => "was stopped by a signal".to_string(),
    };
    Err(if tail.is_empty() {
        format!("`{run}` {how}")
    } else {
        format!("`{run}` {how}:\n{tail}")
    })
}

fn minutes(d: Duration) -> String {
    match d.as_secs() {
        s if s >= 60 && s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HostEnv {
        crate::providers::host::process_env()
    }

    #[test]
    fn a_build_runs_in_the_source_folder() {
        let dir = tempfile::tempdir().unwrap();
        run(
            dir.path(),
            "mkdir -p dist && echo hi > dist/a",
            &env(),
            TIMEOUT,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("dist/a")).unwrap(),
            "hi\n"
        );
    }

    /// A failure says how it ended and the end of what it printed, on
    /// either stream.
    #[test]
    fn a_failed_build_says_how_and_shows_its_output() {
        let dir = tempfile::tempdir().unwrap();
        let e = run(
            dir.path(),
            "echo first; echo oops >&2; exit 3",
            &env(),
            TIMEOUT,
        )
        .unwrap_err();
        assert_eq!(
            e,
            "`echo first; echo oops >&2; exit 3` exited 3:\nfirst\noops"
        );
    }

    #[test]
    fn a_build_past_its_time_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let e = run(dir.path(), "sleep 30", &env(), Duration::from_millis(200)).unwrap_err();
        assert!(e.contains("didn't finish in 0 seconds"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
