use std::cell::Cell;
use std::collections::HashSet;
use std::mem::size_of;
use std::sync::Once;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::Graphics::Gdi::{
    DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{BOOL, PCWSTR, Result, w};

use snowhush::geometry::V2;
use snowhush::simulation::{Pace, SnowSimulation, WindowId, WindowSurface};

use crate::light::{Backdrop, Wallpapers};
use crate::log;
use crate::render::{Ambient, Gpu, Surface};
use crate::tracker::Tracker;

const CLASS: PCWSTR = w!("SnowhushOverlay");

thread_local! {
    /// Set by an overlay window when the displays change under it.
    pub static DISPLAYS_CHANGED: Cell<bool> = const { Cell::new(false) };
    /// Set by an overlay window when the desktop picture or colour changes.
    pub static BACKDROP_CHANGED: Cell<bool> = const { Cell::new(false) };
}

/// How much the snow may spend on drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerBudget {
    Full,
    /// On battery: no frame rates above 60, which are hard to tell apart in snow anyway.
    Battery,
    /// Energy saver: a little less snow, drawn a little less often.
    Saver,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Monitor {
    pub name: String,
    /// The work area in physical pixels: the taskbar is the ground the snow settles on, so it is left out.
    pub rect: RECT,
    /// The whole display in physical pixels.
    pub bounds: RECT,
    pub dpi: u32,
    pub refresh: u32,
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, list: LPARAM) -> BOOL {
        let list = unsafe { &mut *(list.0 as *mut Vec<HMONITOR>) };
        list.push(monitor);
        true.into()
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&raw mut handles as isize));
    }
    handles
        .into_iter()
        .filter_map(|handle| unsafe {
            let mut info = MONITORINFOEXW::default();
            info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
            if !GetMonitorInfoW(handle, (&raw mut info).cast::<MONITORINFO>()).as_bool() {
                return None;
            }
            let (mut dpi, mut _y) = (96, 96);
            let _ = GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi, &mut _y);
            let mut mode = DEVMODEW { dmSize: size_of::<DEVMODEW>() as u16, ..Default::default() };
            let refresh = if EnumDisplaySettingsW(PCWSTR(info.szDevice.as_ptr()), ENUM_CURRENT_SETTINGS, &mut mode).as_bool() {
                mode.dmDisplayFrequency
            } else {
                60
            };
            let length = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
            Some(Monitor {
                name: String::from_utf16_lossy(&info.szDevice[..length]),
                rect: info.monitorInfo.rcWork,
                bounds: info.monitorInfo.rcMonitor,
                dpi,
                refresh,
            })
        })
        .collect()
}

/// The display's vertical blanks on the performance counter: frames are started just after one, so they keep pace with it.
pub struct Clock {
    pub frequency: i64,
    period: i64,
    phase: i64,
    hz: f64,
}

impl Clock {
    pub fn new() -> Clock {
        let mut frequency = 0;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut frequency);
        }
        let mut clock = Clock { frequency, period: frequency / 60, phase: 0, hz: 60.0 };
        clock.refresh();
        clock
    }

    pub fn now() -> i64 {
        let mut now = 0;
        unsafe {
            let _ = QueryPerformanceCounter(&mut now);
        }
        now
    }

    pub fn refresh(&mut self) {
        let mut info = DWM_TIMING_INFO { cbSize: size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
        if unsafe { DwmGetCompositionTimingInfo(HWND::default(), &mut info) }.is_err() || info.qpcRefreshPeriod == 0 {
            return;
        }
        self.period = info.qpcRefreshPeriod as i64;
        self.phase = info.qpcVBlank as i64;
        let ratio = info.rateRefresh;
        self.hz = if ratio.uiDenominator > 0 { ratio.uiNumerator as f64 / ratio.uiDenominator as f64 } else { 60.0 };
    }

    /// When to start the frame after one started at `now`, at `rate` frames per second.
    fn schedule(&self, now: i64, rate: u32) -> i64 {
        let divisor = (self.hz / rate as f64).round().max(1.0) as i64;
        let earliest = now + divisor * self.period - self.period / 2;
        let vblanks = (earliest - self.phase + self.period - 1).div_euclid(self.period);
        self.phase + vblanks * self.period + self.frequency / 2000
    }

    pub fn seconds(&self, ticks: i64) -> f64 {
        ticks as f64 / self.frequency as f64
    }

    pub fn tolerance(&self) -> i64 {
        self.period / 4
    }
}

