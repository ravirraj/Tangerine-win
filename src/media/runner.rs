//! Running a child process.
//!
//! The program is run with structured arguments and never through a shell,
//! so that nothing in a path can be read as syntax. The trait is what
//! makes the tests able to stand in for the real thing: a test can say
//! exactly what the program was asked to do, and what it answered.

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

/// What a finished process left behind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunResult {
    /// The exit code, if the process was run to the end.
    pub status: Option<i32>,
    /// What the process wrote to its standard output.
    pub stdout: Vec<u8>,
    /// What the process wrote to its standard error.
    pub stderr: String,
}

impl RunResult {
    /// Whether the process finished and said it was happy.
    pub fn succeeded(&self) -> bool {
        self.status == Some(0)
    }
}

/// Runs a program and waits for it.
pub trait ProcessRunner {
    /// Runs the program with these arguments.
    ///
    /// When `stdout_to` is given, the program's standard output is written
    /// into that file instead of being collected. The file is the one the
    /// caller made, so the caller keeps whatever ends up in it.
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        stdout_to: Option<&File>,
    ) -> io::Result<RunResult>;
}

/// Runs programs for real.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemRunner;

impl ProcessRunner for SystemRunner {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        stdout_to: Option<&File>,
    ) -> io::Result<RunResult> {
        let mut command = Command::new(program);
        command.args(args);
        // The program must never wait for anything typed at it.
        command.stdin(Stdio::null());

        match stdout_to {
            Some(file) => {
                // The child writes into the file the caller made. The
                // handle is shared, so the child writes where the caller
                // expects it.
                let handle = file.try_clone()?;
                command.stdout(Stdio::from(handle));
                command.stderr(Stdio::piped());
                let output = command.output()?;
                Ok(RunResult {
                    status: output.status.code(),
                    stdout: output.stdout,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                })
            }
            None => {
                command.stdout(Stdio::piped());
                command.stderr(Stdio::piped());
                let output = command.output()?;
                Ok(RunResult {
                    status: output.status.code(),
                    stdout: output.stdout,
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A runner that answers what it is told to answer.
    struct Scripted {
        result: RunResult,
        seen: std::cell::RefCell<Vec<OsString>>,
    }

    impl ProcessRunner for Scripted {
        fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            _stdout_to: Option<&File>,
        ) -> io::Result<RunResult> {
            self.seen.borrow_mut().extend(args.iter().cloned());
            Ok(self.result.clone())
        }
    }

    #[test]
    fn a_scripted_runner_records_what_it_was_asked() {
        let runner = Scripted {
            result: RunResult {
                status: Some(0),
                ..RunResult::default()
            },
            seen: std::cell::RefCell::new(Vec::new()),
        };
        let args = vec![OsString::from("-version")];
        let result = runner
            .run(Path::new("ffmpeg"), &args, None)
            .expect("the runner answered");
        assert!(result.succeeded());
        assert_eq!(&*runner.seen.borrow(), &args);
    }

    #[test]
    fn a_failure_is_not_a_success() {
        let runner = Scripted {
            result: RunResult {
                status: Some(1),
                ..RunResult::default()
            },
            seen: std::cell::RefCell::new(Vec::new()),
        };
        let result = runner
            .run(Path::new("ffmpeg"), &[], None)
            .expect("the runner answered");
        assert!(!result.succeeded());
    }
}
