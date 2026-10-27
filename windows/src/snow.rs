use std::sync::mpsc::Receiver;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::Threading::{
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, CreateWaitableTimerW, INFINITE, SetWaitableTimer, TIMER_ALL_ACCESS,
    WaitForMultipleObjects,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MB_ICONERROR, MB_OK, MSG, MWMO_INPUTAVAILABLE, MessageBoxW, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW,
    PostMessageW, QS_ALLINPUT, TranslateMessage, WM_CLOSE,
};
use windows::core::{HSTRING, PCWSTR, Result, w};

use snowhush::simulation::{SnowSimulation, Snowfall};

use crate::light::daylight;
use crate::log;
use crate::prefs::Settings;
use crate::render::{Ambient, Gpu};
use crate::screen::{Clock, DISPLAYS_CHANGED, Monitor, PowerBudget, Screen, dip_size, monitors};
use crate::tracker::Tracker;

pub enum Command {
    Snowfall(Snowfall),
    Accumulate(bool),
    Melt(bool),
    SnowOnWindows(bool),
    ShowInScreenshots(bool),
    /// Off while the screen is locked, asleep or the session is away.
    Active(bool),
    Quit,
}

/// Raw handles cross to the snow thread as numbers: the thread only waits on the event and posts to the window.
pub struct Links {
    pub wake: isize,
    pub controller: isize,
}

pub fn run(commands: Receiver<Command>, links: Links, settings: Settings) {
    if let Err(error) = run_loop(&commands, &links, settings) {
        log!("stopped: {error}");
        let text = HSTRING::from(format!("Snowhush couldn't start drawing snow.\n\n{error}"));
        unsafe {
            MessageBoxW(None, &text, w!("Snowhush"), MB_OK | MB_ICONERROR);
            let _ = PostMessageW(Some(HWND(links.controller as _)), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn run_loop(commands: &Receiver<Command>, links: &Links, mut settings: Settings) -> Result<()> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let mut gpu = Gpu::new()?;
    log!("adapter: {}", gpu.adapter);
    let mut screens = sync_screens(&gpu, &settings, Vec::new());
    let timer = unsafe {
        CreateWaitableTimerExW(None, PCWSTR::null(), CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0)
            .or_else(|_| CreateWaitableTimerW(None, false, PCWSTR::null()))?
    };
    let wake = HANDLE(links.wake as _);
    let mut clock = Clock::new();
    let started = Clock::now();
    let mut active = true;
    let mut budget = PowerBudget::Full;
    let mut ambient = Ambient { light: daylight(), shadow_tint: [0.0; 4] };
    let mut chores_at = 0;
    let mut light_at = 0;
    let mut displays_changed_at = None;
    let mut stats = Stats::new(started);
    let mut tracker = Tracker::new();

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Quit => return Ok(()),
                Command::Snowfall(snowfall) => {
                    settings.snowfall = snowfall;
                    screens.iter_mut().for_each(|screen| screen.simulation.set_snowfall(snowfall));
                }
                Command::Accumulate(on) => {
                    settings.accumulate = on;
                    screens.iter_mut().for_each(|screen| screen.simulation.accumulates = on);
                }
                Command::Melt(on) => {
                    settings.melt = on;
                    screens.iter_mut().for_each(|screen| screen.simulation.thaws = on);
                }
                Command::SnowOnWindows(on) => {
                    settings.snow_on_windows = on;
                    if !on {
                        screens.iter_mut().for_each(Screen::drop_window_snow);
                    }
                }
                Command::ShowInScreenshots(on) => {
                    settings.show_in_screenshots = on;
                    screens.iter().for_each(|screen| screen.show_in_screenshots(on));
                }
                Command::Active(on) => {
                    if on && !active {
                        screens.iter_mut().for_each(Screen::resume);
                    }
                    active = on;
                    log!("active: {on}");
                }
            }
        }

        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        let mut now = Clock::now();
        if DISPLAYS_CHANGED.take() {
            displays_changed_at = Some(now);
        }
        // Display changes come in bursts; the layout is read once they settle.
        if let Some(changed_at) = displays_changed_at
            && clock.seconds(now - changed_at) > 0.4
        {
            displays_changed_at = None;
            screens = sync_screens(&gpu, &settings, screens);
            clock.refresh();
        }

        if now >= chores_at {
            chores_at = now + clock.frequency;
            clock.refresh();
            let next = power_budget();
            if next != budget {
                budget = next;
                log!("power: {budget:?}");
                let density = if budget == PowerBudget::Saver { 0.75 } else { 1.0 };
                screens.iter_mut().for_each(|screen| screen.simulation.set_density(density));
            }
            screens.iter().for_each(Screen::raise);
            if now >= light_at {
                light_at = now + clock.frequency * 60;
                ambient.light = daylight();
            }
        }

        if active {
            if settings.snow_on_windows {
                // A drag starting is drawn right away instead of at the next calm frame.
                if tracker.take_started_moving() {
                    screens.iter_mut().for_each(|screen| screen.wake(now));
                }
                if screens.iter().any(|screen| screen.next_due <= now + clock.tolerance()) {
                    tracker.refresh(&clock, now);
                }
            }
            let windows = settings.snow_on_windows.then_some(&tracker);
            let time = (clock.seconds(now - started) % 1000.0) as f32;
            let mut lost = false;
            for screen in screens.iter_mut().filter(|screen| screen.next_due <= now + clock.tolerance()) {
                let began = Clock::now();
                match screen.frame(&gpu, &clock, now, budget, ambient, time, windows) {
                    Ok(drew) => stats.frame(drew, Clock::now() - began, screen.simulation.sprite_count()),
                    Err(error) => {
                        log!("frame failed: {error}");
                        lost = true;
                    }
                }
            }
            if lost {
                screens.clear();
                gpu = Gpu::new()?;
                screens = sync_screens(&gpu, &settings, Vec::new());
            }
            now = Clock::now();
            stats.report(&clock, now, &mut tracker);
        }

        let due = screens.iter().map(|screen| screen.next_due).min().filter(|_| active);
        unsafe {
            if let Some(due) = due {
                let wait = -(((due - now).max(0) as i128 * 10_000_000) / clock.frequency as i128) as i64;
                let _ = SetWaitableTimer(timer, &wait, 0, None, None, false);
                // Messages wait for the next frame: window reports arrive with every mouse move,
                // and waking for each would cost more than the snow.
                WaitForMultipleObjects(&[wake, timer], false, INFINITE);
            } else {
                MsgWaitForMultipleObjectsEx(Some(&[wake]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
            }
        }
    }
}

/// One overlay per display: new displays get snow, removed ones lose theirs, and a display that changed
/// size keeps its snow in a fresh overlay.
fn sync_screens(gpu: &Gpu, settings: &Settings, mut current: Vec<Screen>) -> Vec<Screen> {
    let mut result = Vec::new();
    for monitor in monitors() {
        let kept = current.iter().position(|screen| screen.monitor.name == monitor.name);
        let simulation = match kept.map(|i| current.swap_remove(i)) {
            Some(screen) if screen.monitor == monitor => {
                result.push(screen);
                continue;
            }
            Some(screen) => {
                let mut simulation = screen.simulation;
                let (width, height) = dip_size(&monitor);
                simulation.resize(width as f32, height as f32);
                simulation
            }
            None => new_simulation(&monitor, settings),
        };
        log!("display {}: {:?}, {} dpi, {} Hz", monitor.name, monitor.rect, monitor.dpi, monitor.refresh);
        match Screen::new(gpu, monitor, simulation, settings.show_in_screenshots) {
            Ok(screen) => result.push(screen),
            Err(error) => log!("overlay failed: {error}"),
        }
    }
    result
}

fn new_simulation(monitor: &Monitor, settings: &Settings) -> SnowSimulation {
    let (width, height) = dip_size(monitor);
    let mut simulation = SnowSimulation::new(width as f32, height as f32, settings.snowfall);
    simulation.thaws = settings.melt;
    simulation.accumulates = settings.accumulate;
    simulation
}

fn power_budget() -> PowerBudget {
    let mut status = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut status) }.is_err() {
        return PowerBudget::Full;
    }
    if status.SystemStatusFlag == 1 {
        PowerBudget::Saver
    } else if status.ACLineStatus == 0 {
        PowerBudget::Battery
    } else {
        PowerBudget::Full
    }
}

