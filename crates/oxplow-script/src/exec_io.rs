//! Deadline-bound pipe I/O. A broader process can leave inherited pipes
//! open outside its group; that must not keep host drain threads alive.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

pub trait Pipe {
    fn nonblocking(&self) -> std::io::Result<()>;
}

#[cfg(unix)]
impl<T: std::os::fd::AsRawFd> Pipe for T {
    fn nonblocking(&self) -> std::io::Result<()> {
        let fd = self.as_raw_fd();
        // SAFETY: the pipe owns fd throughout both calls. F_SETFL only
        // changes its file status flags; it takes no pointer argument.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(not(unix))]
impl<T> Pipe for T {
    fn nonblocking(&self) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "controlled external pipe I/O is unavailable",
        ))
    }
}

pub fn read(
    mut pipe: impl Read + Pipe,
    limit: usize,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    pipe.nonblocking().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 16 * 1024];
    loop {
        if Instant::now() >= deadline {
            return Err("external output pipe remained open beyond its budget".into());
        }
        let room = buffer
            .len()
            .min(limit.saturating_add(1).saturating_sub(bytes.len()));
        if room == 0 {
            return Ok(bytes);
        }
        match pipe.read(&mut buffer[..room]) {
            Ok(0) => return Ok(bytes),
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(e) => return Err(e.to_string()),
        }
    }
}

pub fn write(
    mut pipe: impl Write + Pipe,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), String> {
    pipe.nonblocking().map_err(|e| e.to_string())?;
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err("external input pipe exceeded its budget".into());
        }
        match pipe.write(bytes) {
            Ok(0) => return Err("external input pipe closed".into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(2))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => (),
            Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
