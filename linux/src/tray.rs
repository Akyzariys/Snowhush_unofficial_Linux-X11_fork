use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::{CheckmarkItem, MenuItem, RadioGroup, RadioItem, StandardItem, SubMenu};
use ksni::{Icon, ToolTip};
use winit::event_loop::EventLoopProxy;

use snowhush::simulation::Snowfall;

use crate::app::Command;
use crate::log;
use crate::prefs::{self, Settings};

/// The panel icon and its menu. It runs on its own thread so the snow never waits for a menu.
pub struct Tray {
    settings: Settings,
    proxy: EventLoopProxy<Command>,
    icon: Option<Icon>,
}

impl Tray {
    fn send(&self, command: Command) {
        let _ = self.proxy.send_event(command);
    }

    fn remember(&mut self, command: Command) {
        self.settings.save();
        self.send(command);
    }
}

impl ksni::Tray for Tray {
    // Left-clicking the panel icon should open the menu, as it does on the other platforms.
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "snowhush".into()
    }

    fn title(&self) -> String {
        "Snowhush".into()
    }

    fn icon_name(&self) -> String {
        "weather-snow".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone().into_iter().collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Snowhush".into(),
            description: "Soft snow on your desktop".into(),
            icon_name: "weather-snow".into(),
            icon_pixmap: self.icon.clone().into_iter().collect(),
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            MenuItem::SubMenu(SubMenu {
                label: "Snowfall".into(),
                submenu: vec![MenuItem::RadioGroup(RadioGroup {
                    selected: Snowfall::ALL
                        .iter()
                        .position(|&snowfall| snowfall == self.settings.snowfall)
                        .unwrap_or(2),
                    select: Box::new(|tray: &mut Tray, index| {
                        let Some(&snowfall) = Snowfall::ALL.get(index) else {
                            return;
                        };
                        tray.settings.snowfall = snowfall;
                        tray.remember(Command::Snowfall(snowfall));
                    }),
                    options: Snowfall::ALL
                        .iter()
                        .map(|snowfall| RadioItem {
                            label: snowfall.title().into(),
                            ..Default::default()
                        })
                        .collect(),
                })],
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Checkmark(CheckmarkItem {
                label: "Accumulate".into(),
                checked: self.settings.accumulate,
                activate: Box::new(|tray: &mut Tray| {
                    let on = !tray.settings.accumulate;
                    tray.settings.accumulate = on;
                    tray.remember(Command::Accumulate(on));
                }),
                ..Default::default()
            }),
            MenuItem::Checkmark(CheckmarkItem {
                label: "Melt Snow".into(),
                checked: self.settings.melt,
                enabled: self.settings.accumulate,
                activate: Box::new(|tray: &mut Tray| {
                    let on = !tray.settings.melt;
                    tray.settings.melt = on;
                    tray.remember(Command::Melt(on));
                }),
                ..Default::default()
            }),
            MenuItem::Checkmark(CheckmarkItem {
                label: "Snow on Windows".into(),
                checked: self.settings.snow_on_windows,
                activate: Box::new(|tray: &mut Tray| {
                    let on = !tray.settings.snow_on_windows;
                    tray.settings.snow_on_windows = on;
                    tray.remember(Command::SnowOnWindows(on));
                }),
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: "Clear Snow".into(),
                activate: Box::new(|tray: &mut Tray| {
                    tray.send(Command::ClearSnow(!prefs::animations_enabled()));
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Checkmark(CheckmarkItem {
                label: "Open at Login".into(),
                checked: prefs::opens_at_login(),
                activate: Box::new(|_tray: &mut Tray| {
                    let on = !prefs::opens_at_login();
                    match prefs::set_opens_at_login(on) {
                        Ok(()) => log!("open at login: {on}"),
                        Err(error) => log!("open at login failed: {error}"),
                    }
                }),
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Tray| tray.send(Command::Quit)),
                ..Default::default()
            }),
        ]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        // A left click shows the menu; there is no separate window to open.
    }
}

/// Starts the tray on its own thread and returns a handle that can rebuild the menu.
pub fn start(settings: Settings, proxy: EventLoopProxy<Command>) -> Option<Handle<Tray>> {
    let tray = Tray {
        settings,
        proxy,
        icon: icon(),
    };
    // The panel's StatusNotifier watcher is often not ready yet when an autostart entry runs.
    // Assume it will appear instead of giving up on the tray for the rest of the session.
    match tray.assume_sni_available(true).spawn() {
        Ok(handle) => Some(handle),
        Err(error) => {
            log!("tray stopped: {error}");
            eprintln!("Snowhush: the tray icon could not be shown: {error}");
            None
        }
    }
}

/// The app icon, in the ARGB32 form the status notifier asks for.
fn icon() -> Option<Icon> {
    let data = include_bytes!("../../site/assets/icon-64.png");
    let decoder = png::Decoder::new(&data[..]);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let mut argb = Vec::with_capacity((info.width * info.height * 4) as usize);
    match info.color_type {
        png::ColorType::Rgba => {
            for pixel in pixels.chunks_exact(4) {
                argb.extend_from_slice(&[pixel[3], pixel[0], pixel[1], pixel[2]]);
            }
        }
        png::ColorType::Rgb => {
            for pixel in pixels.chunks_exact(3) {
                argb.extend_from_slice(&[255, pixel[0], pixel[1], pixel[2]]);
            }
        }
        _ => return None,
    }
    Some(Icon {
        width: info.width as i32,
        height: info.height as i32,
        data: argb,
    })
}
