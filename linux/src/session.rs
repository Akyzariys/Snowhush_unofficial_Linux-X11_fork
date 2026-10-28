use std::thread;
use std::time::Duration;

use winit::event_loop::EventLoopProxy;

use crate::app::Command;
use crate::log;
use crate::screen::PowerBudget;

/// Pauses the snow while the screen is locked, and starts it again on unlock.
/// Polls the desktop's screen saver, since X11 has no session-change broadcast of its own.
pub fn watch(proxy: EventLoopProxy<Command>, initially_locked: bool) {
    let _ = thread::Builder::new()
        .name("session".into())
        .spawn(move || {
            let connection = match dbus::blocking::Connection::new_session() {
                Ok(connection) => connection,
                Err(error) => {
                    log!("session bus unavailable, lock detection is off: {error}");
                    return;
                }
            };
            let mut locked = initially_locked;
            loop {
                let now = screensaver_active(&connection);
                if now != locked {
                    locked = now;
                    let _ = proxy.send_event(Command::Active(!locked));
                }
                thread::sleep(Duration::from_secs(2));
            }
        });
}

/// The lock state at startup, so the first frame is not drawn over a locked screen.
pub fn locked() -> bool {
    let Ok(connection) = dbus::blocking::Connection::new_session() else {
        return false;
    };
    screensaver_active(&connection)
}

fn screensaver_active(connection: &dbus::blocking::Connection) -> bool {
    for (service, path) in [
        ("org.cinnamon.ScreenSaver", "/org/cinnamon/ScreenSaver"),
        ("org.gnome.ScreenSaver", "/org/gnome/ScreenSaver"),
        (
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
        ),
    ] {
        let proxy = connection.with_proxy(service, path, Duration::from_millis(800));
        let result: Result<(bool,), dbus::Error> = proxy.method_call(service, "GetActive", ());
        if let Ok((active,)) = result {
            return active;
        }
    }
    false
}

/// Reads the power supply: on battery, frames are capped at 60, which is hard to tell apart in snow.
pub fn power_budget() -> PowerBudget {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else {
        return PowerBudget::Full;
    };
    let mut battery_discharging = false;
    let mut on_mains = false;
    for entry in entries.flatten() {
        let path = entry.path();
        let kind = read(&path.join("type"));
        match kind.as_str() {
            "Mains" | "USB" | "USB_C" | "AC" | "ACAD" => {
                if read(&path.join("online")) == "1" {
                    on_mains = true;
                }
            }
            "Battery" if read(&path.join("status")).eq_ignore_ascii_case("discharging") => {
                battery_discharging = true;
            }
            _ => {}
        }
    }
    if battery_discharging && !on_mains {
        PowerBudget::Battery
    } else {
        PowerBudget::Full
    }
}

fn read(path: &std::path::Path) -> String {
    std::fs::read_to_string(path)
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}
