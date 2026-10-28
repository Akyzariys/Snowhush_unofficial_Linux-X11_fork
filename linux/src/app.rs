use std::time::{Duration, Instant};

use winit::event::{Event, StartCause, WindowEvent};
use winit::event_loop::ControlFlow;

use snowhush::simulation::Snowfall;

use crate::light::daylight;
use crate::log;
use crate::prefs::Settings;
use crate::render::Gpu;
use crate::screen::{Clock, PowerBudget, Screen, dip_size, monitors};
use crate::tracker::Tracker;
use crate::{session, tray};

#[derive(Clone, Copy, Debug)]
pub enum Command {
    Snowfall(Snowfall),
    Accumulate(bool),
    Melt(bool),
    SnowOnWindows(bool),
    /// Blows all the snow away; when calm, for less motion, melts it where it lies instead.
    ClearSnow(bool),
    /// Off while the screen is locked or asleep.
    Active(bool),
    Quit,
}

pub fn run() -> Result<(), String> {
    let event_loop = winit::event_loop::EventLoopBuilder::<Command>::with_user_event()
        .build()
        .map_err(|error| format!("opening the display: {error}"))?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let settings = Settings::load();
    let proxy = event_loop.create_proxy();
    let locked = session::locked();
    let mut app = App::new(&event_loop, settings, !locked)?;
    let _tray = tray::start(settings, proxy.clone());
    session::watch(proxy, locked);

    event_loop
        .run(move |event, target| match event {
            Event::NewEvents(StartCause::Init) => {
                log!("display ready");
            }
            Event::AboutToWait => app.tick(target),
            Event::UserEvent(command) => app.command(command, target),
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => target.exit(),
            Event::LoopExiting => app.settings.save(),
            _ => {}
        })
        .map_err(|error| error.to_string())
}

struct App {
    settings: Settings,
    gpu: Gpu,
    tracker: Tracker,
    screens: Vec<Screen>,
    clock: Clock,
    active: bool,
    budget: PowerBudget,
    light: [f32; 3],
    checked_at: f64,
    lit_at: f64,
    laid_out_at: f64,
    stats_at: f64,
    steps: u32,
    draws: u32,
    busy: f64,
}

