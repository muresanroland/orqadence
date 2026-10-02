//! Tools is the one seam to every external tool (herdr, bd, gh, git, claude).

use std::fmt;
use std::io::Read;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// A failed command: `Display` is "<command>: <status>: <stderr>".
#[derive(Debug, Clone, PartialEq)]
pub struct RunError {
    /// The command line, space-joined.
    pub command: String,
    /// "exit status N", or why it could not run.
    pub status: String,
    /// Trimmed stderr.
    pub stderr: String,
    /// What it printed before it failed: claude -p prints its error there.
    pub stdout: String,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}: {}", self.command, self.status, self.stderr)
    }
}

impl std::error::Error for RunError {}

/// The space-joined command line of a failure, with the TypeSafe key redacted:
/// the Debate pane takes it as --env, and a failed spawn lands in the log.
fn command_line(argv: &[&str]) -> String {
    argv.iter()
        .map(|arg| match arg.strip_prefix("TYPESAFE_API_KEY=") {
            Some(_) => "TYPESAFE_API_KEY=***",
            None => arg,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Runs argv in dir and returns stdout; a failure carries stderr.
pub trait Tools: Send + Sync {
    fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError>;

    /// `run`, killed and failed once it outlasts `limit`. Only Exec enforces
    /// the limit; a fake answers at once.
    fn run_within(&self, dir: &Path, argv: &[&str], limit: Duration) -> Result<String, RunError> {
        let _ = limit;
        self.run(dir, argv)
    }
}

fn no_command() -> RunError {
    RunError {
        command: String::new(),
        status: "no command".to_string(),
        stderr: String::new(),
        stdout: String::new(),
    }
}

/// stdout on success, else a RunError with the exit status and stderr.
fn finish(
    command: String,
    status: ExitStatus,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<String, RunError> {
    let stdout = String::from_utf8_lossy(stdout).into_owned();
    if !status.success() {
        let status = match status.code() {
            Some(code) => format!("exit status {code}"),
            None => status.to_string(),
        };
        return Err(RunError {
            command,
            status,
            stderr: String::from_utf8_lossy(stderr).trim().to_string(),
            stdout,
        });
    }
    Ok(stdout)
}

/// The real Tools.
pub struct Exec;

impl Tools for Exec {
    fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError> {
        let [name, args @ ..] = argv else {
            return Err(no_command());
        };
        let command = command_line(argv);
        let output = Command::new(name)
            .args(args)
            .current_dir(dir)
            .output()
            .map_err(|err| RunError {
                command: command.clone(),
                status: err.to_string(),
                stderr: String::new(),
                stdout: String::new(),
            })?;
        finish(command, output.status, &output.stdout, &output.stderr)
    }

    fn run_within(&self, dir: &Path, argv: &[&str], limit: Duration) -> Result<String, RunError> {
        let [name, args @ ..] = argv else {
            return Err(no_command());
        };
        let command = command_line(argv);
        let fail = |status: String| RunError {
            command: command.clone(),
            status,
            stderr: String::new(),
            stdout: String::new(),
        };
        let mut cmd = Command::new(name);
        cmd.args(args)
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Its own process group, so the command's descendants can be killed
        // with it and never hold the pipes open past the limit.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn().map_err(|err| fail(err.to_string()))?;
        // Read both pipes on threads so a chatty command never blocks on a
        // full pipe while it is polled.
        let drain = |pipe: Option<Box<dyn Read + Send>>| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut buf);
                }
                buf
            })
        };
        let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
        let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
        let deadline = Instant::now() + limit;
        let status = loop {
            match child.try_wait().map_err(|err| fail(err.to_string()))? {
                Some(status) => break status,
                None if Instant::now() >= deadline => {
                    kill_group(child.id());
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(fail(format!("timed out after {}s", limit.as_secs_f32())));
                }
                None => thread::sleep(Duration::from_millis(50)),
            }
        };
        // A descendant left running would keep the readers blocked.
        kill_group(child.id());
        let stdout = stdout.join().unwrap_or_default();
        let stderr = stderr.join().unwrap_or_default();
        finish(command, status, &stdout, &stderr)
    }
}

/// Runs an editor on a file in the terminal and waits for it: the second
/// seam beside Tools, since an editor needs the terminal and Tools captures
/// its output. The error is the warning, "quit unexpectedly (exit code 1)".
pub trait Editor: Send + Sync {
    fn edit(&self, editor: &str, file: &Path) -> Result<(), String>;
}