struct Overlay(HWND);

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// The snow on one display: its overlay window, drawing surface and simulation.
pub struct Screen {
    pub monitor: Monitor,
    pub simulation: SnowSimulation,
    pub next_due: i64,
    surface: Surface,
    overlay: Overlay,
    size: (u32, u32),
    last_step: i64,
    idle_frames: u32,
    interactive_rate: u32,
    surfaces: Vec<WindowSurface>,
    windows_seen: u64,
    windows_changed_at: i64,
    covered: bool,
    backdrop: Option<Backdrop>,
    shadow_tint: [f32; 4],
}

impl Screen {
    pub fn new(gpu: &Gpu, monitor: Monitor, simulation: SnowSimulation, shown_in_screenshots: bool) -> Result<Screen> {
        let (physical_width, physical_height) = physical_size(&monitor);
        let size = dip_size(&monitor);
        let overlay = Overlay(create_overlay(&monitor.rect, shown_in_screenshots)?);
        let scale = (physical_width as f32 / size.0 as f32, physical_height as f32 / size.1 as f32);
        let surface = Surface::new(gpu, overlay.0, size.0, size.1, scale)?;
        Ok(Screen {
            interactive_rate: monitor.refresh.clamp(60, 120),
            monitor,
            simulation,
            next_due: 0,
            surface,
            overlay,
            size,
            last_step: 0,
            idle_frames: 0,
            surfaces: Vec::new(),
            windows_seen: 0,
            windows_changed_at: 0,
            covered: false,
            backdrop: None,
            shadow_tint: [0.0; 4],
        })
    }

    /// Steps the snow and draws it if anything visible changed; returns whether it drew.
    /// `windows` is left out while snow on windows is off.
    pub fn frame(
        &mut self,
        gpu: &Gpu,
        clock: &Clock,
        now: i64,
        budget: PowerBudget,
        light: [f32; 3],
        time: f32,
        windows: Option<&Tracker>,
    ) -> Result<bool> {
        let dt = if self.last_step == 0 { 1.0 / 60.0 } else { clock.seconds(now - self.last_step).min(0.05) as f32 };
        self.last_step = now;
        if let Some(tracker) = windows {
            self.follow(tracker, clock, now);
        }
        let changed = self.simulation.step(dt, self.cursor());

        let fastest = if budget == PowerBudget::Full { self.interactive_rate } else { self.interactive_rate.min(60) };
        // 24 only where it divides the display's rate evenly; elsewhere it would judder.
        let calm = if budget == PowerBudget::Saver && self.interactive_rate % 24 == 0 { 24 } else { 30 };
        let rate = match self.simulation.pace() {
            Pace::Calm => calm,
            Pace::Lively => fastest.min(60),
            Pace::Interactive => fastest,
        };
        self.next_due = clock.schedule(now, rate);
        self.idle_frames = if changed { 0 } else { self.idle_frames + 1 };
        // A still pile only needs a slow refresh for its glints.
        if !(changed || (self.simulation.has_snow() && self.idle_frames % 2 == 0)) {
            return Ok(false);
        }
        self.surface.draw(gpu, &self.simulation, Ambient { light, shadow_tint: self.shadow_tint }, time)?;
        Ok(true)
    }

    /// Hands the simulation the windows on this display when they changed.
    fn follow(&mut self, tracker: &Tracker, clock: &Clock, now: i64) {
        if tracker.generation == self.windows_seen {
            return;
        }
        self.windows_seen = tracker.generation;
        let surfaces = tracker.surfaces(&self.monitor.rect, self.size);
        if surfaces == self.surfaces {
            return;
        }
        let since = clock.seconds(now - self.windows_changed_at) as f32;
        // A window that moves outside a drag, as when it is maximized or snapped, jumps there in one go:
        // its snow is left behind instead of being carried along.
        let interval = if tracker.moving() || since < 0.1 { since.min(0.25) } else { 1.0 / 60.0 };
        let elsewhere: HashSet<WindowId> = self
            .surfaces
            .iter()
            .map(|surface| surface.id)
            .filter(|&id| !surfaces.iter().any(|surface| surface.id == id) && tracker.elsewhere(id))
            .collect();
        self.simulation.update_windows(&surfaces, interval, &HashSet::new(), &elsewhere, false);
        self.surfaces = surfaces;
        self.windows_changed_at = now;
    }

    pub fn drop_window_snow(&mut self) {
        self.simulation.update_windows(&[], 0.1, &HashSet::new(), &HashSet::new(), false);
        self.surfaces.clear();
        self.windows_seen = 0;
    }

