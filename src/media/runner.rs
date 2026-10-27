//! Running a child program.
//!
//! The program is run with structured arguments and never through a shell,
//! so that nothing in a path can be read as syntax. The trait is what
//! makes the tests able to stand in for the real thing: a test can say
//! exactly what the program was asked to do, and what it answered.
//!
//! The real runner waits, but not for ever: every helper runs under a
//! cap. A helper that keeps standing after the cap is stood down, its
//! whole tree with it (the helper and every program it started belong
//! to a job of the run's own), and the run says it was stood down
//! rather than pretending the helper had no opinion. A wedged helper
//! can therefore never park its worker for ever.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    SetInformationJobObject, TerminateJobObject,
};
use std::os::windows::io::AsRawHandle;

/// How long any helper may keep standing before it is stood down.
///
/// A generous cap, not a pace: a real conversion of a big deck can take
/// minutes of honest work. The point is only that a wedged helper never
/// parks its worker for ever.
pub const RUN_CAP_MS: u64 = 600_000;

/// How often the wait looks at whether the helper is done yet.
const WAIT_STEP_MS: u64 = 50;

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

impl SystemRunner {
    /// Runs the program, but only for as long as the cap allows.
    ///
    /// The helper is put under a job of the run's own, so that standing
    /// it down stands every program it started down too, and so that
    /// this program dying takes the helper's tree with it. The piped
    /// sides are drained as the helper talks, the way the shell-less
    /// waiting before did, so a talkative helper cannot wedge itself on
    /// a full pipe while the wait stands on it.
    pub fn run_capped(
        &self,
        program: &Path,
        args: &[OsString],
        stdout_to: Option<&File>,
        cap_ms: u64,
    ) -> io::Result<RunResult> {
        let mut command = Command::new(program);
        command.args(args);
        // The program must never wait for anything typed at it.
        command.stdin(Stdio::null());

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        match stdout_to {
            Some(file) => {
                // The child writes into the file the caller made. The
                // handle is shared, so the child writes where the caller
                // expects it.
                let handle = file.try_clone()?;
                command.stdout(Stdio::from(handle));
                command.stderr(Stdio::piped());
            }
            None => {
                command.stdout(Stdio::piped());
                command.stderr(Stdio::piped());
            }
        }
        let mut child = command.spawn()?;

        // The tree belongs to this run.
        let job = match unsafe { under_a_job_of_its_own(&child) } {
            Ok(job) => Some(job),
            Err(_) => None,
        };

        // The sides the helper talks on are drained from beside, so
        // that no pipe can fill while the wait stands on the process.
        let stdout_piped = if stdout_to.is_none() {
            child.stdout.take()
        } else {
            None
        };
        let stderr_piped = child.stderr.take();
        let stdout_words = stdout_piped.map(|piped| {
            thread::spawn(move || {
                let mut whole = Vec::new();
                let mut piped = piped;
                let _ = piped.read_to_end(&mut whole);
                whole
            })
        });
        let stderr_words = stderr_piped.map(|piped| {
            thread::spawn(move || {
                let mut whole = Vec::new();
                let mut piped = piped;
                let _ = piped.read_to_end(&mut whole);
                String::from_utf8_lossy(&whole).into_owned()
            })
        });

        // The wait of the run: done is done, and the cap is the last
        // word.
        let deadline = Instant::now() + Duration::from_millis(cap_ms);
        let stood_down;
        loop {
            if child.try_wait()?.is_some() {
                stood_down = false;
                break;
            }
            if Instant::now() >= deadline {
                stood_down = true;
                match job {
                    Some(job) => unsafe {
                        let _ = TerminateJobObject(job, 1);
                    },
                    None => {
                        let _ = child.kill();
                        let _ = child.wait();
                    }
                }
                let _ = child.wait()?;
                break;
            }
            thread::sleep(Duration::from_millis(WAIT_STEP_MS));
        }
        let ended = child.wait()?;
        if let Some(job) = job {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(job);
            }
        }

        Ok(RunResult {
            // A helper the cap stood down did not end by its own word,
            // so the code it left behind is not said of it: the saying
            // says so instead.
            status: if stood_down { None } else { ended.code() },
            stdout: stdout_words
                .and_then(|words| words.join().ok())
                .unwrap_or_default(),
            stderr: {
                let mut noise = stderr_words
                    .and_then(|words| words.join().ok())
                    .unwrap_or_default();
                if stood_down {
                    noise.push_str(&format!(
                        "\nthe helper was stood down after {cap_ms} ms of standing\n"
                    ));
                }
                noise
            },
        })
    }
}

/// Puts a started program under a job of its own.
///
/// The job is made with kill-on-close: if this program dies while the
/// helper stands, the helper's tree is taken with it. Telling the job
/// apart from the other handles of the run: standing it down with
/// TerminateJobObject ends the helper and every program it started.
unsafe fn under_a_job_of_its_own(child: &Child) -> io::Result<HANDLE> {
    let job = unsafe {
        CreateJobObjectW(None, None)
    }
    .map_err(|_| io::Error::other("the job was not made"))?;
    let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    }
    .map_err(|_| io::Error::other("the job was not told its limits"))?;
    unsafe {
        AssignProcessToJobObject(job, HANDLE(child.as_raw_handle() as _))
    }
    .map_err(|_| {
        io::Error::other("the helper could not be put under the job")
    })?;
    Ok(job)
}

impl ProcessRunner for SystemRunner {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        stdout_to: Option<&File>,
    ) -> io::Result<RunResult> {
        self.run_capped(program, args, stdout_to, RUN_CAP_MS)
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

    /// A helper that keeps standing past its cap is stood down: it does
    /// not finish, and the saying says what stood it down.
    #[test]
    fn a_helper_beyond_its_cap_is_stood_down() {
        let args = vec![
            OsString::from("/c"),
            OsString::from("ping"),
            OsString::from("-n"),
            OsString::from("30"),
            OsString::from("127.0.0.1"),
        ];
        let made = SystemRunner
            .run_capped(Path::new("cmd"), &args, None, 500)
            .expect("the cap said something");
        assert_eq!(made.status, None, "a stood-down helper did not end");
        assert!(
            made.stderr.contains("stood down"),
            "the saying says why: {}",
            made.stderr
        );
    }

    /// A helper that finishes in its own time is left whole: a cap is
    /// not a pace.
    #[test]
    fn a_helper_inside_its_cap_stands_whole() {
        let args = vec![
            OsString::from("/c"),
            OsString::from("ping"),
            OsString::from("-n"),
            OsString::from("1"),
            OsString::from("127.0.0.1"),
        ];
        let made = SystemRunner
            .run_capped(Path::new("cmd"), &args, None, 30_000)
            .expect("the run said something");
        assert_eq!(made.status, Some(0), "the quick helper finished: {made:?}");
    }
}