impl Editor for Exec {
    /// Through sh, as git does, so an editor with arguments or a quoted
    /// path runs as the user wrote it.
    fn edit(&self, editor: &str, file: &Path) -> Result<(), String> {
        let status = Command::new("sh")
            .args(["-c", &format!("{editor} \"$@\""), editor])
            .arg(file)
            .status()
            .map_err(|err| format!("could not run: {err}"))?;
        match status.code() {
            Some(0) => Ok(()),
            Some(code) => Err(format!("quit unexpectedly (exit code {code})")),
            None => Err(format!("quit unexpectedly ({status})")),
        }
    }
}

/// The editor to run: $VISUAL, then $EDITOR (empty counts as unset), else
/// the first of code -w, vi and nano on PATH; a bare code or subl gets its
/// wait flag, or it would return before the file is edited.
pub(crate) fn editor(env: &dyn Fn(&str) -> String) -> Option<String> {
    let set = ["VISUAL", "EDITOR"]
        .map(|var| env(var).trim().to_string())
        .into_iter()
        .find(|e| !e.is_empty());
    if let Some(e) = set {
        return Some(match e.as_str() {
            "code" | "subl" => format!("{e} -w"),
            _ => e,
        });
    }
    let path = env("PATH");
    let on_path = |name: &str| {
        (path.split(':').filter(|dir| !dir.is_empty()))
            .any(|dir| Path::new(dir).join(name).is_file())
    };
    ["code -w", "vi", "nano"]
        .into_iter()
        .find(|e| on_path(e.split(' ').next().unwrap_or(e)))
        .map(String::from)
}

/// Kills the process group `pgid` leads; best effort, a no-op off unix.
fn kill_group(pgid: u32) {
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(not(unix))]
    let _ = pgid;
}

/// The test double for the Tools seam.
#[cfg(test)]
pub(crate) mod fake {
    use super::{RunError, Tools};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Answers a call with stdout, or with the stderr of a failure.
    pub(crate) type Handle = dyn Fn(&Path, &[&str]) -> Result<String, String> + Send + Sync;

    /// Records every call and answers it from `handle`; no handle returns ""
    /// with no error.
    pub(crate) struct Fake {
        calls: Mutex<Vec<String>>,
        handle: Option<Box<Handle>>,
    }

    impl Fake {
        pub(crate) fn new(
            handle: impl Fn(&Path, &[&str]) -> Result<String, String> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(Fake {
                calls: Mutex::new(Vec::new()),
                handle: Some(Box::new(handle)),
            })
        }

        pub(crate) fn quiet() -> Arc<Self> {
            Arc::new(Fake {
                calls: Mutex::new(Vec::new()),
                handle: None,
            })
        }

        /// Every call so far, in order, as space-joined command lines.
        pub(crate) fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    /// What a fake editor does to the file it is given.
    pub(crate) type EditHandle = dyn Fn(&Path) -> Result<(), String> + Send + Sync;

    /// The test double for the Editor seam: records each editor and file,
    /// and answers from `handle`; none leaves the file as it is.
    #[derive(Default)]
    pub(crate) struct FakeEditor {
        calls: Mutex<Vec<(String, std::path::PathBuf)>>,
        handle: Option<Box<EditHandle>>,
    }

    impl FakeEditor {
        pub(crate) fn new(
            handle: impl Fn(&Path) -> Result<(), String> + Send + Sync + 'static,
        ) -> Arc<Self> {
            Arc::new(FakeEditor {
                calls: Mutex::default(),
                handle: Some(Box::new(handle)),
            })
        }

        /// Every editor run so far and the file it was given.
        pub(crate) fn calls(&self) -> Vec<(String, std::path::PathBuf)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl super::Editor for FakeEditor {
        fn edit(&self, editor: &str, file: &Path) -> Result<(), String> {
            let call = (editor.to_string(), file.to_path_buf());
            self.calls.lock().unwrap().push(call);
            self.handle.as_ref().map_or(Ok(()), |handle| handle(file))
        }
    }

    impl Tools for Fake {
        fn run(&self, dir: &Path, argv: &[&str]) -> Result<String, RunError> {
            self.calls.lock().unwrap().push(argv.join(" "));
            match &self.handle {
                None => Ok(String::new()),
                Some(handle) => handle(dir, argv).map_err(|stderr| RunError {
                    command: super::command_line(argv),
                    status: "exit status 1".to_string(),
                    stderr,
                    stdout: String::new(),
                }),
            }
        }

        /// Records the call with its bound, so a test sees the limit kept.
        fn run_within(
            &self,
            dir: &Path,
            argv: &[&str],
            limit: std::time::Duration,
        ) -> Result<String, RunError> {
            let result = self.run(dir, argv);
            let mut calls = self.calls.lock().unwrap();
            if let Some(last) = calls.last_mut() {
                last.push_str(&format!(" [within {}s]", limit.as_secs()));
            }
            result
        }
    }
}

#[cfg(test)]
mod tools_test;