    /// Hides the snow while a full-screen app covers the display, and stops it, as it stops over a full-screen app on a Mac.
    pub fn set_covered(&mut self, covered: bool) {
        if covered == self.covered {
            return;
        }
        self.covered = covered;
        unsafe {
            let _ = ShowWindow(self.overlay.0, if covered { SW_HIDE } else { SW_SHOWNOACTIVATE });
        }
        if !covered {
            self.resume();
        }
    }

    pub fn covered(&self) -> bool {
        self.covered
    }

    /// Picks up the shadow colour again if the desktop picture changed.
    pub fn update_backdrop(&mut self, wallpapers: &Wallpapers) {
        let backdrop = wallpapers.backdrop(&self.monitor.bounds);
        if backdrop == self.backdrop {
            return;
        }
        self.shadow_tint = backdrop.as_ref().and_then(|backdrop| wallpapers.tint(backdrop)).unwrap_or([0.0; 4]);
        log!("backdrop of {}: {backdrop:?}, shadow {:?}", self.monitor.name, self.shadow_tint);
        self.backdrop = backdrop;
    }

    pub fn wake(&mut self, now: i64) {
        self.next_due = self.next_due.min(now);
    }

    pub fn resume(&mut self) {
        self.last_step = 0;
        self.next_due = 0;
    }

    pub fn show_in_screenshots(&self, shown: bool) {
        set_capture(self.overlay.0, shown);
    }

    /// Topmost windows opened later stack above; this puts the snow back on top of them.
    pub fn raise(&self) {
        unsafe {
            let _ = SetWindowPos(self.overlay.0, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }

    /// The pointer in overlay coordinates.
    fn cursor(&self) -> V2 {
        let mut point = POINT::default();
        if unsafe { GetCursorPos(&mut point) }.is_err() {
            return V2::new(-10_000.0, -10_000.0);
        }
        self.local(point.x as f32, point.y as f32)
    }

    /// A point in physical screen pixels in overlay coordinates: DIPs up from the bottom left of this display's work area.
    pub fn local(&self, x: f32, y: f32) -> V2 {
        let rect = &self.monitor.rect;
        let (width, height) = physical_size(&self.monitor);
        V2::new((x - rect.left as f32) * self.size.0 as f32 / width as f32, (rect.bottom as f32 - y) * self.size.1 as f32 / height as f32)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        let bounds = &self.monitor.bounds;
        x >= bounds.left as f32 && x < bounds.right as f32 && y >= bounds.top as f32 && y < bounds.bottom as f32
    }
}

pub fn dip_size(monitor: &Monitor) -> (u32, u32) {
    let (width, height) = physical_size(monitor);
    let scale = monitor.dpi.max(96) as f32 / 96.0;
    ((width as f32 / scale).ceil() as u32, (height as f32 / scale).ceil() as u32)
}

fn physical_size(monitor: &Monitor) -> (i32, i32) {
    let rect = &monitor.rect;
    ((rect.right - rect.left).max(1), (rect.bottom - rect.top).max(1))
}

fn create_overlay(rect: &RECT, shown_in_screenshots: bool) -> Result<HWND> {
    static REGISTER: Once = Once::new();
    unsafe {
        let instance = GetModuleHandleW(None)?;
        REGISTER.call_once(|| {
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(overlay_procedure),
                hInstance: instance.into(),
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class);
        });
        let window = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            CLASS,
            w!("Snowhush"),
            WS_POPUP,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        // A layered window stays invisible until it is given an opacity; WS_EX_TRANSPARENT then lets clicks through.
        SetLayeredWindowAttributes(window, COLORREF(0), 255, LWA_ALPHA)?;
        set_capture(window, shown_in_screenshots);
        let _ = ShowWindow(window, SW_SHOWNOACTIVATE);
        Ok(window)
    }
}

fn set_capture(window: HWND, shown: bool) {
    unsafe {
        let _ = SetWindowDisplayAffinity(window, if shown { WDA_NONE } else { WDA_EXCLUDEFROMCAPTURE });
    }
}

unsafe extern "system" fn overlay_procedure(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_DISPLAYCHANGE | WM_DPICHANGED => {
            DISPLAYS_CHANGED.set(true);
            LRESULT(0)
        }
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETWORKAREA.0 => {
            DISPLAYS_CHANGED.set(true);
            LRESULT(0)
        }
        WM_SETTINGCHANGE if wparam.0 as u32 == SPI_SETDESKWALLPAPER.0 => {
            BACKDROP_CHANGED.set(true);
            LRESULT(0)
        }
        WM_SYSCOLORCHANGE => {
            BACKDROP_CHANGED.set(true);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}
