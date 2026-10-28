use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use snowhush::simulation::Snowfall;

/// What the tray writes down and the snow reads back at startup.
#[derive(Clone, Copy)]
pub struct Settings {
    pub snowfall: Snowfall,
    pub accumulate: bool,
    pub melt: bool,
    pub snow_on_windows: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        let calm = !animations_enabled();
        Settings {
            snowfall: if calm {
                Snowfall::Light
            } else {
                Snowfall::Steady
            },
            accumulate: true,
            melt: true,
            snow_on_windows: true,
        }
    }
}

impl Settings {
    pub fn load() -> Settings {
        let defaults = Settings::default();
        Settings {
            snowfall: read("snowfall")
                .and_then(|i| Snowfall::ALL.get(i as usize).copied())
                .unwrap_or(defaults.snowfall),
            accumulate: read("accumulate").is_none_or(|v| v != 0),
            melt: read("meltSnow").is_none_or(|v| v != 0),
            snow_on_windows: read("snowOnWindows").is_none_or(|v| v != 0),
        }
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(folder) = path.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let snowfall = Snowfall::ALL
            .iter()
            .position(|&s| s == self.snowfall)
            .unwrap_or(2);
        let body = format!(
            "# Snowhush settings. Written by the tray menu.\nsnowfall={snowfall}\naccumulate={}\nmeltSnow={}\nsnowOnWindows={}\n",
            self.accumulate as u8, self.melt as u8, self.snow_on_windows as u8
        );
        let _ = write_atomic(&path, body.as_bytes());
    }
}

static TEMP: AtomicU64 = AtomicU64::new(0);

/// Writes beside the destination and renames, so a crash or a full disk cannot leave a half-written
/// settings file, and an existing symlink at the destination is replaced rather than followed.
fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let Some(folder) = path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no parent",
        ));
    };
    std::fs::create_dir_all(folder)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snowhush");
    let mut last = None;
    for _ in 0..16 {
        let counter = TEMP.fetch_add(1, Ordering::Relaxed);
        let temp = folder.join(format!(".{name}.{}.{}.tmp", std::process::id(), counter));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)
        {
            Ok(mut file) => {
                let result = (|| {
                    file.write_all(body)?;
                    file.sync_all()?;
                    std::fs::rename(&temp, path)?;
                    Ok(())
                })();
                if result.is_err() {
                    let _ = std::fs::remove_file(&temp);
                }
                return result;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => last = Some(error),
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "temporary file already exists",
        )
    }))
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("snowhush").join("config"))
}

fn read(key: &str) -> Option<u32> {
    let text = std::fs::read_to_string(config_path()?).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            if name.trim() == key {
                return value.trim().parse().ok();
            }
        }
    }
    None
}

/// Removes the settings and the login entry, for `--uninstall`.
pub fn uninstall() {
    if let Some(path) = config_path() {
        if let Some(folder) = path.parent() {
            let _ = std::fs::remove_dir_all(folder);
        }
    }
    let _ = std::fs::remove_file(autostart_path());
}

/// `$XDG_CONFIG_HOME/autostart/snowhush.desktop`, or `~/.config/autostart/snowhush.desktop`.
fn autostart_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("autostart").join("snowhush.desktop")
}

pub fn opens_at_login() -> bool {
    let Ok(text) = std::fs::read_to_string(autostart_path()) else {
        return false;
    };
    let Ok(command) = command() else { return false };
    text.lines()
        .any(|line| line.trim() == format!("Exec={command}"))
}

fn command() -> Result<String, String> {
    let path =
        std::env::current_exe().map_err(|error| format!("finding Snowhush's path: {error}"))?;
    let path = path
        .to_str()
        .ok_or_else(|| "Snowhush's path is not valid UTF-8".to_string())?;
    exec_quote(path)
}

/// Quotes one executable path for a desktop entry's `Exec=` value. A lone `%` starts a field code,
/// so it must become `%%`; the other escapes follow the desktop-entry quoting rules.
fn exec_quote(path: &str) -> Result<String, String> {
    if path
        .chars()
        .any(|character| matches!(character, '\n' | '\r' | '\0'))
    {
        return Err("Snowhush's path contains a control character".into());
    }
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for character in path.chars() {
        match character {
            '"' | '\\' | '`' | '$' => {
                quoted.push('\\');
                quoted.push(character);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    Ok(quoted)
}

pub fn set_opens_at_login(on: bool) -> std::io::Result<()> {
    let path = autostart_path();
    if !on {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    let command =
        command().map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let body = format!(
        "[Desktop Entry]\nType=Application\nName=Snowhush\nComment=Soft snow on your desktop\nExec={command}\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    );
    write_atomic(&path, body.as_bytes())
}

/// The desktop's "Animation effects" setting: off means the person prefers less motion.
pub fn animations_enabled() -> bool {
    for schema in [
        "org.cinnamon.desktop.interface",
        "org.gnome.desktop.interface",
    ] {
        let Ok(output) = Command::new("gsettings")
            .args(["get", schema, "enable-animations"])
            .output()
        else {
            continue;
        };
        if !output.status.success() {
            continue;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        return !text.trim().eq_ignore_ascii_case("false");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::exec_quote;

    #[test]
    fn quotes_an_executable_for_a_desktop_entry() {
        assert_eq!(exec_quote("/opt/Snowhush").unwrap(), "\"/opt/Snowhush\"");
        assert_eq!(exec_quote("/tmp/a b%f").unwrap(), "\"/tmp/a b%%f\"",);
        assert_eq!(
            exec_quote("/tmp/a\"b\\c$d`e").unwrap(),
            "\"/tmp/a\\\"b\\\\c\\$d\\`e\"",
        );
        assert!(exec_quote("/tmp/a\nb").is_err());
    }
}
