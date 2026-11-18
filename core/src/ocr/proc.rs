//! Subprocess helper for OCR: run an external command (Tesseract) with a
//! hard timeout and captured stdout/stderr. No panics — every failure mode
//! becomes a [`CoreError::Ocr`].
//!
//! Reader threads drain the output pipes while the parent polls for exit so
//! a chatty child can never deadlock on a full pipe buffer. After the child
//! exits (or is killed) we wait a short grace period for the readers — a
//! grandchild that inherited the pipe can keep it open, and the run must
//! not hang on it.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::error::{CoreError, CoreResult};

/// Grace period for pipe-reader threads after the child is gone. Only
/// pathological grandchildren (inheriting the pipe) can hit this; the
/// reader thread finishes on its own later and is simply detached.
const PIPE_DRAIN_GRACE: Duration = Duration::from_secs(1);

/// Captured result of one timed subprocess run.
pub(crate) struct ProcOutput {
    pub status_success: bool,
    pub stdout: Vec<u8>,
    pub stderr: String,
    /// True when the timeout elapsed and the child had to be killed.
    pub timed_out: bool,
}

/// Run `cmd` with the given timeout, capturing stdout/stderr.
///
/// On timeout the child is killed and reaped; the result is returned with
/// `timed_out = true` rather than an error so callers can produce precise,
/// honest warnings.
pub(crate) fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> CoreResult<ProcOutput> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        CoreError::Ocr(format!(
            "failed to spawn {}: {e}",
            cmd.get_program().to_string_lossy()
        ))
    })?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    // Drain pipes on helper threads through channels: the parent never
    // reads the pipes itself, so a full pipe cannot stall the child, and
    // the parent never blocks longer than the grace period on a reader
    // whose pipe is still held open by a surviving grandchild.
    let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>();
    let (stderr_tx, stderr_rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = stdout_pipe {
            use std::io::Read;
            let _ = pipe.read_to_end(&mut buf);
        }
        let _ = stdout_tx.send(buf);
    });
    std::thread::spawn(move || {
        let mut raw = Vec::new();
        if let Some(mut pipe) = stderr_pipe {
            use std::io::Read;
            let _ = pipe.read_to_end(&mut raw);
        }
        let _ = stderr_tx.send(String::from_utf8_lossy(&raw).into_owned());
    });

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if started.elapsed() >= timeout {
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(CoreError::Ocr(format!("waiting on child failed: {e}")));
            }
        }
    };

    let mut timed_out = false;
    let status = match status {
        Some(s) => s,
        None => {
            timed_out = true;
            let _ = child.kill();
            // Reap so the child does not linger as a zombie.
            child
                .wait()
                .map_err(|e| CoreError::Ocr(format!("reaping killed child failed: {e}")))?
        }
    };

    let stdout = stdout_rx.recv_timeout(PIPE_DRAIN_GRACE).unwrap_or_default();
    let stderr = stderr_rx.recv_timeout(PIPE_DRAIN_GRACE).unwrap_or_default();

    Ok(ProcOutput {
        status_success: status.success(),
        stdout,
        stderr,
        timed_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn captures_stdout_and_exit_status() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo hello; exit 0"]);
        let out = run_with_timeout(&mut cmd, Duration::from_secs(10)).unwrap();
        assert!(out.status_success);
        assert!(!out.timed_out);
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hello");
    }

    #[test]
    fn times_out_and_kills_sleeper() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 30"]);
        let started = Instant::now();
        let out = run_with_timeout(&mut cmd, Duration::from_millis(200)).unwrap();
        assert!(out.timed_out);
        // Killed promptly, not waited out for the full 30s sleep.
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn grandchild_holding_pipe_does_not_hang_the_run() {
        // `sleep` survives its killed shell and inherits the pipe; the run
        // must still return promptly after the timeout.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60 & wait"]);
        let started = Instant::now();
        let out = run_with_timeout(&mut cmd, Duration::from_millis(300)).unwrap();
        assert!(out.timed_out);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn large_stdout_does_not_deadlock_on_pipe_fill() {
        // ~1 MiB of output far exceeds the default 64 KiB pipe buffer; the
        // reader threads must keep it flowing while the parent polls.
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "head -c 1048576 /dev/zero"]);
        let out = run_with_timeout(&mut cmd, Duration::from_secs(15)).unwrap();
        assert!(out.status_success);
        assert_eq!(out.stdout.len(), 1_048_576);
    }

    #[test]
    fn missing_binary_is_an_error_not_a_panic() {
        let mut cmd = Command::new("definitely-not-a-real-binary-xyz");
        assert!(run_with_timeout(&mut cmd, Duration::from_secs(1)).is_err());
    }
}
