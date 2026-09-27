//! Orqadence: drives a beads Epic through the Pipeline.

pub mod cli;
pub(crate) mod orchestrator;
pub(crate) mod setup;
pub(crate) mod shell;
pub(crate) mod skills;
pub mod tools;
pub(crate) mod update;
pub mod version;

/// A scratch directory that is removed on Drop.
pub(crate) mod tempdir {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        /// A directory that did not exist before: one a killed or leaking
        /// process left under a pid now reused must not be reused with it,
        /// stale files and all.
        pub(crate) fn create() -> io::Result<Self> {
            static N: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "orqadence-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Ok(TempDir(path)),
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(err) => {
                        return Err(io::Error::new(
                            err.kind(),
                            format!("{}: {err}", path.display()),
                        ))
                    }
                }
            }
        }

        #[cfg(test)]
        pub(crate) fn new() -> Self {
            static SWEEP: std::sync::Once = std::sync::Once::new();
            SWEEP.call_once(sweep);
            Self::create().unwrap_or_else(|err| panic!("{err}"))
        }

        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Removes the dirs of processes now dead. A test whose World an Arc
    /// cycle or a still-running Ticket thread keeps alive never drops it, so
    /// its dirs outlive the process; the next run clears them.
    #[cfg(test)]
    fn sweep() {
        let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let pid = name
                .to_str()
                .and_then(|n| n.strip_prefix("orqadence-")?.split_once('-'))
                .filter(|(_, n)| n.parse::<u64>().is_ok())
                .and_then(|(pid, _)| pid.parse::<i32>().ok());
            if pid.is_some_and(|pid| pid > 0 && !running(pid)) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }

    /// Signal 0 sends nothing: kill only says whether the pid exists. Any
    /// answer but "no such process" (one of another user's is EPERM) is
    /// running.
    // ponytail: a pid from another pid namespace sharing this temp dir reads
    // as dead; hold a lock file per process if tests ever run that way.
    #[cfg(test)]
    fn running(pid: i32) -> bool {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        const ESRCH: i32 = 3;
        // SAFETY: kill with signal 0 only checks the pid; pid > 0 names one
        // process, never a group.
        let found = unsafe { kill(pid, 0) } == 0;
        found || std::io::Error::last_os_error().raw_os_error() != Some(ESRCH)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_sweep_removes_a_dead_pids_dirs_and_keeps_a_live_ones() {
            // past every pid_max: never a running process; this run's pid as
            // the counter keeps a concurrent run off the same dir
            let stale = std::env::temp_dir().join(format!(
                "orqadence-{}-{}",
                i32::MAX,
                std::process::id()
            ));
            std::fs::create_dir_all(stale.join(".orqadence")).unwrap();
            // a fresh dir of this process, removed by its own Drop
            let live = TempDir::create().unwrap();

            sweep();

            assert!(!stale.exists(), "{} is left", stale.display());
            assert!(live.path().exists(), "{} is gone", live.path().display());
        }
    }
}