impl App {
    fn new<T: 'static>(
        target: &winit::event_loop::EventLoopWindowTarget<T>,
        settings: Settings,
        active: bool,
    ) -> Result<App, String> {
        let clock = Clock::new();
        let mut tracker = Tracker::new()?;
        tracker.refresh(0.0);
        let gpu = Gpu::new()?;
        log!("adapter: {}", gpu.adapter_name);
        let mut app = App {
            settings,
            gpu,
            tracker,
            screens: Vec::new(),
            clock,
            active,
            budget: PowerBudget::Full,
            light: daylight(),
            checked_at: 0.0,
            lit_at: 0.0,
            laid_out_at: 0.0,
            stats_at: 0.0,
            steps: 0,
            draws: 0,
            busy: 0.0,
        };
        app.sync(target, true);
        Ok(app)
    }

    fn tick<T: 'static>(&mut self, target: &winit::event_loop::EventLoopWindowTarget<T>) {
        let now = self.clock.now();
        let listed = self.tracker.refresh(now);
        if !self.active {
            // A display hot-plugged or rebuilt while locked must start hidden too.
            for screen in &mut self.screens {
                screen.set_paused(true);
            }
        }

        if now - self.laid_out_at >= 1.0 {
            self.laid_out_at = now;
            self.sync(target, false);
        }
        if now - self.checked_at >= 10.0 {
            self.checked_at = now;
            let budget = session::power_budget();
            if budget != self.budget {
                self.budget = budget;
                log!("power: {budget:?}");
            }
        }
        if now - self.lit_at >= 60.0 {
            self.lit_at = now;
            self.light = daylight();
        }

        if self.active {
            if self.tracker.take_started_moving() && self.settings.snow_on_windows {
                for screen in &mut self.screens {
                    screen.wake(now);
                }
            }
            let cursor = self.tracker.cursor().unwrap_or((-10_000, -10_000));
            let time = (now % 1000.0) as f32;
            for index in 0..self.screens.len() {
                let covering = self
                    .tracker
                    .covering(&self.screens[index].monitor.bounds)
                    .map(str::to_string);
                if covering.is_some() != self.screens[index].covered() {
                    log!(
                        "{} {}",
                        self.screens[index].monitor.name,
                        covering
                            .as_deref()
                            .map_or("uncovered".to_string(), |class| format!(
                                "covered by {class}"
                            ))
                    );
                    self.screens[index].set_covered(covering.is_some());
                }
                if self.screens[index].covered() {
                    self.screens[index].next_due = now + 0.25;
                    continue;
                }
                if listed {
                    self.screens[index].raise(&self.tracker);
                }
                if self.screens[index].next_due > now + 0.004 {
                    continue;
                }
                let windows = (self.settings.snow_on_windows).then_some(&self.tracker);
                let began = Instant::now();
                match self.screens[index].frame(
                    &self.gpu,
                    now,
                    self.budget,
                    self.light,
                    time,
                    windows,
                    cursor,
                ) {
                    Ok(drew) => {
                        self.steps += 1;
                        self.draws += drew as u32;
                        self.busy += began.elapsed().as_secs_f64();
                    }
                    Err(error) => {
                        log!("frame failed: {error}");
                        match error {
                            wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated => {
                                self.screens[index].reconfigure(&self.gpu)
                            }
                            wgpu::SurfaceError::OutOfMemory => {
                                log!("out of graphics memory");
                                target.exit();
                                return;
                            }
                            _ => {}
                        }
                    }
                }
            }
            if now - self.stats_at >= 30.0 {
                let elapsed = now - self.stats_at;
                log!(
                    "frames: {:.1} steps/s, {:.1} draws/s, {:.2}% of a core",
                    self.steps as f64 / elapsed,
                    self.draws as f64 / elapsed,
                    self.busy / elapsed * 100.0
                );
                self.stats_at = now;
                self.steps = 0;
                self.draws = 0;
                self.busy = 0.0;
            }
        }

        let due = self
            .screens
            .iter()
            .map(|screen| screen.next_due)
            .fold(f64::INFINITY, f64::min);
        let due = (due.is_finite() && self.active).then_some(due);
        match due {
            Some(due) => {
                let wait = (due - self.clock.now()).max(0.0005);
                target.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_secs_f64(wait),
                ));
            }
            None if self.active => {
                // Nothing to draw: look again in a second, in case a display appears or the lock lifts.
                target.set_control_flow(ControlFlow::WaitUntil(
                    Instant::now() + Duration::from_secs(1),
                ));
            }
            None => target.set_control_flow(ControlFlow::Wait),
        }
    }

    fn command<T: 'static>(
        &mut self,
        command: Command,
        target: &winit::event_loop::EventLoopWindowTarget<T>,
    ) {
        match command {
            Command::Quit => {
                self.settings.save();
                log!("quit");
                target.exit();
            }
            Command::Snowfall(snowfall) => {
                self.settings.snowfall = snowfall;
                self.screens
                    .iter_mut()
                    .for_each(|screen| screen.simulation.set_snowfall(snowfall));
            }
            Command::Accumulate(on) => {
                self.settings.accumulate = on;
                self.screens
                    .iter_mut()
                    .for_each(|screen| screen.simulation.accumulates = on);
            }
            Command::Melt(on) => {
                self.settings.melt = on;
                self.screens
                    .iter_mut()
                    .for_each(|screen| screen.simulation.thaws = on);
            }
            Command::SnowOnWindows(on) => {
                self.settings.snow_on_windows = on;
                if !on {
                    self.screens.iter_mut().for_each(Screen::drop_window_snow);
                }
            }
            Command::ClearSnow(calm) => {
                let direction = self
                    .screens
                    .first()
                    .map_or(-1.0, |screen| screen.simulation.breeze_direction());
                self.screens
                    .iter_mut()
                    .for_each(|screen| screen.simulation.blow_away(direction, calm));
            }
            Command::Active(on) => {
                let was_active = self.active;
                self.active = on;
                for screen in &mut self.screens {
                    screen.set_paused(!on);
                }
                if on && !was_active {
                    self.tracker.refresh(self.clock.now());
                    for screen in &self.screens {
                        if !screen.covered() {
                            screen.raise(&self.tracker);
                        }
                    }
                }
                log!("active: {on}");
            }
        }
        // Commands arrive between frames; draw the effect now instead of at the next quiet tick.
        if self.active {
            for screen in &mut self.screens {
                screen.wake(self.clock.now());
            }
        }
    }

    /// One overlay per display: new displays get snow, removed ones lose theirs, and a display that changed
    /// size keeps its snow in a fresh overlay.
    fn sync<T: 'static>(
        &mut self,
        target: &winit::event_loop::EventLoopWindowTarget<T>,
        rebuild: bool,
    ) {
        let handles: Vec<_> = target.available_monitors().collect();
        let desired = monitors(&handles, &self.tracker);
        if desired.is_empty() {
            if !self.screens.is_empty() {
                log!("no displays");
                self.screens.clear();
            }
            return;
        }
        if !rebuild
            && desired.len() == self.screens.len()
            && desired
                .iter()
                .zip(&self.screens)
                .all(|(monitor, screen)| monitor == &screen.monitor)
        {
            return;
        }
        let mut current = std::mem::take(&mut self.screens);
        let mut result = Vec::new();
        for monitor in desired {
            let kept = current
                .iter()
                .position(|screen| screen.monitor.name == monitor.name);
            let format_hint = self.gpu.format;
            let simulation = match kept.map(|index| current.swap_remove(index)) {
                Some(screen) if screen.monitor == monitor && !rebuild => {
                    result.push(screen);
                    continue;
                }
                Some(screen) => {
                    let mut simulation = screen.simulation;
                    let (width, height) = dip_size(&monitor);
                    simulation.resize(width as f32, height as f32);
                    simulation
                }
                None => new_simulation(&monitor, &self.settings),
            };
            match Screen::new(
                &mut self.gpu,
                target,
                &self.tracker,
                monitor.clone(),
                simulation,
                format_hint,
                !self.active,
            ) {
                Ok(screen) => result.push(screen),
                Err(error) => log!("overlay failed on {}: {error}", monitor.name),
            }
        }
        self.screens = result;
    }
}

fn new_simulation(
    monitor: &crate::screen::Monitor,
    settings: &Settings,
) -> snowhush::simulation::SnowSimulation {
    let (width, height) = dip_size(monitor);
    let mut simulation =
        snowhush::simulation::SnowSimulation::new(width as f32, height as f32, settings.snowfall);
    simulation.thaws = settings.melt;
    simulation.accumulates = settings.accumulate;
    simulation
}