/// What a frame costs on this thread, written to the log twice a minute.
struct Stats {
    since: i64,
    steps: u32,
    draws: u32,
    busy: i64,
    sprites: usize,
}

impl Stats {
    fn new(now: i64) -> Stats {
        Stats { since: now, steps: 0, draws: 0, busy: 0, sprites: 0 }
    }

    fn frame(&mut self, drew: bool, busy: i64, sprites: usize) {
        self.steps += 1;
        self.draws += drew as u32;
        self.busy += busy;
        self.sprites = self.sprites.max(sprites);
    }

    fn report(&mut self, clock: &Clock, now: i64, tracker: &mut Tracker) {
        let elapsed = clock.seconds(now - self.since);
        if elapsed < 30.0 {
            return;
        }
        let busy = clock.seconds(self.busy);
        let listing = clock.seconds(tracker.cost);
        log!(
            "frames: {:.1} steps/s, {:.1} draws/s, {:.0} µs per step, {:.2}% of a core, up to {} sprites; windows: {:.1} reads/s, {:.0} µs per read",
            self.steps as f64 / elapsed,
            self.draws as f64 / elapsed,
            busy * 1e6 / self.steps.max(1) as f64,
            (busy + listing) / elapsed * 100.0,
            self.sprites,
            tracker.listings as f64 / elapsed,
            listing * 1e6 / tracker.listings.max(1) as f64
        );
        *self = Stats::new(now);
        tracker.listings = 0;
        tracker.cost = 0;
    }
}
