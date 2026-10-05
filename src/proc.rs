//! Thin child-process interface, so hooks that spawn git / herdr / PowerShell are testable with fakes.

use std::io::Write;
use std::process::{Command, Stdio};

/// A child's stdout and exit code, relayed back to Claude Code unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct ChildAnswer {
    pub stdout: Vec<u8>,
    pub code: i32,
}

/// The ways the hooks run other programs.
pub trait ProcessRunner {
    /// Runs and waits, discarding all output (fire-and-report calls like herdr).
    fn run_quiet(&self, program: &str, args: &[String]);
    /// Runs with `stdin` piped in; `None` when the program cannot start.
    fn run_piped(&self, program: &str, args: &[String], stdin: &[u8]) -> Option<ChildAnswer>;
    /// Runs and returns trimmed stdout when the exit code is 0.
    fn capture(&self, program: &str, args: &[String]) -> Option<String>;
}

/// Spawns real processes, without a console window (hooks run behind the desktop app).
pub struct SystemProcessRunner;

impl ProcessRunner for SystemProcessRunner {
    fn run_quiet(&self, program: &str, args: &[String]) {
        let mut command = hidden_command(program, args);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = command.status();
    }

    fn run_piped(&self, program: &str, args: &[String], stdin: &[u8]) -> Option<ChildAnswer> {
        let mut command = hidden_command(program, args);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().ok()?;
        if let Some(mut pipe) = child.stdin.take() {
            let _ = pipe.write_all(stdin);
        }
        let output = child.wait_with_output().ok()?;
        let code = output.status.code().unwrap_or(0);
        Some(ChildAnswer {
            stdout: output.stdout,
            code,
        })
    }

    fn capture(&self, program: &str, args: &[String]) -> Option<String> {
        let mut command = hidden_command(program, args);
        command.stdin(Stdio::null()).stderr(Stdio::null());
        let output = command.output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

/// Builds a command that will not flash a console window on Windows.
///
/// Example: `hidden_command("git", &["--version".into()]).status()`
pub fn hidden_command(program: &str, args: &[String]) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
pub mod fakes {
    use super::{ChildAnswer, ProcessRunner};
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// One recorded call: program, args, stdin bytes.
    pub type RecordedCall = (String, Vec<String>, Vec<u8>);

    /// Records every call; `capture` answers come from a map keyed by "program arg arg".
    #[derive(Default)]
    pub struct RecordingProcessRunner {
        pub calls: RefCell<Vec<RecordedCall>>,
        pub captures: HashMap<String, String>,
        pub piped_answer: Option<ChildAnswer>,
    }

    impl RecordingProcessRunner {
        fn record(&self, program: &str, args: &[String], stdin: &[u8]) {
            let call = (program.to_string(), args.to_vec(), stdin.to_vec());
            self.calls.borrow_mut().push(call);
        }
    }

    impl ProcessRunner for RecordingProcessRunner {
        fn run_quiet(&self, program: &str, args: &[String]) {
            self.record(program, args, b"");
        }

        fn run_piped(&self, program: &str, args: &[String], stdin: &[u8]) -> Option<ChildAnswer> {
            self.record(program, args, stdin);
            self.piped_answer.clone()
        }

        fn capture(&self, program: &str, args: &[String]) -> Option<String> {
            self.record(program, args, b"");
            let key = format!("{program} {}", args.join(" "));
            self.captures.get(&key).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd_args(script: &str) -> Vec<String> {
        vec!["/C".into(), script.into()]
    }

    #[test]
    fn hidden_command_keeps_program_and_args() {
        let command = hidden_command("git", &["status".into()]);
        assert_eq!(command.get_program(), "git");
        assert_eq!(command.get_args().count(), 1);
    }

    #[test]
    fn capture_trims_stdout_and_fails_on_nonzero() {
        let runner = SystemProcessRunner;
        assert_eq!(
            runner.capture("cmd", &cmd_args("echo hi ")).as_deref(),
            Some("hi")
        );
        assert_eq!(runner.capture("cmd", &cmd_args("exit 3")), None);
        assert_eq!(runner.capture("no-such-program-xyz", &[]), None);
    }

    #[test]
    fn run_piped_relays_stdin_stdout_and_code() {
        let runner = SystemProcessRunner;
        let answer = runner.run_piped("cmd", &cmd_args("findstr x & exit 4"), b"x1\r\ny2\r\n");
        let answer = answer.unwrap();
        assert_eq!(answer.code, 4);
        assert_eq!(String::from_utf8_lossy(&answer.stdout).trim(), "x1");
    }

    #[test]
    fn run_quiet_survives_missing_program() {
        SystemProcessRunner.run_quiet("no-such-program-xyz", &[]);
    }
}
