//! Runs a command with a kill timeout, so a hung `git`, `mdfind`, `open` or `osascript` never
//! stalls the caller.

use std::io::Read;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Runs `command` to the end and returns its status and stdout. Stderr is dropped. Kills the child
/// and fails when it is still running after `timeout`.
pub fn output(command: &mut Command, timeout: Duration) -> Result<Output> {
    let name = command.get_program().to_string_lossy().into_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("running {name}"))?;
    let mut stdout = child.stdout.take().context("reading stdout")?;
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("waiting for {name}"))?
        {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            bail!("{name} did not finish in {} s", timeout.as_secs());
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = reader.join().unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_the_output_of_a_finished_command() {
        let out = output(Command::new("echo").arg("hi"), Duration::from_secs(5)).expect("runs");
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi\n");
    }

    #[test]
    fn reads_output_larger_than_the_pipe_buffer() {
        let mut command = Command::new("sh");
        command.args(["-c", "head -c 300000 /dev/zero"]);
        let out = output(&mut command, Duration::from_secs(5)).expect("runs");
        assert_eq!(out.stdout.len(), 300_000);
    }

    #[test]
    fn kills_a_command_that_runs_too_long() {
        let started = Instant::now();
        let result = output(Command::new("sleep").arg("30"), Duration::from_millis(100));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
