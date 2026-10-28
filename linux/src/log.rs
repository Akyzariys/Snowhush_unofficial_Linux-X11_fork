use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

static LOG: Mutex<Option<(File, Instant)>> = Mutex::new(None);

/// `$XDG_STATE_HOME/snowhush/snowhush.log`, or `~/.local/state/snowhush/snowhush.log`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })?;
    Some(base.join("snowhush").join("snowhush.log"))
}

/// Opens the log for appending. A second launch must not truncate the log of the instance that is
/// already running; the primary instance clears it with [`restart`] only after taking its lock.
pub fn start() {
    let Some(path) = path() else { return };
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path);
    if let Ok(file) = file {
        if let Ok(mut log) = LOG.lock() {
            *log = Some((file, Instant::now()));
        }
    }
}

/// Starts a fresh log for this run. Called only after the single-instance lock is held, so no other
/// live process can have the file open for writing.
pub fn restart() {
    if let Ok(mut log) = LOG.lock() {
        if let Some((file, started)) = log.as_mut() {
            let _ = file.set_len(0);
            let _ = file.seek(SeekFrom::Start(0));
            *started = Instant::now();
        }
    }
}

pub fn line(arguments: std::fmt::Arguments) {
    if let Ok(mut log) = LOG.lock() {
        if let Some((file, started)) = log.as_mut() {
            let _ = writeln!(
                file,
                "{:>9.2}  {}",
                started.elapsed().as_secs_f64(),
                arguments
            );
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($argument:tt)*) => { $crate::log::line(format_args!($($argument)*)) };
}
