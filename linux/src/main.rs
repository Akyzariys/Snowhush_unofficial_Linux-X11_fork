mod app;
mod light;
mod log;
mod prefs;
mod render;
mod screen;
mod session;
mod tracker;
mod tray;

use std::fs::{File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::PathBuf;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|argument| argument == "--version") {
        println!("Snowhush {} (Linux)", env!("CARGO_PKG_VERSION"));
        return;
    }
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        println!(
            "Snowhush {}\n\nSoft snow on your desktop.\n\nUsage: Snowhush [--version] [--uninstall]\n\nRuns in the notification area. The menu there controls the snow.",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    if arguments.iter().any(|argument| argument == "--uninstall") {
        prefs::uninstall();
        println!("Snowhush removed its settings and login entry.");
        return;
    }

    log::start();

    if std::env::var_os("DISPLAY").is_none() {
        let message =
            "Snowhush needs an X11 display. Start it in an X11 session, or set DISPLAY to one.";
        log!("{message}");
        fail(message);
    }
    if std::env::var("XDG_SESSION_TYPE").is_ok_and(|kind| kind.eq_ignore_ascii_case("wayland")) {
        let message = "Snowhush draws on the X11 desktop: a Wayland session does not let an app place snow on other windows.\n\
             Log in to an \"X11\" or \"Xorg\" session (Cinnamon, Xfce, KDE and GNOME all offer one) and start Snowhush there.";
        log!("{message}");
        fail(message);
    }

    let _instance = match lock() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            log!("another instance is already running");
            eprintln!("Snowhush is already running. Use the tray icon to change or quit it.");
            return;
        }
        Err(error) => {
            log!("could not take the instance lock: {error}");
            fail(&format!(
                "Snowhush couldn't start.\n\n{error}\n\nSee {}",
                log::path().map_or("the log".into(), |path| path.display().to_string())
            ));
        }
    };
    log::restart();
    log!("Snowhush {} (Linux)", env!("CARGO_PKG_VERSION"));
    if let Err(error) = app::run() {
        log!("failed: {error}");
        fail(&format!(
            "Snowhush couldn't start.\n\n{error}\n\nSee {}",
            log::path().map_or("the log".into(), |path| path.display().to_string())
        ));
    }
}

/// Only one copy may draw snow: later launches bow out so the tray icon stays singular.
///
/// `Ok(None)` means another instance holds the lock. `Err` means the lock itself could not be used;
/// reporting that as "already running" (as an earlier version did) hides a real startup failure.
fn lock() -> Result<Option<File>, String> {
    let uid = unsafe { libc::getuid() };
    let folder = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute() && folder.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    let path = folder.join(format!("snowhush-{uid}.lock"));
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| format!("opening {}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("inspecting {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.uid() != uid {
        return Err(format!(
            "{} is not a regular file owned by you",
            path.display()
        ));
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN)
        {
            return Ok(None);
        }
        return Err(format!("locking {}: {error}", path.display()));
    }
    file.set_len(0)
        .map_err(|error| format!("clearing {}: {error}", path.display()))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("rewinding {}: {error}", path.display()))?;
    writeln!(&file, "{}", std::process::id())
        .map_err(|error| format!("writing {}: {error}", path.display()))?;
    Ok(Some(file))
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}
